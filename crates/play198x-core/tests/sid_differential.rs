#![cfg(feature = "sid")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Rendered-audio differential against sidplayfp. The reference executable and
//! HVSC tune are local tools/corpus inputs; neither is committed.

use play198x_core::player::{Player, pump::FramePump, sid::SidPlayer};
use std::path::PathBuf;
use std::process::Command;

const FRAMES: usize = 96_000;

#[test]
#[ignore = "needs sidplayfp and HVSC #85"]
fn representative_psid_tracks_sidplayfp_audio() {
    let sidplayfp = std::env::var_os("SIDPLAYFP")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/private/tmp/play198x-sidplayfp/src/sidplayfp"));
    let tune = std::env::var_os("PLAY198X_SID_DIFFERENTIAL_TUNE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("/private/tmp/play198x-hvsc85/C64Music/DEMOS/0-9/10_Orbyte.sid")
        });
    let wav = std::env::temp_dir().join(format!("play198x-sidplayfp-{}.wav", std::process::id()));

    let status = Command::new(sidplayfp)
        .args(["--sidlite", "-q", "-t2", "-f48000", "-p32", "-m"])
        .arg(format!("-w{}", wav.display()))
        .arg(&tune)
        .status()
        .expect("sidplayfp should start");
    assert!(status.success(), "sidplayfp reference render failed");

    let bytes = std::fs::read(&tune).expect("differential SID should be readable");
    let mut ours =
        FramePump::new(SidPlayer::new(&bytes, 0, 48_000).expect("selected PSID should play"));
    let mut stereo = vec![0.0; FRAMES * 2];
    assert_eq!(ours.render(&mut stereo), FRAMES);
    let ours: Vec<f32> = stereo
        .as_chunks::<2>()
        .0
        .iter()
        .map(|frame| frame[0])
        .collect();
    let reference = wav_f32(&std::fs::read(&wav).expect("reference WAV should exist"));
    let _ = std::fs::remove_file(&wav);

    let count = ours.len().min(reference.len());
    assert!(count >= 95_000, "both players should render two seconds");
    let ours_rms = rms(&ours[..count]);
    let reference_rms = rms(&reference[..count]);
    let sample_correlation = normalised_correlation(&ours[..count], &reference[..count]);
    let ours_envelope = rms_windows(&ours[..count], 960);
    let reference_envelope = rms_windows(&reference[..count], 960);
    let envelope_correlation = pearson(&ours_envelope, &reference_envelope);
    println!(
        "SID differential: samples={count} ours_rms={ours_rms:.4} reference_rms={reference_rms:.4} sample_correlation={sample_correlation:.4} envelope_correlation={envelope_correlation:.4}"
    );
    assert!(
        ours_rms > 0.001 && reference_rms > 0.001,
        "both renders must be audible"
    );
    assert!(
        envelope_correlation >= 0.80,
        "render lost the reference tune's note/timing envelope: correlation {envelope_correlation:.4}"
    );
}

fn wav_f32(bytes: &[u8]) -> Vec<f32> {
    let at = bytes
        .windows(4)
        .position(|window| window == b"data")
        .expect("WAV data chunk")
        + 8;
    bytes[at..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

fn normalised_correlation(a: &[f32], b: &[f32]) -> f32 {
    let dot = a.iter().zip(b).map(|(a, b)| a * b).sum::<f32>();
    dot / (a.iter().map(|x| x * x).sum::<f32>() * b.iter().map(|x| x * x).sum::<f32>()).sqrt()
}

fn rms_windows(samples: &[f32], width: usize) -> Vec<f32> {
    samples.chunks_exact(width).map(rms).collect()
}

fn pearson(a: &[f32], b: &[f32]) -> f32 {
    let count = a.len().min(b.len());
    let mean_a = a[..count].iter().sum::<f32>() / count as f32;
    let mean_b = b[..count].iter().sum::<f32>() / count as f32;
    let mut dot = 0.0;
    let mut aa = 0.0;
    let mut bb = 0.0;
    for (&a, &b) in a[..count].iter().zip(&b[..count]) {
        let a = a - mean_a;
        let b = b - mean_b;
        dot += a * b;
        aa += a * a;
        bb += b * b;
    }
    dot / (aa * bb).sqrt()
}
