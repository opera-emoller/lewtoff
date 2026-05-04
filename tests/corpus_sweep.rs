//! Walk `<repo_root>/corpus/` and assert byte-identical parity against
//! libvorbis 1.3.7 Q5 for every audio file found. Skipped if the directory
//! doesn't exist (CI-friendly; contributors symlink their own corpus).
//!
//! Inputs are decoded via ffmpeg to s16le 44.1kHz stereo, encoded by both
//! lewtoff and the oracle-encoder, and compared byte-for-byte.

#![cfg(feature = "oracle")]

use std::path::{Path, PathBuf};
use std::process::Command;

const CORPUS_DIR: &str = "corpus";
const AUDIO_EXTENSIONS: &[&str] = &["wav", "mp3", "ogg", "flac", "m4a", "aif", "aiff"];

fn ffmpeg_decode(path: &Path, rate: u32, ch: u16) -> Vec<i16> {
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
        .unwrap_or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                panic!(
                    "ffmpeg not on PATH — install it (e.g. `brew install ffmpeg` on macOS, \
                     `apt install ffmpeg` on Debian/Ubuntu) and re-run"
                );
            }
            panic!("failed to invoke ffmpeg on {}: {e}", path.display());
        });
    out.stdout
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn oracle_encode(samples: &[i16], rate: u32, ch: u16) -> Vec<u8> {
    let raw: Vec<u8> = samples.iter().flat_map(|&s| s.to_le_bytes()).collect();
    use std::io::Write;
    use std::process::Stdio;
    let bin = "./tools/oracle-encoder/oracle-encoder";
    let mut child = Command::new(bin)
        .args([&rate.to_string(), &ch.to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                panic!("{bin} not found — build it first with `./tools/oracle-encoder/build.sh`");
            }
            panic!("failed to spawn {bin}: {e}");
        });
    child.stdin.take().unwrap().write_all(&raw).unwrap();
    child.wait_with_output().unwrap().stdout
}

fn check_one(path: &Path) -> Result<(), String> {
    let samples = ffmpeg_decode(path, 44100, 2);
    if samples.is_empty() {
        return Err("empty samples".into());
    }
    let oracle = oracle_encode(&samples, 44100, 2);
    if oracle.len() < 18 {
        return Err(format!("oracle too short: {} bytes", oracle.len()));
    }
    let serial = u32::from_le_bytes(oracle[14..18].try_into().unwrap());
    let vendor: &[u8] = b"Xiph.Org libVorbis I 20200704 (Reducing Environment)";
    let lw = lewtoff::encode_with_serial_and_meta(
        &samples,
        lewtoff::SampleRate::Hz44100,
        lewtoff::Channels::Stereo,
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

fn walk_audio_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn recurse(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                recurse(&path, out);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if AUDIO_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(ext)) {
                    out.push(path);
                }
            }
        }
    }
    recurse(root, &mut out);
    out
}

/// Walk `<repo_root>/corpus/` recursively and assert parity for every audio
/// file found. Skips silently if the directory is absent (typical CI case).
///
/// Set `CORPUS_LIMIT=N` to test only the first N files (sorted) — useful for
/// quick smoke runs. Failures are reported per category and the test panics
/// at the end with a summary if any file diverged.
#[test]
#[ignore = "manual sweep over <repo_root>/corpus/ (gitignored, large)"]
fn corpus_parity_sweep() {
    let root = std::env::current_dir().unwrap().join(CORPUS_DIR);
    if !root.exists() {
        eprintln!("no {} directory; skipping", root.display());
        return;
    }

    let limit = std::env::var("CORPUS_LIMIT")
        .ok()
        .and_then(|s| s.parse::<usize>().ok());

    let mut files = walk_audio_files(&root);
    files.sort();
    if let Some(n) = limit {
        files.truncate(n);
    }

    if files.is_empty() {
        eprintln!("no audio files under {}; skipping", root.display());
        return;
    }

    let mut total = 0usize;
    let mut passed = 0usize;
    let mut by_category: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    let mut failures: Vec<(PathBuf, String)> = Vec::new();

    for path in &files {
        let rel = path.strip_prefix(&root).unwrap();
        // First path component as "category", or "(root)" for files directly
        // under corpus/.
        let cat = if rel.components().count() > 1 {
            rel.components()
                .next()
                .unwrap()
                .as_os_str()
                .to_string_lossy()
                .into_owned()
        } else {
            "(root)".to_string()
        };
        total += 1;
        match check_one(path) {
            Ok(()) => {
                passed += 1;
                by_category.entry(cat).or_default().0 += 1;
            }
            Err(why) => {
                by_category.entry(cat).or_default().1 += 1;
                failures.push((rel.to_path_buf(), why));
            }
        }
    }

    eprintln!("\n=== corpus sweep (stereo, 44.1k) ===");
    eprintln!("Overall: {}/{} byte-identical", passed, total);
    eprintln!("By category:");
    for (cat, &(pass, fail)) in &by_category {
        eprintln!("  {:30} {:5} pass, {:5} fail", cat, pass, fail);
    }
    if !failures.is_empty() {
        eprintln!("\nFailures (showing up to 50):");
        for (rel, why) in failures.iter().take(50) {
            eprintln!("  {} — {}", rel.display(), why);
        }
        panic!(
            "{}/{} files diverged (see stderr for details)",
            failures.len(),
            total
        );
    }
}
