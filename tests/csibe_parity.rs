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

/// Quietly check one file — returns Ok(()) on byte-identical, Err(reason) otherwise.
fn check_one(rel: &str, channels: lewtoff::Channels) -> Result<(), String> {
    let path: PathBuf = [CSIBE_ROOT, rel].iter().collect();
    if !path.exists() {
        return Err(format!("missing: {}", path.display()));
    }
    let ch_n: u16 = match channels {
        lewtoff::Channels::Mono => 1,
        lewtoff::Channels::Stereo => 2,
    };
    let samples = ffmpeg_decode(&path, 44100, ch_n);
    if samples.is_empty() {
        return Err("empty samples".into());
    }
    let oracle = oracle_encode(&samples, 44100, ch_n);
    if oracle.len() < 18 {
        return Err(format!("oracle output too short: {} bytes", oracle.len()));
    }
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
        Ok(())
    } else {
        let mut first_diff = lw.len().min(oracle.len());
        for i in 0..lw.len().min(oracle.len()) {
            if lw[i] != oracle[i] {
                first_diff = i;
                break;
            }
        }
        Err(format!(
            "lw={} or={} delta={} first_diff={}",
            lw.len(),
            oracle.len(),
            lw.len() as i64 - oracle.len() as i64,
            first_diff
        ))
    }
}

/// Walk the csibe corpus and run `take_per_category` files from each category.
/// Set CSIBE_TAKE=N to control sample size per category (default 5).
#[test]
#[ignore = "manual: large sweep over /Users/emoller/Downloads/csibe_raw"]
fn csibe_parity_sweep_stereo44() {
    let take_per_category: usize = std::env::var("CSIBE_TAKE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);

    let root = std::path::Path::new(CSIBE_ROOT);
    let mut categories: Vec<std::path::PathBuf> = std::fs::read_dir(root)
        .expect("csibe_raw dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    categories.sort();

    let mut total = 0usize;
    let mut passed = 0usize;
    let mut by_category: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    let mut failures: Vec<(String, String)> = Vec::new();

    for cat in &categories {
        let cat_name = cat.file_name().unwrap().to_string_lossy().to_string();
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(cat)
            .expect("cat dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "wav"))
            .collect();
        files.sort();
        for path in files.iter().take(take_per_category) {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            total += 1;
            match check_one(&rel, lewtoff::Channels::Stereo) {
                Ok(()) => {
                    passed += 1;
                    by_category.entry(cat_name.clone()).or_default().0 += 1;
                }
                Err(why) => {
                    failures.push((rel.clone(), why));
                    by_category.entry(cat_name.clone()).or_default().1 += 1;
                }
            }
        }
    }

    eprintln!("\n=== csibe sweep (stereo, 44.1k) ===");
    eprintln!("Overall: {}/{} byte-identical", passed, total);
    eprintln!("By category:");
    for (cat, &(pass, fail)) in &by_category {
        eprintln!("  {:25} {:3} pass, {:3} fail", cat, pass, fail);
    }
    if !failures.is_empty() {
        eprintln!("\nFailures (showing up to 50):");
        for (rel, why) in failures.iter().take(50) {
            eprintln!("  {} — {}", rel, why);
        }
    }
}
