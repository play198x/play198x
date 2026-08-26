# The differential harness

`differential.rs` measures this engine against **libxmp**, an independent
ProTracker implementation nobody here wrote.

Every other test in this crate checks the engine against itself: arithmetic it
believes, verified by tests its author wrote. That catches mistakes, but it
cannot catch a *shared* misreading of the replayer. libxmp read the same
assembly independently, so where it agrees the reading is probably right, and
where it disagrees one of the two has something to learn.

## Running it

The harness is `#[ignore]`d, so it stays off the default run:

```sh
cargo test -p play198x-core --test differential -- --ignored --nocapture
```

It shells out to the `xmp` CLI (`brew install xmp`); set `XMP` to point at a
different binary. **When the tool is missing the harness panics rather than
returning.** A test that quietly returns looks identical to one that ran and
passed, and it is `#[ignore]`d precisely so that asking for it produces an
answer. There is no libxmp FFI dependency and no new crate — the harness runs
the binary and reads the WAV it writes.

`--nocapture` is worth passing: every measure prints with both players' readings
and the gap between them, which carries more than the assertion does.

## What it compares

Sample-exact comparison is **not achievable**, and pretending otherwise would
waste the harness. Different interpolation and different mixing make the
waveforms legitimately differ. So each comparison is of a derived measure:

| Measure | How | Fixture |
|---|---|---|
| Carrier pitch | per waveform cycle, from positive-going zero crossings | held C-2, no effect |
| Note onset | first frame past a tenth of the render's own peak | held C-2; `900`/`904` |
| Vibrato rate | Schmitt trigger over the pitch track | `4AF`, `45F` |
| Vibrato depth | period spread across the pitch track | `4AF`, `45F` |
| Portamento rate | median carrier per row span, back-converted to periods | `204` |
| Volume-slide rate | peak envelope per row, as a fraction of full scale | `A01` |
| Tremolo rate | Schmitt trigger over the envelope | `7A4` from volume 32 |
| Tremolo depth | envelope spread | `7A4` from volume 32 |
| Envelope shape | Pearson correlation over the 10 ms peak envelope | `A01`, `7A4` |

Fixtures are the synthetic single-effect modules the effect tests already use —
one held note, one effect, generated in code. **No media file is committed.**
Real music confounds these measurements: an attempt on 2026-08-25 measured a
real module's vibrato and got the *row* rate instead, because at speed 5 the
0.100 s row period dominated the envelope.

The measurement helpers are the ones the engine tests built (`tests/common`):
the pitch tracker that refuses a window under 100 ms, and the Schmitt trigger
that replaced a mean-crossing count which read 9.28 Hz for a 6.51 Hz vibrato.
They encode measurement mistakes this project actually made; a second set would
repeat them.

## What it deliberately does not compare

- **Sample values.** See above. Nothing here asserts on a waveform.
- **Absolute levels.** Mixer gain is an arbitrary constant on both sides. Every
  amplitude measure is a ratio, a fraction of the player's own full scale, or a
  correlation.
- **Stereo placement.** xmp is asked for mono and the result is duplicated
  across both channels, so its default pan separation never enters a number.
  Our hard panning is pinned by `engine.rs` instead.
- **Interpolation quality.** xmp is put in `nearest` mode to match this engine.
  Against its default spline the harness would be measuring the interpolator.
- **Anything past the first pattern.** Every fixture is one pattern of one
  effect. Sequencing is `engine.rs`'s and `duration.rs`'s business.

## Thresholds

Each tolerance was set **after** running the harness, from the agreement
actually observed, with the observed figure in a comment beside it. A threshold
chosen before measuring either passes everything or fails everything, and in
both cases says nothing.

Measured 2026-08-26 against xmp 4.3.1 linking libxmp 4.7.2, at 44.1 kHz:

| Measure | ours | libxmp | delta |
|---|---|---|---|
| Carrier pitch | 259.4118 Hz | 259.4118 Hz | +0.00% |
| Note onset, `904` | 0.0000 ms | 0.0000 ms | 0.0000 ms |
| Note onset, `900` | 123.5828 ms | 123.5147 ms | −0.068 ms |
| Envelope ripple, held note | 0.0000% | 0.0000% | +0.00% |
| Vibrato rate, `4AF` | 6.5065 Hz | 6.4874 Hz | −0.29% |
| Vibrato rate, `45F` | 3.2571 Hz | 3.2461 Hz | −0.34% |
| Vibrato depth, `4AF` | 28.904 periods | 27.647 periods | −4.35% |
| Portamento slide, `204` | 20.1071 per/row | 20.1071 per/row | +0.00% |
| Volume slide, `A01` | 0.0781 of full/row | 0.0781 of full/row | +0.00% |
| Volume-slide envelope | — | — | r = 1.0000 |
| Tremolo rate, `7A4` | 6.4865 Hz | 6.4854 Hz | −0.02% |
| Tremolo depth, `7A4` | 0.4688 of level | 0.4688 of level | +0.00% |
| Tremolo envelope | — | — | r = 0.9133 |

The carrier reads 259.4118 Hz in both against a replayer-derived 258.9730,
because a cycle of 170.29 frames at 44.1 kHz can only be counted as 170. Both
players quantise it identically, which is the point: the 0.17% is the frame
grid, not a disagreement.

## The one disagreement this harness found, and how it closed

### Tremolo depth — libxmp was right, and ProTracker's own source says why

**Closed 2026-08-26.** This section used to record an open difference. It is
settled, against this engine, and the engine has been changed.

