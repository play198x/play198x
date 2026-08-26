//! The engine measured against libxmp, an implementation nobody here wrote.
//!
//! Everything else in this crate is measured against itself: arithmetic this
//! engine believes, checked by tests its author wrote. That catches mistakes,
//! but it cannot catch a *shared* misreading of the replayer. libxmp read the
//! same assembly independently — where it agrees, the reading is probably
//! right, and where it disagrees one of the two has something to learn.
//!
//! Sample-exact comparison is not achievable, and pretending otherwise would
//! waste the harness: different interpolation and different mixing make the
//! waveforms legitimately differ. So every comparison here is of a *derived*
//! measure — carrier pitch, onset time, per-effect rate, effect depth,
//! envelope shape.
//!
//! Fixtures are the synthetic single-effect modules the effect tests use: one
//! held note and one effect. Real music confounds the measurement; an attempt
//! on 2026-08-25 measured a real module's vibrato and got the *row* rate,
//! because at speed 5 the 0.100 s row period dominated the envelope.
//!
//! `tests/README.md` records what this harness measures, what it deliberately
//! does not, and the one place the two players differ on purpose.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Cell, SampleSpec, left, modulation_hz, module_bytes, pitch_track, square};
use play198x_core::engine::Engine;
use std::path::PathBuf;
use std::process::Command;

const RATE: u32 = 44_100;

/// Frames in one row at the default speed 6 and tempo 125: 6 * 882.
const ROW_FRAMES: usize = 5_292;

/// The PAL Amiga clock, for turning a measured carrier back into a period.
const PAL_CLOCK: f64 = 7_093_789.2;

/// Bytes in one cycle of the fixtures' square-wave sample.
const BYTES_PER_CYCLE: f64 = 32.0;

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

/// One test's measurements, printed as a single block.
///
/// Buffered rather than printed line by line because `cargo test` runs these
/// in parallel and five interleaved tables are unreadable. The block is
/// flushed on drop, so a test that fails an assertion still prints the numbers
/// that led to the failure — which are the whole point of the harness.
struct Report(Vec<String>);

impl Report {
    fn new(heading: &str) -> Self {
        Self(vec![format!("\n{heading}")])
    }

    /// A value only this engine or only the replayer has — no counterpart to
    /// compare it against.
    fn note(&mut self, label: &str, value: f64, unit: &str) {
        self.0.push(format!("  {label:<30} {value:>11.4} {unit}"));
    }

    /// Both players' readings of one measure, and the gap between them as a
    /// percentage of ours. Returns that percentage so the caller can assert on
    /// it without re-deriving it.
    fn compare(&mut self, label: &str, ours: f64, theirs: f64, unit: &str) -> f64 {
        let delta = (theirs - ours) / ours * 100.0;
        self.0.push(format!(
            "  {label:<30} ours {ours:>11.4} {unit:<9} xmp {theirs:>11.4} {unit:<9} {delta:+7.2}%"
        ));
        delta
    }
}

impl Drop for Report {
    fn drop(&mut self) {
        println!("{}", self.0.join("\n"));
    }
}

// ---------------------------------------------------------------------------
// The other player
// ---------------------------------------------------------------------------

/// The `xmp` binary, or a panic.
///
/// **Panic, never return.** A test that quietly returns when its tool is
/// missing looks exactly like a test that ran and passed, and that failure
/// mode is the reason this file exists at all. The `#[ignore]` is what keeps
/// the harness off the default run; asking for it has to produce an answer.
fn xmp_binary() -> PathBuf {
    let path = PathBuf::from(std::env::var("XMP").unwrap_or_else(|_| "xmp".into()));
    match Command::new(&path).arg("-V").output() {
        Ok(out) if out.status.success() => path,
        Ok(out) => panic!(
            "`{} -V` exited {} — this test must not pass by skipping",
            path.display(),
            out.status
        ),
        Err(error) => panic!(
            "`{}` could not be run ({error}) — this test must not pass by \
             skipping. Install it (`brew install xmp`) or point XMP at it.",
            path.display()
        ),
    }
}

