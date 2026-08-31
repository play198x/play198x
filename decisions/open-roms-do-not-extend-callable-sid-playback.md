# Decision: Open replacement ROMs do not extend callable SID playback

**Status:** Active  
**Date:** 2026-08-31  
**Issue:** [play198x/play198x#37](https://github.com/play198x/play198x/issues/37)

## Decision

Play198x does not commit, build, fetch, or ask a listener to load replacement
C64 ROMs to recover callable PSID tunes. A tune whose reads resolve to BASIC,
KERNAL, or CHARGEN remains a named `NeedsRom` failure.

This is a playback-policy decision, not a claim that clean-room ROMs are
illegitimate or unusable. It also does not decide the separate RSID and
self-driven PSID work in
[#39](https://github.com/play198x/play198x/issues/39). If that work needs a
power-on machine, its boundary is judged there rather than smuggled into the
small callable-tune host.

## Why

### The licence permits distribution, with obligations

The MEGA65 Open ROMs repository was inspected at commit
[`ad178dbe`](https://github.com/MEGA65/open-roms/commit/ad178dbe4d48cd6a317737a8e0e7e662f7e33d32)
(2026-05-16), rather than relying on GitHub's `NOASSERTION` classification.
Its [`LICENSE`](https://github.com/MEGA65/open-roms/blob/ad178dbe4d48cd6a317737a8e0e7e662f7e33d32/LICENSE)
applies LGPL-3.0-or-later to the package, except for specifically marked BASIC
sources under MIT. The prebuilt ROM directory says the PXL font was separately
permitted for inclusion under LGPL-3.0; the generic set measured here instead
used `chargen_openroms.rom`.

Those terms do not prohibit Play198x from distributing binaries. They would,
however, make Play198x a distributor of separately licensed firmware, with
notice, licence, corresponding-source, provenance, and update obligations.
Fetching at runtime moves the transfer but does not remove the product and
compatibility decision. Visitor-supplied ROMs avoid distribution, but add a
setup ceremony for a very small tail and contradict the player's zero-setup
browser surface.

### The measured gain is small and not a fidelity result

The deterministic first 5,000 paths in the local HVSC #85 corpus were run with
the implemented address decoder at 8 kHz for init plus ten frames. The current
player classified 4,161 files as callable PSID: 3,933 completed ROM-free, 33
reported `NeedsRom`, and 195 exhausted an init or play budget. The remaining
839 were outside the callable policy or failed parsing.

The detected cohort was rerun with MEGA65's pinned generic BASIC and KERNAL
images plus `chargen_openroms.rom` mapped into the same host. Of 35 tunes that
touched ROM across the two passes, 32 completed ten frames and three did not:
a **91.4% execution-candidate rate**, but only **0.64% of the 5,000-file
sample**.

Completion means only that the CPU returned inside its budgets. It does not
show that the audio matches a C64. The callable host intentionally lacks much
of the hardware a KERNAL routine can observe, and replacement ROM bytes differ
from Commodore's. A tune may use an undocumented entry point or ROM bytes as
data and still produce plausible, wrong output. Proving those 32 tunes would
require differential audio against the original ROMs, after first expanding
the host toward a machine. That is disproportionate work for less than one
percentage point of this sample.

### The existing failure is the honest result

The decoder distinguishes a real mapped-ROM read from RAM banked underneath
the same address. Refusing only after that observed read preserves the broad
ROM-free slice without guessing from address ranges or headers. `NeedsRom`
names the missing ROM instead of rendering silence or replacement-ROM output
that Play198x cannot claim is byte-compatible.

## Consequences

- `play198x-core` remains ROM-free and its `sid` feature acquires no firmware.
- The web player neither downloads a ROM nor asks a visitor to supply one.
- ROM-dependent callable PSIDs remain identified and explicitly refused.
- A future proposal may revisit this only with materially better coverage or a
  fidelity method, and must treat the firmware licences and disclosure as part
  of the feature rather than packaging detail.

