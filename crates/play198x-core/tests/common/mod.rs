//! Synthetic modules, built in code.
//!
//! No media file is ever committed to this repository, so every engine test
//! assembles the bytes it needs and runs them through the real decoder. Going
//! through `decode::module` rather than constructing a `Module` by hand is
//! deliberate: a fixture the shipped parser rejects is a fixture that proves
//! nothing about playback.
#![allow(dead_code, clippy::unwrap_used)]

use format198x_commodore_amiga_mod::Module;

/// One sample slot, in the terms the file header stores.
pub struct SampleSpec {
    /// Raw PCM, signed 8-bit. Must be an even number of bytes: the header
    /// counts words, so an odd length is not expressible.
    pub data: Vec<u8>,
    pub volume: u8,
    pub repeat_start_words: u16,
    pub repeat_length_words: u16,
}

impl SampleSpec {
    /// A silent, unused slot — what 30 of the 31 headers are in these fixtures.
    pub fn empty() -> Self {
        Self {
            data: Vec::new(),
            volume: 0,
            repeat_start_words: 0,
            repeat_length_words: 0,
        }
    }
}

/// One pattern cell worth setting. Everything not named is zero, which is the
/// format's own "nothing happens here".
///
/// `Default` exists so a cell can name only the fields it cares about:
/// most tests set a note and no effect, and every effect test sets an effect
/// on a row that may or may not carry a note.
#[derive(Default)]
pub struct Cell {
    pub row: usize,
    pub channel: usize,
    /// 1-based, as the file stores it. 0 means "no sample change".
    pub sample: u8,
    /// Amiga period. 0 means "no note".
    pub period: u16,
    /// Effect number, 0..=15. The high nibble of the cell's third byte.
    pub effect: u8,
    /// Effect parameter byte.
    pub param: u8,
}

/// A module's text fields: the title, and a name for any of the 31 sample
/// slots.
///
/// Raw bytes rather than `&str` because Amiga text is ISO-8859-1, and the
/// names worth pinning are exactly the ones a UTF-8 reading would lose.
pub struct Text<'a> {
    /// The title field's content, before its NUL padding. Truncated at 20
    /// bytes, which is all the field holds.
    pub title: &'a [u8],
    /// Name bytes per slot, in slot order, truncated at 22 bytes each. Slots
    /// past the end of this list are left blank.
    ///
    /// A slot named here need not carry any sample data, and the metadata
    /// tests rely on that: authors hid messages in the slots a song never
    /// plays.
    pub sample_names: &'a [&'a [u8]],
}

impl Default for Text<'_> {
    fn default() -> Self {
        Self {
            title: b"FIXTURE",
            sample_names: &[],
        }
    }
}

/// Assemble a four-channel `M.K.` module and decode it.
///
/// `song_length` is passed separately from `orders` so a test can state a song
/// length that disagrees with the table it supplies — which is the only way to
/// pin that the played prefix is bounded by `song_length` and not by the table.
pub fn module(
    samples: &[SampleSpec],
    patterns: &[Vec<Cell>],
    orders: &[u8],
    song_length: u8,
) -> Module {
    module_with_text(&Text::default(), samples, patterns, orders, song_length)
}

/// As [`module`], with the title and the sample names stated.
pub fn module_with_text(
    text: &Text<'_>,
    samples: &[SampleSpec],
    patterns: &[Vec<Cell>],
    orders: &[u8],
    song_length: u8,
) -> Module {
    play198x_core::decode::module(&module_bytes_with_text(
        text,
        samples,
        patterns,
        orders,
        song_length,
    ))
    .unwrap()
}

/// The same fixture as [`module`], left as file bytes.
///
/// The differential harness hands these to another player, which reads a file
/// and not a `Module`. Every other test decodes them, so the bytes an external
/// player sees are the same bytes the shipped parser accepts — a fixture only
/// one of the two would take proves nothing about either.
pub fn module_bytes(
    samples: &[SampleSpec],
    patterns: &[Vec<Cell>],
    orders: &[u8],
    song_length: u8,
) -> Vec<u8> {
    module_bytes_with_text(&Text::default(), samples, patterns, orders, song_length)
}

/// As [`module_bytes`], with the title and the sample names stated.
pub fn module_bytes_with_text(
    text: &Text<'_>,
    samples: &[SampleSpec],
    patterns: &[Vec<Cell>],
    orders: &[u8],
    song_length: u8,
) -> Vec<u8> {
    let mut out = vec![0u8; 20];
    let title = &text.title[..text.title.len().min(20)];
    out[..title.len()].copy_from_slice(title);

    for slot in 0..31 {
        let spec = samples.get(slot);
        let mut header = vec![0u8; 30];
        if let Some(name) = text.sample_names.get(slot) {
            let name = &name[..name.len().min(22)];
            header[..name.len()].copy_from_slice(name);
        }
        if let Some(spec) = spec {
            let words = u16::try_from(spec.data.len() / 2).unwrap();
            header[22..24].copy_from_slice(&words.to_be_bytes());
            header[25] = spec.volume;
            header[26..28].copy_from_slice(&spec.repeat_start_words.to_be_bytes());
            header[28..30].copy_from_slice(&spec.repeat_length_words.to_be_bytes());
        }
        out.extend_from_slice(&header);
    }

    out.push(song_length);
    out.push(0); // restart position: a Noisetracker leftover PT2.3 ignores
    let mut table = [0u8; 128];
    table[..orders.len()].copy_from_slice(orders);
    out.extend_from_slice(&table);
    out.extend_from_slice(b"M.K.");

    for cells in patterns {
        let mut pattern = vec![0u8; 64 * 4 * 4];
        for cell in cells {
            let at = (cell.row * 4 + cell.channel) * 4;
            pattern[at] = (cell.sample & 0xF0) | u8::try_from(cell.period >> 8).unwrap();
            pattern[at + 1] = (cell.period & 0xFF) as u8;
            pattern[at + 2] = ((cell.sample & 0x0F) << 4) | (cell.effect & 0x0F);
            pattern[at + 3] = cell.param;
        }
        out.extend_from_slice(&pattern);
    }

    for spec in samples {
        out.extend_from_slice(&spec.data);
    }

    out
}