/// A scratch path nothing else in this process will claim.
///
/// `xmp` reads a module from a file and writes a WAV to one, so the harness
/// needs two real paths. They live in the system temp directory and are
/// removed on the way out: no media enters the repository, generated or not.
fn scratch(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "play198x-differential-{}-{name}",
        std::process::id()
    ));
    path
}

/// Render `bytes` through `xmp` for at least `seconds`, as interleaved stereo.
///
/// Mono, then duplicated across both channels, so the shared measurement
/// helpers — which read the left channel of an interleaved buffer — apply
/// unchanged to both players' output. Taking xmp's left channel instead would
/// fold its default pan separation into every amplitude measured.
///
/// `nearest` interpolation because that is what this engine does, and because
/// it is the closest of the three xmp offers to what Paula does. Comparing our
/// nearest-neighbour against xmp's default spline would measure the
/// interpolator rather than the replayer.
///
/// `--time` takes **whole seconds**: xmp parses it with `atoi`, so `--time 1.6`
/// asks for one second and quietly returns a buffer that ends before the
/// window being measured — which is exactly how this harness first read "0
/// waveform cycles" out of a perfectly good render. The bound is rounded up,
/// and the length actually returned is checked rather than assumed.
fn render_with_xmp(label: &str, bytes: &[u8], seconds: f64) -> Vec<f32> {
    let xmp = xmp_binary();
    let module = scratch(&format!("{label}.mod"));
    let wav = scratch(&format!("{label}.wav"));
    std::fs::write(&module, bytes).unwrap();

    let output = Command::new(&xmp)
        .args(["--norc", "--nocmd", "--quiet"])
        .args(["--player-mode", "protracker"])
        .args(["--interpolation", "nearest"])
        .args(["--amplify", "0"])
        .args(["--bits", "16"])
        .arg("--mono")
        .arg("--nofilter")
        .args(["--frequency", &RATE.to_string()])
        .args(["--time", &format!("{}", seconds.ceil() as u64 + 1)])
        .arg("--output-file")
        .arg(&wav)
        .arg(&module)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "xmp exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let wave = std::fs::read(&wav).unwrap();
    let _ = std::fs::remove_file(&module);
    let _ = std::fs::remove_file(&wav);
    let frames = mono_wav_to_stereo(&wave);
    let rendered = frames.len() as f64 / 2.0 / f64::from(RATE);
    assert!(
        rendered >= seconds,
        "xmp returned {rendered:.3} s for {label}, short of the {seconds:.3} s \
         being measured"
    );
    frames
}

