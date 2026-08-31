#!/usr/bin/env python3
"""Require shipping web-shell changes to carry a reviewed release version."""

from __future__ import annotations

import argparse
import subprocess
import tomllib
from pathlib import Path


MANIFEST = "crates/play198x-web/Cargo.toml"
CHANGELOG = "crates/play198x-web/CHANGELOG.md"


def version(text: str) -> str:
    return str(tomllib.loads(text)["package"]["version"])


def release_errors(old_manifest: str, new_manifest: str, changelog_diff: str) -> list[str]:
    old = version(old_manifest)
    new = version(new_manifest)
    errors = []
    if old == new:
        errors.append(f"web shipping changes keep version {new}; bump {MANIFEST}")
    if f"+## [{new}]" not in changelog_diff:
        errors.append(f"web {new} has no added '## [{new}]' heading in {CHANGELOG}")
    return errors


def self_test() -> None:
    manifest = lambda value: f'[package]\nname = "play198x-web"\nversion = "{value}"\n'
    assert len(release_errors(manifest("1.0.0"), manifest("1.0.0"), "")) == 2
    assert release_errors(manifest("1.0.0"), manifest("1.1.0"), "+## [1.1.0]") == []
    assert release_errors(manifest("1.0.0"), manifest("1.1.0"), "+## [1.0.0]")


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, text=True, capture_output=True).stdout


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
    if not args.base:
        return 0

    changed = git("diff", "--name-only", args.base, "HEAD").splitlines()
    ships = any(path == MANIFEST or path.startswith("crates/play198x-web/src/") for path in changed)
    if not ships:
        return 0

    old_manifest = git("show", f"{args.base}:{MANIFEST}")
    new_manifest = Path(MANIFEST).read_text()
    changelog_diff = git("diff", "--unified=0", args.base, "HEAD", "--", CHANGELOG)
    errors = release_errors(old_manifest, new_manifest, changelog_diff)
    for error in errors:
        print(f"error: {error}")
    return int(bool(errors))


if __name__ == "__main__":
    raise SystemExit(main())
