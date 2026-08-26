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
pub struct Cell {
    pub row: usize,
    pub channel: usize,
    /// 1-based, as the file stores it. 0 means "no sample change".
    pub sample: u8,
    /// Amiga period. 0 means "no note".
    pub period: u16,
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
    let mut out = b"FIXTURE".to_vec();
    out.resize(20, 0);

    for slot in 0..31 {
        let spec = samples.get(slot);
        let mut header = vec![0u8; 30];
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
            pattern[at + 2] = (cell.sample & 0x0F) << 4;
        }
        out.extend_from_slice(&pattern);
    }

    for spec in samples {
        out.extend_from_slice(&spec.data);
    }

    play198x_core::decode::module(&out).unwrap()
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