/// The samples of a 16-bit mono RIFF/WAVE file, duplicated to stereo.
///
/// Chunks are walked rather than assumed to start at byte 44: xmp writes a
/// placeholder header first and rewrites it at close, and a hard-coded offset
/// would silently read the tail of a header as audio.
fn mono_wav_to_stereo(wave: &[u8]) -> Vec<f32> {
    assert_eq!(&wave[..4], b"RIFF", "not a RIFF file");
    assert_eq!(&wave[8..12], b"WAVE", "not a WAVE file");
    let mut at = 12;
    let mut channels = 0u16;
    let mut bits = 0u16;
    while at + 8 <= wave.len() {
        let id = &wave[at..at + 4];
        let size = u32::from_le_bytes(wave[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &wave[at + 8..(at + 8 + size).min(wave.len())];
        match id {
            b"fmt " => {
                channels = u16::from_le_bytes(body[2..4].try_into().unwrap());
                bits = u16::from_le_bytes(body[14..16].try_into().unwrap());
            }
            b"data" => {
                assert_eq!(channels, 1, "the harness asks xmp for mono");
                assert_eq!(bits, 16, "the harness asks xmp for 16-bit");
                return body
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .flat_map(|pair| {
                        let value = f32::from(i16::from_le_bytes(*pair)) / 32_768.0;
                        [value, value]
                    })
                    .collect();
            }
            _ => {}
        }
        at += 8 + size + (size & 1);
    }
    panic!("no data chunk in the WAV xmp wrote");
}

/// Render a fixture through this engine, as interleaved stereo frames.
fn render_with_ours(bytes: &[u8], seconds: f64) -> Vec<f32> {
    let module = play198x_core::decode::module(bytes).unwrap();
    let frames = (f64::from(RATE) * seconds) as usize;
    let mut buf = vec![0f32; frames * 2];
    let mut engine = Engine::new(module, RATE);
    engine.render(&mut buf);
    buf
}

// ---------------------------------------------------------------------------
// Fixtures — one held note, one effect
// ---------------------------------------------------------------------------

/// A looping square wave, one cycle every 32 bytes.
fn square_sample(volume: u8) -> SampleSpec {
    SampleSpec {
        data: square(32, 1, 100),
        volume,
        repeat_start_words: 0,
        repeat_length_words: 16,
    }
}

/// The file bytes of a module whose row 0 plays C-2 with `(effect, param)` and
/// whose remaining rows repeat the effect with no new note.
fn held_bytes(effect: u8, param: u8, rows: usize) -> Vec<u8> {
    held_bytes_with_sample(square_sample(64), effect, param, rows)
}

fn held_bytes_with_sample(sample: SampleSpec, effect: u8, param: u8, rows: usize) -> Vec<u8> {
    let cells = (0..rows)
        .map(|row| Cell {
            row,
            channel: 0,
            sample: if row == 0 { 1 } else { 0 },
            period: if row == 0 { 428 } else { 0 },
            effect,
            param,
        })
        .collect();
    module_bytes(&[sample], &[cells], &[0], 1)
}

// ---------------------------------------------------------------------------
// Derived measures
// ---------------------------------------------------------------------------

/// Peak amplitude of the left channel per 10 ms window: `(seconds, level)`.
///
/// Peak rather than RMS, and 10 ms rather than a tick: half a tick, so a
/// per-tick volume step lands in a window of its own instead of being averaged
/// across the step that produced it.
fn envelope(interleaved: &[f32], sample_rate: f64) -> Vec<(f64, f64)> {
    let window = (sample_rate / 100.0) as usize;
    let samples: Vec<f32> = left(interleaved).collect();
    samples
        .chunks_exact(window)
        .enumerate()
        .map(|(index, chunk)| {
            let peak = chunk.iter().fold(0f32, |peak, v| peak.max(v.abs()));
            ((index * window) as f64 / sample_rate, f64::from(peak))
        })
        .collect()
}

/// Pearson correlation of two envelopes over their common length.
///
/// Correlation and not a difference, because the two players' absolute levels
/// differ by mixer gain alone: xmp's downmix and this engine's headroom
/// scaling are both arbitrary constants, and a measure sensitive to them would
/// report a disagreement that is not one. Shape is the claim.
///
/// A flat envelope has zero variance and no shape to correlate, so it is
/// refused rather than divided by: the `NaN` that comes out otherwise reads as
/// a failure only if something happens to test for it, and as agreement
/// everywhere else.
fn pearson(a: &[(f64, f64)], b: &[(f64, f64)]) -> f64 {
    let n = a.len().min(b.len());
    assert!(n > 50, "{n} envelope windows is too few to correlate");
    let a: Vec<f64> = a[..n].iter().map(|(_, v)| *v).collect();
    let b: Vec<f64> = b[..n].iter().map(|(_, v)| *v).collect();
    let mean = |v: &[f64]| v.iter().sum::<f64>() / n as f64;
    let (ma, mb) = (mean(&a), mean(&b));
    let mut cov = 0.0;
    let mut va = 0.0;
    let mut vb = 0.0;
    for i in 0..n {
        let (da, db) = (a[i] - ma, b[i] - mb);
        cov += da * db;
        va += da * da;
        vb += db * db;
    }
    assert!(
        va > 0.0 && vb > 0.0,
        "a flat envelope has no shape to correlate"
    );
    cov / (va.sqrt() * vb.sqrt())
}

/// When the left channel first reaches a tenth of its eventual peak, in
/// seconds.
///
/// A fraction of the render's own peak rather than an absolute threshold,
/// because the two players' gains differ; a tenth is far above either one's
/// noise floor and far below the square wave's plateau, so the crossing lands
/// on the attack and not on a ripple.
fn onset_seconds(interleaved: &[f32], sample_rate: f64) -> f64 {
    let samples: Vec<f32> = left(interleaved).collect();
    let peak = samples.iter().fold(0f32, |peak, v| peak.max(v.abs()));
    assert!(peak > 0.0, "silence has no onset");
    let at = samples
        .iter()
        .position(|v| v.abs() > peak / 10.0)
        .expect("a buffer with a peak has a tenth of one");
    at as f64 / sample_rate
}

/// The `fraction`-th and `1 - fraction`-th values of a series, sorted.
///
/// Percentiles rather than min and max: a pitch track carries one wild value
/// wherever a note starts or an effect steps, and an extreme would report that
/// artefact as the swing.
fn spread(mut values: Vec<f64>, fraction: f64) -> (f64, f64) {
    assert!(values.len() > 40, "too few values for a percentile spread");
    values.sort_by(f64::total_cmp);
    let at = (values.len() as f64 * fraction) as usize;
    (values[at], values[values.len() - 1 - at])
}

/// The pitch of every waveform cycle after `skip` rows, in Hz.
fn carrier_track(interleaved: &[f32], skip_rows: usize, rows: usize) -> Vec<f64> {
    let track = pitch_track(interleaved, f64::from(RATE));
    let from = (skip_rows * ROW_FRAMES) as f64 / f64::from(RATE);
    let to = ((skip_rows + rows) * ROW_FRAMES) as f64 / f64::from(RATE);
    let window: Vec<f64> = track
        .iter()
        .filter(|(at, _)| *at >= from && *at < to)
        .map(|(_, hz)| *hz)
        .collect();
    assert!(
        window.len() > 20,
        "{} cycles between {from:.3} s and {to:.3} s is too few to measure",
        window.len()
    );
    window
}

/// The median carrier pitch over a span of rows, in Hz.
///
/// Median, because a mean would let the handful of cycles straddling a note
/// start or an effect step move the answer.
fn median_carrier_hz(interleaved: &[f32], skip_rows: usize, rows: usize) -> f64 {
    let mut window = carrier_track(interleaved, skip_rows, rows);
    window.sort_by(f64::total_cmp);
    window[window.len() / 2]
}

/// The Amiga period a measured carrier pitch implies.
fn period_from_carrier(hz: f64) -> f64 {
    PAL_CLOCK / (2.0 * hz * BYTES_PER_CYCLE)
}

// ---------------------------------------------------------------------------
// The comparisons
// ---------------------------------------------------------------------------

#[test]
#[ignore = "needs the xmp CLI; run with -- --ignored"]
fn the_carrier_pitch_and_the_note_onset_agree_with_libxmp() {
    // A held C-2 with no effect at all. If the two players disagree here then
    // every later measurement is measuring that disagreement and nothing else,
    // so this one runs first and states the floor.
    let mut report = Report::new("held C-2, no effect");
    let bytes = held_bytes(0x00, 0x00, 16);
    let ours = render_with_ours(&bytes, 1.6);
    let theirs = render_with_xmp("carrier", &bytes, 1.6);

    // Period 428 on PAL is 8287.14 bytes/s, and the sample is one square cycle
    // every 32 bytes: 258.97 Hz. Both players read 259.41 instead, because a
    // cycle of 170.29 frames at 44.1 kHz can only be counted as 170 — the same
    // quantisation, from the same nearest-neighbour rule, in both.
    let expected = PAL_CLOCK / (2.0 * 428.0) / BYTES_PER_CYCLE;
    let ours_hz = median_carrier_hz(&ours, 1, 12);
    let theirs_hz = median_carrier_hz(&theirs, 1, 12);
    report.note("replayer-derived", expected, "Hz");
    let pitch_delta = report.compare("carrier pitch", ours_hz, theirs_hz, "Hz");

    let ours_onset = onset_seconds(&ours, f64::from(RATE)) * 1e3;
    let theirs_onset = onset_seconds(&theirs, f64::from(RATE)) * 1e3;
    report.compare(
        "note onset",
        ours_onset.max(1e-9),
        theirs_onset.max(1e-9),
        "ms",
    );

    // An unmodulated square wave has a flat envelope, so there is no shape to
    // correlate here; `pearson` refuses it and the envelope comparisons live
    // on the fixtures that move one. What is worth checking is that it really
    // is flat in both, which is what a mixer bug — a decaying voice, a drifting
    // gain — would break first.
    let ripple = |buf: &[f32]| {
        let env: Vec<f64> = envelope(buf, f64::from(RATE))
            .into_iter()
            .skip(20)
            .map(|(_, v)| v)
            .collect();
        let mean = env.iter().sum::<f64>() / env.len() as f64;
        let (lo, hi) = spread(env, 0.02);
        (hi - lo) / mean * 100.0
    };
    report.compare(
        "envelope ripple",
        ripple(&ours).max(1e-9),
        ripple(&theirs).max(1e-9),
        "%",
    );

    // Measured 2026-08-26, xmp 4.3.1 / libxmp 4.7.2: carrier +0.00%, both
    // 259.4118 Hz; onsets both 0.0000 ms; ripple 0.00% in both. The tolerances
    // are an order of magnitude wider than the observations rather than picked
    // to taste — there is nothing here for two nearest-neighbour resamplers of
    // the same 8-bit table to differ about, so any gap at all is a defect.
    assert!(
        pitch_delta.abs() < 0.5,
        "carrier pitch differs by {pitch_delta:+.3}%, measured +0.00%"
    );
    assert!(
        (ours_hz - expected).abs() / expected < 0.005,
        "our carrier {ours_hz:.4} Hz is not the replayer-derived {expected:.4} \
         Hz within the 0.17% the frame grid quantises it by"
    );
    assert!(
        (theirs_onset - ours_onset).abs() < 1.0,
        "onsets {:.4} ms apart, measured 0.0000 ms",
        theirs_onset - ours_onset
    );
    assert!(
        ripple(&ours) < 1.0 && ripple(&theirs) < 1.0,
        "a held note's envelope is not flat: ours {:.3}%, xmp {:.3}%",
        ripple(&ours),
        ripple(&theirs)
    );
}

#[test]
#[ignore = "needs the xmp CLI; run with -- --ignored"]
fn the_vibrato_rate_and_depth_agree_with_libxmp() {
    // 4xF held, speed 6. Vibrato is a `fx_tab` effect, so its position advances
    // on `speed - 1` ticks a row: (x * 5) / 64 cycles per 120 ms row.
    //
    // This is the measure the reference document singles out as the one where
    // implementations disagree, so it is worth stating plainly what the harness
    // finds: with xmp put in ProTracker mode, it does not. Both players land on
    // the replayer's figure and neither is near the community formula's.
    for (nibble, expected) in [(0xAu8, 6.5104f64), (0x5, 3.2552)] {
        let mut report = Report::new(&format!("held C-2 with vibrato 4{nibble:X}F, speed 6"));
        let bytes = held_bytes(0x04, (nibble << 4) | 0x0F, 64);
        let ours = render_with_ours(&bytes, 4.0);
        let theirs = render_with_xmp(&format!("vibrato{nibble:X}"), &bytes, 4.0);

        let ours_hz = modulation_hz(&pitch_track(&ours, f64::from(RATE)));
        let theirs_hz = modulation_hz(&pitch_track(&theirs, f64::from(RATE)));
        report.note("replayer-derived", expected, "Hz");
        let rate_delta = report.compare("vibrato rate", ours_hz, theirs_hz, "Hz");

        // (x * ticks) / 64 — the community specification's version, which the
        // reference records as wrong. Printed because it is the only way to see
        // that the gap between the two players is small next to the error
        // actually at stake.
        report.note("community spec (20% high)", expected * 6.0 / 5.0, "Hz");

        // Depth. The replayer's table entry is `SINE[pos & 31] * amplitude /
        // 128`, so amplitude 15 swings the period by at most 255 * 15 / 128 =
        // 29 either way. Measured as a period, because that is the quantity the
        // replayer adds to; the carrier is only how it is observed.
        let swing = |buf: &[f32]| {
            let (lo, hi) = spread(carrier_track(buf, 2, 30), 0.05);
            (period_from_carrier(lo) - period_from_carrier(hi)) / 2.0
        };
        report.note("replayer-derived depth", 29.0, "periods");
        let depth_delta = report.compare("vibrato depth", swing(&ours), swing(&theirs), "periods");

        assert!(
            (ours_hz - expected).abs() / expected < 0.02,
            "our vibrato {ours_hz:.4} Hz is not the replayer-derived \
             {expected:.4} Hz within 2%"
        );
        // Measured 2026-08-26, xmp 4.3.1 / libxmp 4.7.2: rate -0.29% at speed
        // A and -0.34% at speed 5; depth -4.35% and +0.00%. 3% on the rate is
        // wide enough for the Schmitt trigger's half-cycle granularity over a
        // four-second span and far narrower than the 20% that separates the
        // replayer from the community formula, so a player using the wrong one
        // still cannot satisfy it. 8% on the depth covers reading a period back
        // out of a quantised zero-crossing count, which is what the -4.35% is:
        // one bin of a carrier counted in whole frames.
        assert!(
            rate_delta.abs() < 3.0,
            "vibrato rate differs from xmp by {rate_delta:+.2}%, measured -0.29%"
        );
        assert!(
            depth_delta.abs() < 8.0,
            "vibrato depth differs from xmp by {depth_delta:+.2}%, measured -4.35%"
        );
    }
}

#[test]
#[ignore = "needs the xmp CLI; run with -- --ignored"]
fn the_portamento_slide_rate_agrees_with_libxmp() {
    // 204 held: portamento down adds 4 to the period on every `fx_tab` tick,
    // so five ticks a row at speed 6 is 20 periods a row. Measured as a carrier
    // pitch turned back into a period — what a listener hears, rather than what
    // the register holds, which is what the engine's own tests already pin.
    let mut report = Report::new("held C-2 with portamento down 204, speed 6");
    let bytes = held_bytes(0x02, 0x04, 16);
    let ours = render_with_ours(&bytes, 1.6);
    let theirs = render_with_xmp("portamento", &bytes, 1.6);

    let slide = |buf: &[f32]| {
        let early = period_from_carrier(median_carrier_hz(buf, 1, 2));
        let late = period_from_carrier(median_carrier_hz(buf, 9, 2));
        (late - early) / 8.0
    };
    let ours_rate = slide(&ours);
    let theirs_rate = slide(&theirs);
    report.note("replayer-derived", 20.0, "periods/row");
    let delta = report.compare("portamento slide", ours_rate, theirs_rate, "per/row");

    assert!(
        (ours_rate - 20.0).abs() < 0.5,
        "our slide is {ours_rate:.4} periods a row, not the 20 the replayer's \
         five fx_tab ticks give"
    );
    // Measured 2026-08-26, xmp 4.3.1 / libxmp 4.7.2: +0.00% — both read
    // 20.1071 periods a row, the 20 the replayer gives plus the same
    // quantisation. 3% is wide enough
    // for the quantisation in reading a period back out of a zero-crossing
    // count and narrow enough to catch a six-tick slide, which would read 24
    // periods a row — 20% out.
    assert!(
        delta.abs() < 3.0,
        "portamento slide differs by {delta:+.2}%, measured +0.00%"
    );
}

#[test]
#[ignore = "needs the xmp CLI; run with -- --ignored"]
fn the_volume_slide_envelope_agrees_with_libxmp() {
    // A01 from full volume: one unit off per `fx_tab` tick, five a row, so the
    // envelope falls linearly to silence over about 12.8 rows. Absolute levels
    // are not comparable between players — mixer gain is an arbitrary constant
    // — so the fall is stated as a fraction of each player's own full-scale
    // level, which is the very first 10 ms window, before the first slide tick
    // at 20 ms.
    let mut report = Report::new("held C-2 with volume slide A01, speed 6");
    let bytes = held_bytes(0x0A, 0x01, 16);
    let ours = render_with_ours(&bytes, 1.6);
    let theirs = render_with_xmp("volumeslide", &bytes, 1.6);

    let fall = |buf: &[f32]| {
        let env = envelope(buf, f64::from(RATE));
        let at = |row: usize| {
            let want = (row * ROW_FRAMES) as f64 / f64::from(RATE) + 0.06;
            env.iter()
                .min_by(|a, b| (a.0 - want).abs().total_cmp(&(b.0 - want).abs()))
                .unwrap()
                .1
        };
        (at(1) - at(9)) / env[0].1 / 8.0
    };
    // Five units a row off a full-scale 64 is 5/64 = 0.078125.
    report.note("replayer-derived", 0.078_125, "of full/row");
    let ours_fall = fall(&ours);
    let slide_delta = report.compare("volume slide", ours_fall, fall(&theirs), "frac/row");

    let correlation = pearson(
        &envelope(&ours, f64::from(RATE)),
        &envelope(&theirs, f64::from(RATE)),
    );
    report.note("envelope correlation", correlation, "");

    assert!(
        (ours_fall - 0.078_125).abs() / 0.078_125 < 0.02,
        "our volume slide falls {ours_fall:.5} of full scale a row, not the \
         0.07813 five fx_tab ticks give"
    );
    // Measured 2026-08-26, xmp 4.3.1 / libxmp 4.7.2: slide +0.00%, correlation
    // 1.000000. Both players step the same integer volume on the same ticks,
    // so 2% and 0.99 are slack, not calibration.
    assert!(
        slide_delta.abs() < 2.0,
        "volume slide differs by {slide_delta:+.2}%, measured +0.00%"
    );
    assert!(
        correlation > 0.99,
        "envelope correlation {correlation:.6}, measured 1.000000"
    );
}

#[test]
#[ignore = "needs the xmp CLI; run with -- --ignored"]
fn the_tremolo_rate_agrees_with_libxmp_but_its_depth_is_twice_ours() {
    // 7A4 from a stored volume of 32. Amplitude 4 and a mid-scale base so that
    // neither player's swing reaches 0 or 64: tremolo clamps at both ends, and
    // a clamped envelope is a flat-topped wave whose modulation rate reads
    // 8.13 Hz instead of 6.51 — the measurement this harness got first, from a
    // fixture at full volume, and reported as a rate disagreement that was
    // really an amplitude one.
    //
    // The depth *is* a real disagreement, and it is recorded rather than
    // tolerated: see tests/README.md.
    let mut report = Report::new("held C-2 with tremolo 7A4 from volume 32, speed 6");
    let bytes = held_bytes_with_sample(square_sample(32), 0x07, 0xA4, 64);
    let ours = render_with_ours(&bytes, 4.0);
    let theirs = render_with_xmp("tremolo", &bytes, 4.0);

    let ours_env = envelope(&ours, f64::from(RATE));
    let theirs_env = envelope(&theirs, f64::from(RATE));

    // Tremolo shares the vibrato machinery, so the rate is the same expression:
    // (x * 5) / 64 cycles a row.
    report.note("replayer-derived rate", 6.5104, "Hz");
    let rate_delta = report.compare(
        "tremolo rate",
        modulation_hz(&ours_env),
        modulation_hz(&theirs_env),
        "Hz",
    );

    // Depth as a fraction of the un-modulated level, so the two players' mixer
    // gains cancel. The replayer's table gives `SINE[pos & 31] * 4 / 128`, at
    // most 7 either way, against a stored volume of 32: 7/32 = 0.21875.
    let depth = |env: &[(f64, f64)]| {
        let values: Vec<f64> = env.iter().skip(20).map(|(_, v)| *v).collect();
        let (lo, hi) = spread(values, 0.05);
        (hi - lo) / (hi + lo)
    };
    report.note("replayer-derived depth", 7.0 / 32.0, "of level");
    let ours_depth = depth(&ours_env);
    let theirs_depth = depth(&theirs_env);
    let depth_delta = report.compare("tremolo depth", ours_depth, theirs_depth, "of level");
    report.note("depth ratio, xmp / ours", theirs_depth / ours_depth, "x");

    // Pearson over the two envelopes. High correlation beside a depth ratio of
    // 2 is the finding in two numbers: same waveform, same phase, same rate,
    // twice the amplitude. It reads 0.91 rather than 1.00 because the envelope
    // is a peak-per-window reading of an integer volume, and a swing of +/-7
    // lands on half as many distinct levels as one of +/-15 — the quantisation
    // differs even though the shape does not.
    let correlation = pearson(&ours_env, &theirs_env);
    report.note("envelope correlation", correlation, "");

    assert!(
        (ours_depth - 7.0 / 32.0).abs() / (7.0 / 32.0) < 0.05,
        "our tremolo swings {ours_depth:.5} of the level, not the 0.21875 the \
         replayer's table gives for amplitude 4 at volume 32"
    );
    // Measured 2026-08-26, xmp 4.3.1 / libxmp 4.7.2: rate -0.02%, depth
    // +114.29%, ratio 2.1429, correlation 0.9125.
    assert!(
        rate_delta.abs() < 3.0,
        "tremolo rate differs by {rate_delta:+.2}%, measured -0.02%"
    );
    assert!(
        correlation > 0.85,
        "tremolo envelope correlation {correlation:.6}, measured 0.9125 — the \
         two swings are meant to differ in depth only, not in shape"
    );
    // The depth ratio is asserted rather than ignored, so this stays a recorded
    // difference and not a blind spot. If libxmp ever matches the replayer this
    // fails — and the fix then is to tighten the bound towards 1 and delete the
    // README entry, never to widen it.
    assert!(
        (1.9..2.4).contains(&(theirs_depth / ours_depth)),
        "xmp's tremolo is {:.3}x ours; the recorded difference is 2.14x, and a \
         different figure is a new finding to read the replayer about",
        theirs_depth / ours_depth
    );
    assert!(
        depth_delta > 0.0,
        "the recorded difference is xmp swinging deeper, not shallower"
    );
}

#[test]
#[ignore = "needs the xmp CLI; run with -- --ignored"]
fn the_sample_offset_onset_agrees_with_libxmp() {
    // A sample whose first 1024 bytes are silent. Without an offset it begins
    // sounding at 1024 / 8287.14 = 123.6 ms; `904` starts playback 1024 bytes
    // in, so it sounds immediately. The measure is when, not what.
    let mut data = vec![0u8; 1_024];
    data.extend(square(32, 64, 100));
    let sample = || SampleSpec {
        data: data.clone(),
        volume: 64,
        repeat_start_words: 512,
        repeat_length_words: 16,
    };

    let mut report = Report::new("silence-then-square sample, 900 and 904");
    for (param, expected_ms) in [(0x00u8, 123.57f64), (0x04, 0.0)] {
        let bytes = held_bytes_with_sample(sample(), 0x09, param, 8);
        let ours = render_with_ours(&bytes, 0.8);
        let theirs = render_with_xmp(&format!("offset{param:02X}"), &bytes, 0.8);
        let ours_ms = onset_seconds(&ours, f64::from(RATE)) * 1e3;
        let theirs_ms = onset_seconds(&theirs, f64::from(RATE)) * 1e3;
        report.note(
            &format!("replayer-derived, 9{param:02X}"),
            expected_ms,
            "ms",
        );
        report.compare(
            &format!("onset with 9{param:02X}"),
            ours_ms.max(1e-9),
            theirs_ms.max(1e-9),
            "ms",
        );

        assert!(
            (ours_ms - expected_ms).abs() < 1.0,
            "our onset with 9{param:02X} is {ours_ms:.4} ms, not the \
             {expected_ms:.4} ms the sample rate gives"
        );
        // Measured 2026-08-26, xmp 4.3.1 / libxmp 4.7.2: 0.068 ms apart with
        // 900 (123.5828 against 123.5147, either side of the 123.57 the sample
        // rate gives) and 0.0000 ms apart with 904. 1 ms is 44 frames — wider
        // than the observation, and far narrower than the 123.6 ms the effect
        // itself is worth.
        assert!(
            (theirs_ms - ours_ms).abs() < 1.0,
            "onsets with 9{param:02X} are {:.4} ms apart, measured 0.068 ms \
             with 900 and 0.0000 ms with 904",
            theirs_ms - ours_ms
        );
    }
}