**What the harness measured.** With `7A4` from a stored volume of 32, this
engine swung the volume by ±7 (0.2188 of the level) where libxmp swung it by
±15 (0.4688) — a ratio of 2.14. libopenmpt 0.8.9 agreed with libxmp, at 0.4351
on the same fixture. Rate (−0.02%), waveform and phase all matched; only the
scale differed. Two independent mature players agreeing against us was the
signal to go and find a better source, not to widen a tolerance.

**Why we had it wrong.** The engine was built from
`protracker-23b-playroutine.asm`, Frank Wille's Protracker V2.3B Playroutine
v6.3, which computes the tremolo table offset at line 2202 as

```
	; calculate tremolo table offset: 64 * amplitude + (pos & 63)
.4:	lsl.w	#6,d2
	moveq	#63,d0
	and.b	n_tremolopos(a2),d0
	add.w	d0,d2
```

and reads it out of `mt_VibratoSineTable` at line 2226 — the *same* table, at
the *same* offset formula, that `mt_vibrato` uses at lines 2127 and 2151. There
is only one such table in the file, so in that replayer tremolo and vibrato have
identical depth. The engine faithfully implemented what the file said.

**What settled it.** ProTracker's own source was acquired:
`protracker-23f-replay-cia.s`, Olav Sørensen's disassembly and re-source of
ProTracker 2.3D (BSD-3-Clause), now under `reference/by-topic/music-formats/`.
It runs the identical multiply in both effects and shifts the product by one bit
less for tremolo:

| | |
|---|---|
| `mt_vib_set` | `MULU.W D0,D2` then `LSR.W #7,D2` |
| `mt_tre_set` | `MULU.W D0,D2` then `LSR.W #6,D2` |

One bit is exactly the factor of two. Wille's table-driven rewrite folds both
effects onto the one precomputed table and normalises the asymmetry away, which
makes it the better reading of *the algorithm* and the wrong authority for
*ProTracker's behaviour*. The family reference records this as *"Tremolo is
twice as deep as vibrato, and only ProTracker's own source says so"* in
`protracker-playback-reference.md`.

**What changed.** `effects.rs` divides the tremolo product by 64 and the vibrato
product by 128, with the citation at the tremolo site so the next reader does not
"fix" it back to match the rest of the engine's source. The harness now asserts
*agreement* on depth within 2% — measured at +0.00%, both players reading 0.4688
of the level — instead of asserting the 2.14x gap. The unit tests moved with it:
`tremolo_swings_the_volume_around_the_stored_one` uses amplitude 7 so the swing
keeps headroom and measures depth alone, and `tremolo_clamps_the_volume_at_both_ends`
is new, because doubling the swing put `mt_Tremolo3`'s `0..=64` clamp within
reach of a fixture where it never was before.

**The lesson, since the method generalises.** Two independent implementations
agreeing against a single source is a reason to go and find a better source.
The harness recorded the gap as an assertion rather than tolerating it, which is
what kept it findable until the source turned up.

### Not a difference: the vibrato rate

The reference document's *"⚠ The community spec is wrong about the vibrato
rate"* section records a measurement table in which libxmp 4.7.2 plays `4AF` at
6.20 Hz and libopenmpt 0.8.9 at 8.85 Hz against a replayer-derived 6.51 Hz, and
concludes that "two mature players disagree here".

**That table does not reproduce.** On the same fixture on 2026-08-26:

| player | `4AF` | `45F` |
|---|---|---|
| replayer-derived formula | 6.5104 Hz | 3.2552 Hz |
| this engine | 6.5065 Hz | 3.2571 Hz |
| libxmp 4.7.2 (via xmp 4.3.1) | 6.4874 Hz | 3.2461 Hz |
| libopenmpt 0.8.9 (via openmpt123) | 6.5069 Hz | 3.2571 Hz |

All four agree within 0.4%, and none is anywhere near the community
specification's 7.8125 Hz. libxmp gives the same figure in every player mode
(`auto`, `mod`, `protracker`, `noisetracker`), so the mode is not the
explanation either.

The reference's conclusion — that the replayer is the authority and the
community formula is 20% wrong — still holds and this harness confirms it. Only
the two measured columns are wrong, and both look like artefacts of the
measurement bugs this project has since fixed in its own helpers: an unbiased
span (libxmp's 6.20 ≈ counting over the whole buffer rather than between
triggers) and a Schmitt trigger (libopenmpt's 8.85 ≈ the 9.28 Hz a mean-crossing
count reports for this exact fixture).

The libopenmpt figures above are a one-off cross-check, not part of the harness,
which is scoped to libxmp. To reproduce:

```sh
openmpt123 --quiet --no-progress --samplerate 44100 --channels 1 --render fixture.mod
```

## Other test files

| File | Pins |
|---|---|
| `container.rs`, `container_adf.rs` | opening plain files, ZIPs, ADFs; PP20 decrunching |
| `probe.rs` | format identification and its confidence |
| `decode_images.rs`, `decode_module.rs` | Format198x dispatch and palette conversion |
| `engine.rs` | timing, Paula rates, panning, seeking, hostile input |
| `effects.rs` | the three effect dispatch tables, one effect at a time |
| `engine_allocations.rs` | `render` allocates nothing |
| `duration.rs` | silent playthrough and loop detection |
| `metadata.rs` | what an interface shows about a work |
| `public_api.rs` | the error type's distinctions |
| `common/mod.rs` | synthetic fixtures and the shared measurement helpers |
