//! Parity test against a few hand-picked sounds from the CSIBE raw corpus.
//! Runs as mono 44.1kHz (the natural format of the source files) AND as
//! stereo 44.1kHz (ffmpeg will duplicate the channel) to exercise both paths.
#![cfg(feature = "oracle")]

use std::path::PathBuf;
use std::process::Command;

const CSIBE_ROOT: &str = "/Users/emoller/Downloads/csibe_raw";

const CSIBE_FILES: &[&str] = &[
    "ambient_noise_1/aasp_clearthroat01-01.wav",
    "baby_cry_2/baby_cry1_1.wav",
    "bell_3/bell1a_1.wav",
    "guitar_8/guitar1a_1.wav",
    "piano_11/piano1_1.wav",
    "traffic_13/traffic10_motorbike_10.wav",
];

fn ffmpeg_decode(path: &PathBuf, rate: u32, ch: u16) -> Vec<i16> {
    let out = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            path.to_str().unwrap(),
            "-f",
            "s16le",
            "-ac",
            &ch.to_string(),
            "-ar",
            &rate.to_string(),
            "-",
        ])
        .output()
        .unwrap();
    out.stdout
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn oracle_encode(samples: &[i16], rate: u32, ch: u16) -> Vec<u8> {
    let raw: Vec<u8> = samples.iter().flat_map(|&s| s.to_le_bytes()).collect();
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new("./tools/oracle-encoder/oracle-encoder")
        .args([&rate.to_string(), &ch.to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&raw).unwrap();
    child.wait_with_output().unwrap().stdout
}

fn run_one(rel: &str, channels: lewtoff::Channels) {
    let path: PathBuf = [CSIBE_ROOT, rel].iter().collect();
    if !path.exists() {
        panic!("missing csibe file: {}", path.display());
    }
    let ch_n: u16 = match channels {
        lewtoff::Channels::Mono => 1,
        lewtoff::Channels::Stereo => 2,
    };
    let samples = ffmpeg_decode(&path, 44100, ch_n);
    let oracle = oracle_encode(&samples, 44100, ch_n);
    let serial = u32::from_le_bytes(oracle[14..18].try_into().unwrap());
    let vendor: &[u8] = b"Xiph.Org libVorbis I 20200704 (Reducing Environment)";
    let lw = lewtoff::encode_with_serial_and_meta(
        &samples,
        lewtoff::SampleRate::Hz44100,
        channels,
        serial,
        Some(vendor),
        Some(b""),
    );
    if lw == oracle {
        eprintln!(
            "PASS  {rel} ({} ch, {} samples)",
            ch_n,
            samples.len() / ch_n as usize
        );
    } else {
        eprintln!(
            "FAIL  {rel} ({} ch): lw={} or={} delta={}",
            ch_n,
            lw.len(),
            oracle.len(),
            lw.len() as i64 - oracle.len() as i64
        );
        // find first diff
        for i in 0..lw.len().min(oracle.len()) {
            if lw[i] != oracle[i] {
                eprintln!("    first diff at byte {}", i);
                break;
            }
        }
        panic!("parity fail: {rel}");
    }
}

#[test]
#[ignore = "manual: requires /Users/emoller/Downloads/csibe_raw"]
fn csibe_parity_mono44() {
    let mut failures = Vec::new();
    for &rel in CSIBE_FILES {
        let result = std::panic::catch_unwind(|| run_one(rel, lewtoff::Channels::Mono));
        if result.is_err() {
            failures.push(rel);
        }
    }
    if !failures.is_empty() {
        panic!(
            "{}/{} csibe mono diverged: {:?}",
            failures.len(),
            CSIBE_FILES.len(),
            failures
        );
    }
}

#[test]
#[ignore = "manual: requires /Users/emoller/Downloads/csibe_raw"]
fn csibe_parity_stereo44() {
    let mut failures = Vec::new();
    for &rel in CSIBE_FILES {
        let result = std::panic::catch_unwind(|| run_one(rel, lewtoff::Channels::Stereo));
        if result.is_err() {
            failures.push(rel);
        }
    }
    if !failures.is_empty() {
        panic!(
            "{}/{} csibe stereo diverged: {:?}",
            failures.len(),
            CSIBE_FILES.len(),
            failures
        );
    }
}
