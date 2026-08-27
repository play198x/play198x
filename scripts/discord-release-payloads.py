#!/usr/bin/env python3
"""Turn release-plz's `releases` output into Discord webhook payloads.

release-plz reports only package_name, version and tag — no notes. The notes it
wrote are in each crate's CHANGELOG.md, committed by the release PR, so main
already holds them by the time the release job runs. Reading them there gives
every crate its own changelog in the announcement, including crates that never
get a GitHub Release (a library with git_release_enable = false still has a
changelog, and that is the only place its notes exist).

One embed per crate rather than one embed listing crates: a multi-crate cycle
releases several unrelated changelogs, and a list of links throws all of them
away.

Writes payload-N.json files, at most ten embeds each, which is Discord's limit.
"""

import argparse
import json
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

# Discord's embed description limit is 4096; leave room for the links line.
NOTES_LIMIT = 3600
EMBEDS_PER_MESSAGE = 10
COLOUR = 3066993


def changelog_section(path: Path, version: str) -> str:
    if not path.exists():
        return ""
    text = path.read_text()
    # Matches "## [0.1.3](compare-url) - 2026-08-27", "## [0.1.3] - date" and a
    # bare "## 0.1.3".
    #
    # The lookahead is load-bearing: without it, asking for 0.1.1 matches the
    # heading for 0.1.10 and announces the wrong release's notes. Every version
    # that is a prefix of a later one hits this, which is every x.y.9 -> x.y.10.
    heading = re.compile(
        rf"^##\s+\[?{re.escape(version)}\]?(?![0-9.])[^\n]*$", re.MULTILINE
    )
    match = heading.search(text)
    if not match:
        return ""
    rest = text[match.end():]
    following = re.search(r"^##\s+", rest, re.MULTILINE)
    return (rest[: following.start()] if following else rest).strip()


def on_crates_io(package: str, version: str) -> bool:
    request = urllib.request.Request(
        f"https://crates.io/api/v1/crates/{package}/{version}",
        headers={"User-Agent": "198x-ci"},
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            return response.status == 200
    except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError):
        return False


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--releases", required=True, help="release-plz `releases` JSON")
    parser.add_argument("--repo", required=True, help="owner/name")
    parser.add_argument("--server-url", default="https://github.com")
    parser.add_argument("--crate-dir", default="crates", help="where crates live")
    parser.add_argument(
        "--crates-io-only",
        action="store_true",
        help="announce only crates that reached the registry (for a publish step)",
    )
    parser.add_argument("--out-dir", default=".")
    args = parser.parse_args()

    releases = json.loads(args.releases)
    out_dir = Path(args.out_dir)

    embeds = []
    for release in releases:
        package = release.get("package_name")
        version = release.get("version")
        if not package or not version:
            continue

        published = on_crates_io(package, version)
        if args.crates_io_only and not published:
            print(f"{package} {version} is not on crates.io — skipping.")
            continue

        tag = release.get("tag") or ""
        if tag:
            url = f"{args.server_url}/{args.repo}/releases/tag/{tag}"
        elif published:
            url = f"https://crates.io/crates/{package}/{version}"
        else:
            url = f"{args.server_url}/{args.repo}"

        notes = changelog_section(
            Path(args.crate_dir) / package / "CHANGELOG.md", version
        )
        if not notes:
            print(f"::notice::no changelog section for {package} {version}")
            notes = "_No changelog entry recorded for this version._"
        elif len(notes) > NOTES_LIMIT:
            notes = notes[:NOTES_LIMIT].rstrip() + "\n\n…truncated."

        links = []
        if tag:
            links.append(f"[release notes]({url})")
        if published:
            links.append(f"[crates.io](https://crates.io/crates/{package}/{version})")
            links.append(f"[docs.rs](https://docs.rs/{package}/{version})")
        if links:
            notes = f"{notes}\n\n{' · '.join(links)}"

        embeds.append(
            {
                "title": f"{package} v{version}",
                "url": url,
                "description": notes,
                "color": COLOUR,
                "footer": {"text": args.repo},
            }
        )

    if not embeds:
        print("Nothing to announce.")
        return 0

    written = 0
    for index in range(0, len(embeds), EMBEDS_PER_MESSAGE):
        written += 1
        payload = {
            "username": "198x releases",
            "embeds": embeds[index: index + EMBEDS_PER_MESSAGE],
        }
        (out_dir / f"payload-{written}.json").write_text(json.dumps(payload))

    print(f"{len(embeds)} crate(s) in {written} message(s).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