/// `cycles` cycles of a square wave, `bytes_per_cycle` bytes each, at
/// amplitude `level`.
pub fn square(bytes_per_cycle: usize, cycles: usize, level: i8) -> Vec<u8> {
    (0..bytes_per_cycle * cycles)
        .map(|i| {
            if i % bytes_per_cycle < bytes_per_cycle / 2 {
                level as u8
            } else {
                (-level) as u8
            }
        })
        .collect()
}

/// The playback rate PAL Paula produces for `period`, in bytes per second.
pub fn paula_rate(period: u16) -> f64 {
    7_093_789.2 / (2.0 * f64::from(period))
}

/// Pitch of the left channel, counted per waveform cycle over `window` frames.
///
/// Zero crossings, not autocorrelation: autocorrelation locks onto multiples
/// of the true period and has already reported one rate as slower than another
/// twice its speed on this project. The window must be long — a few
/// milliseconds cannot resolve a 130 Hz carrier at all.
pub fn zero_crossing_hz(interleaved: &[f32], sample_rate: f64, window: usize) -> f64 {
    assert!(
        window as f64 / sample_rate >= 0.1,
        "a pitch window shorter than 100 ms cannot resolve these carriers"
    );
    let mut crossings = 0usize;
    let mut previous_positive = None;
    for frame in interleaved.as_chunks::<2>().0.iter().take(window) {
        let value = frame[0];
        if value == 0.0 {
            continue;
        }
        let positive = value > 0.0;
        if previous_positive.is_some_and(|was| was != positive) {
            crossings += 1;
        }
        previous_positive = Some(positive);
    }
    crossings as f64 / 2.0 * sample_rate / window as f64
}

pub fn left(interleaved: &[f32]) -> impl Iterator<Item = f32> + '_ {
    interleaved.as_chunks::<2>().0.iter().map(|frame| frame[0])
}

/// The pitch of every waveform cycle in the left channel: `(frame, hz)`.
///
/// Per waveform cycle, from positive-going zero crossings — never
/// autocorrelation over a window, which locks onto multiples of the true
/// period and reported one rate as *slower* than another twice its speed on
/// this project on 2026-08-25.
pub fn pitch_track(interleaved: &[f32], sample_rate: f64) -> Vec<(f64, f64)> {
    let mut crossings = Vec::new();
    let mut previous = 0f32;
    for (index, value) in left(interleaved).enumerate() {
        if previous <= 0.0 && value > 0.0 {
            crossings.push(index);
        }
        previous = value;
    }
    crossings
        .windows(2)
        .map(|pair| {
            let cycle = (pair[1] - pair[0]) as f64;
            (pair[0] as f64 / sample_rate, sample_rate / cycle)
        })
        .collect()
}

/// How fast a pitch track wobbles, in Hz.
///
/// A Schmitt trigger, not a plain mean crossing, and the reason is a real
/// ProTracker behaviour rather than measurement fussiness: `4xy` is a `fx_tab`
/// effect, so on the row's *note* tick `morefx_tab` falls through to
/// `mt_pernop` and the period snaps back to the un-wobbled one for 20 ms. That
/// notch sits exactly on the mean and crosses it twice a row, so counting mean
/// crossings measures 9.3 Hz for a 6.51 Hz vibrato — the row rate leaking into
/// the answer again. Thresholds at a third of the swing cannot see a notch
/// that only reaches the middle of it.
///
/// The rate is taken between the first and last upward trigger rather than
/// over the whole buffer, so a partial cycle at either end cannot bias it.
pub fn modulation_hz(track: &[(f64, f64)]) -> f64 {
    assert!(track.len() > 8, "too few waveform cycles to see a wobble");
    let span = track[track.len() - 1].0 - track[0].0;
    assert!(
        span >= 0.5,
        "a {span:.3} s span cannot separate a vibrato from a row rate"
    );
    let mean = track.iter().map(|(_, hz)| hz).sum::<f64>() / track.len() as f64;
    let swing = track
        .iter()
        .map(|(_, hz)| (hz - mean).abs())
        .fold(0.0f64, f64::max);
    let threshold = swing / 3.0;

    let mut state = 0i8;
    let mut upward: Vec<f64> = Vec::new();
    for (at, hz) in track {
        if hz - mean > threshold {
            if state < 0 {
                upward.push(*at);
            }
            state = 1;
        } else if mean - hz > threshold {
            state = -1;
        }
    }
    assert!(
        upward.len() >= 3,
        "only {} wobbles in {span:.2} s is too few to call a rate",
        upward.len()
    );
    (upward.len() - 1) as f64 / (upward[upward.len() - 1] - upward[0])
}
