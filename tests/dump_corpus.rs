//! Helper that dumps lewtoff/oracle ogg for a corpus file (manually run).
#![cfg(feature = "oracle")]

use std::io::Write;
use std::process::{Command, Stdio};

fn ffmpeg_decode(path: &str, rate: u32, ch: u16) -> Vec<i16> {
    let out = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            path,
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

#[test]
#[ignore = "manual: dumps a corpus file's lw + oracle ogg to /tmp/"]
fn dump_corpus_one() {
    let name = std::env::var("CORPUS").unwrap_or_else(|_| "snd_ui_input_confirm.wav".into());
    let path = format!("sounds/{name}");
    let samples = ffmpeg_decode(&path, 44100, 2);
    let oracle = oracle_encode(&samples, 44100, 2);
    let serial = u32::from_le_bytes(oracle[14..18].try_into().unwrap());
    // extract vendor from oracle to match
    let vendor: &[u8] = b"Xiph.Org libVorbis I 20200704 (Reducing Environment)";
    let encoder_tag: &[u8] = b"";
    let lw = lewtoff::encode_with_serial_and_meta(
        &samples,
        lewtoff::SampleRate::Hz44100,
        lewtoff::Channels::Stereo,
        serial,
        Some(vendor),
        Some(encoder_tag),
    );
    std::fs::write("/tmp/corpus_oracle.ogg", &oracle).unwrap();
    std::fs::write("/tmp/corpus_lewtoff.ogg", &lw).unwrap();
    eprintln!(
        "oracle: {} bytes\nlewtoff: {} bytes",
        oracle.len(),
        lw.len()
    );
}
