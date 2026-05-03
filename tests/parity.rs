//! Byte-parity tests: lewtoff vs ffmpeg -c:a libvorbis -q:a 5.
//!
//! Gated behind the `oracle` feature so contributors without ffmpeg can still
//! run the rest of the suite. Run with:
//!   cargo nextest run --features oracle parity_
//!
//! All tests are marked `#[ignore]` because parity is not yet achieved —
//! remove `#[ignore]` once the divergences are fixed.

#![cfg(feature = "oracle")]

use lewtoff::{Channels, SampleRate};
use std::io::Write;
use std::process::{Command, Stdio};

/// Encode using our own statically-linked libvorbis (tools/oracle-encoder).
/// Pinned compile flags (-O0 -ffp-contract=off) make this deterministic;
/// CI / dev hosts produce identical bytes given identical input.
fn oracle_encode_q5(samples: &[i16], rate: u32, channels: u16) -> Vec<u8> {
    let raw: Vec<u8> = samples.iter().flat_map(|&s| s.to_le_bytes()).collect();
    let bin = std::env::current_dir()
        .unwrap()
        .join("tools/oracle-encoder/oracle-encoder");
    let mut child = Command::new(&bin)
        .arg(rate.to_string())
        .arg(channels.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn oracle-encoder at {}: {e}", bin.display()));
    child.stdin.take().unwrap().write_all(&raw).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "oracle-encoder exited non-zero");
    out.stdout
}

fn assert_parity_oracle(samples: &[i16], rate: SampleRate, channels: Channels) {
    let rate_hz: u32 = match rate {
        SampleRate::Hz44100 => 44100,
        SampleRate::Hz48000 => 48000,
    };
    let ch: u16 = match channels {
        Channels::Mono => 1,
        Channels::Stereo => 2,
    };
    let oracle_bytes = oracle_encode_q5(samples, rate_hz, ch);
    let serial = extract_serial(&oracle_bytes);
    let (vendor, encoder_tag) = extract_comment_strings(&oracle_bytes);
    let lewtoff_bytes = lewtoff::encode_with_serial_and_meta(
        samples,
        rate,
        channels,
        serial,
        Some(&vendor),
        Some(&encoder_tag),
    );
    if lewtoff_bytes != oracle_bytes {
        let div = first_diff(&lewtoff_bytes, &oracle_bytes);
        let lw_end = (div + 16).min(lewtoff_bytes.len());
        let or_end = (div + 16).min(oracle_bytes.len());
        let start = div.saturating_sub(8);
        panic!(
            "ORACLE parity diverged at byte {div}\n  lewtoff len: {}\n  oracle  len: {}\n  lewtoff ctx: {:02x?}\n  oracle  ctx: {:02x?}",
            lewtoff_bytes.len(),
            oracle_bytes.len(),
            &lewtoff_bytes[start..lw_end],
            &oracle_bytes[start.min(oracle_bytes.len())..or_end],
        );
    }
}

fn ffmpeg_encode_q5(samples: &[i16], rate: u32, channels: u16) -> Vec<u8> {
    let raw: Vec<u8> = samples.iter().flat_map(|&s| s.to_le_bytes()).collect();

    let mut child = Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "s16le",
            "-ar",
            &rate.to_string(),
            "-ac",
            &channels.to_string(),
            "-i",
            "pipe:0",
            "-c:a",
            "libvorbis",
            "-q:a",
            "5",
            "-f",
            "ogg",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn ffmpeg — is it on PATH with libvorbis?");

    child.stdin.take().unwrap().write_all(&raw).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "ffmpeg exited with non-zero status"
    );
    output.stdout
}

fn extract_serial(ogg_bytes: &[u8]) -> u32 {
    assert!(
        ogg_bytes.len() >= 18,
        "ogg output too short to contain serial"
    );
    u32::from_le_bytes(ogg_bytes[14..18].try_into().unwrap())
}

/// Extract the vendor string and first user comment (encoder tag) from a
/// Vorbis comment header packet embedded in the given OGG bitstream.
///
/// The comment header is the second packet in the stream (second OGG page,
/// first segment).  Returns (vendor, encoder_tag) as owned Vec<u8>.
fn extract_comment_strings(ogg_bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
    assert!(ogg_bytes.len() >= 58, "OGG output too short");
    let page0_len = {
        let page_segs = ogg_bytes[26] as usize;
        let seg_table = &ogg_bytes[27..27 + page_segs];
        let data_len: usize = seg_table.iter().map(|&s| s as usize).sum();
        27 + page_segs + data_len
    };
    let page1 = &ogg_bytes[page0_len..];
    assert!(page1.len() >= 27, "page1 too short");
    let page1_segs = page1[26] as usize;
    assert!(page1.len() >= 27 + page1_segs, "page1 seg table too short");
    let first_pkt_len = page1[27] as usize;
    let pkt = &page1[27 + page1_segs..27 + page1_segs + first_pkt_len];

    let mut off = 7usize;
    let vlen = u32::from_le_bytes(pkt[off..off + 4].try_into().unwrap()) as usize;
    off += 4;
    let vendor = pkt[off..off + vlen].to_vec();
    off += vlen;
    let count = u32::from_le_bytes(pkt[off..off + 4].try_into().unwrap()) as usize;
    off += 4;
    let encoder_tag = if count > 0 {
        let clen = u32::from_le_bytes(pkt[off..off + 4].try_into().unwrap()) as usize;
        off += 4;
        pkt[off..off + clen].to_vec()
    } else {
        Vec::new()
    };
    (vendor, encoder_tag)
}

fn first_diff(a: &[u8], b: &[u8]) -> usize {
    let common = a.len().min(b.len());
    for i in 0..common {
        if a[i] != b[i] {
            return i;
        }
    }
    common
}

fn assert_parity(samples: &[i16], rate: SampleRate, channels: Channels) {
    let rate_hz: u32 = match rate {
        SampleRate::Hz44100 => 44100,
        SampleRate::Hz48000 => 48000,
    };
    let ch: u16 = match channels {
        Channels::Mono => 1,
        Channels::Stereo => 2,
    };

    let ffmpeg_bytes = ffmpeg_encode_q5(samples, rate_hz, ch);
    let serial = extract_serial(&ffmpeg_bytes);
    let (vendor, encoder_tag) = extract_comment_strings(&ffmpeg_bytes);
    let lewtoff_bytes = lewtoff::encode_with_serial_and_meta(
        samples,
        rate,
        channels,
        serial,
        Some(&vendor),
        Some(&encoder_tag),
    );

    if lewtoff_bytes != ffmpeg_bytes {
        let div = first_diff(&lewtoff_bytes, &ffmpeg_bytes);
        let lw_start = div.saturating_sub(8);
        let lw_end = (div + 16).min(lewtoff_bytes.len());
        let ff_end = (div + 16).min(ffmpeg_bytes.len());
        let ff_start = div.saturating_sub(8).min(ffmpeg_bytes.len());
        panic!(
            "parity diverged at byte {div}\n  lewtoff len: {}\n  ffmpeg  len: {}\n  lewtoff ctx: {:02x?}\n  ffmpeg  ctx: {:02x?}",
            lewtoff_bytes.len(),
            ffmpeg_bytes.len(),
            &lewtoff_bytes[lw_start..lw_end],
            &ffmpeg_bytes[ff_start..ff_end],
        );
    }
}

fn make_sine_mono(rate: u32, freq: f32, duration_secs: f32) -> Vec<i16> {
    let n = (rate as f32 * duration_secs) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            (f32::sin(2.0 * std::f32::consts::PI * freq * t) * 16384.0) as i16
        })
        .collect()
}

#[test]
fn parity_silence_mono44() {
    assert_parity(&vec![0i16; 44100], SampleRate::Hz44100, Channels::Mono);
}

#[test]
fn parity_silence_stereo44() {
    let stereo: Vec<i16> = vec![0i16; 44100 * 2];
    assert_parity(&stereo, SampleRate::Hz44100, Channels::Stereo);
}

#[test]
fn parity_silence_mono48() {
    assert_parity(&vec![0i16; 48000], SampleRate::Hz48000, Channels::Mono);
}

#[test]
fn parity_silence_stereo48() {
    let stereo: Vec<i16> = vec![0i16; 48000 * 2];
    assert_parity(&stereo, SampleRate::Hz48000, Channels::Stereo);
}

#[test]
fn parity_sine_440_mono44() {
    let samples = make_sine_mono(44100, 440.0, 1.0);
    assert_parity(&samples, SampleRate::Hz44100, Channels::Mono);
}

// --- Oracle-based parity (own libvorbis build, deterministic flags) ---

#[test]
fn oracle_parity_silence_mono44() {
    assert_parity_oracle(&vec![0i16; 44100], SampleRate::Hz44100, Channels::Mono);
}

#[test]
fn oracle_parity_silence_stereo44() {
    let stereo: Vec<i16> = vec![0i16; 44100 * 2];
    assert_parity_oracle(&stereo, SampleRate::Hz44100, Channels::Stereo);
}

#[test]
fn oracle_parity_sine_440_mono44() {
    let samples = make_sine_mono(44100, 440.0, 1.0);
    assert_parity_oracle(&samples, SampleRate::Hz44100, Channels::Mono);
}

#[test]
#[ignore = "ffmpeg-built libvorbis differs from oracle (FMA, optimization). Use oracle_parity_ramp_stereo44 instead."]
fn parity_ramp_stereo44() {
    let n = 44100usize;
    let samples: Vec<i16> = (0..n * 2)
        .map(|i| ((i % 65536) as i32 - 32768) as i16)
        .collect();
    assert_parity(&samples, SampleRate::Hz44100, Channels::Stereo);
}

#[test]
#[ignore = "stereo ramp parity not yet achieved — likely stereo coupling / ramp-specific psy"]
fn oracle_parity_ramp_stereo44() {
    let n = 44100usize;
    let samples: Vec<i16> = (0..n * 2)
        .map(|i| ((i % 65536) as i32 - 32768) as i16)
        .collect();
    assert_parity_oracle(&samples, SampleRate::Hz44100, Channels::Stereo);
}

#[test]
#[ignore = "manual dump for parity-diff analysis — ramp stereo"]
fn parity_dump_ramp_files() {
    let n = 44100usize;
    let samples: Vec<i16> = (0..n * 2)
        .map(|i| ((i % 65536) as i32 - 32768) as i16)
        .collect();
    let oracle_bytes = oracle_encode_q5(&samples, 44100, 2);
    let serial = extract_serial(&oracle_bytes);
    let (vendor, encoder_tag) = extract_comment_strings(&oracle_bytes);
    let lewtoff_bytes = lewtoff::encode_with_serial_and_meta(
        &samples,
        SampleRate::Hz44100,
        Channels::Stereo,
        serial,
        Some(&vendor),
        Some(&encoder_tag),
    );
    std::fs::write("/tmp/oracle_ramp.ogg", &oracle_bytes).unwrap();
    std::fs::write("/tmp/lw_ramp.ogg", &lewtoff_bytes).unwrap();
    eprintln!(
        "Wrote oracle_ramp.ogg ({} bytes) and lw_ramp.ogg ({} bytes)",
        oracle_bytes.len(),
        lewtoff_bytes.len()
    );
}

#[test]
#[ignore = "manual dump for parity-diff analysis"]
fn parity_dump_files() {
    let samples = vec![0i16; 44100];
    let ffmpeg_bytes = ffmpeg_encode_q5(&samples, 44100, 1);
    let serial = extract_serial(&ffmpeg_bytes);
    let lewtoff_bytes =
        lewtoff::encode_with_serial(&samples, SampleRate::Hz44100, Channels::Mono, serial);
    std::fs::write("/tmp/ff_parity.ogg", &ffmpeg_bytes).unwrap();
    std::fs::write("/tmp/lw_parity.ogg", &lewtoff_bytes).unwrap();
    eprintln!(
        "Wrote ff_parity.ogg ({} bytes) and lw_parity.ogg ({} bytes)",
        ffmpeg_bytes.len(),
        lewtoff_bytes.len()
    );
}

#[test]
#[ignore = "manual dump for parity-diff analysis — sine wave"]
fn parity_dump_sine_files() {
    let samples = make_sine_mono(44100, 440.0, 1.0);
    let ffmpeg_bytes = ffmpeg_encode_q5(&samples, 44100, 1);
    let serial = extract_serial(&ffmpeg_bytes);
    let (vendor, encoder_tag) = extract_comment_strings(&ffmpeg_bytes);
    let lewtoff_bytes = lewtoff::encode_with_serial_and_meta(
        &samples,
        SampleRate::Hz44100,
        Channels::Mono,
        serial,
        Some(&vendor),
        Some(&encoder_tag),
    );
    std::fs::write("/tmp/ff_sine.ogg", &ffmpeg_bytes).unwrap();
    std::fs::write("/tmp/lw_sine.ogg", &lewtoff_bytes).unwrap();
    eprintln!(
        "Wrote ff_sine.ogg ({} bytes) and lw_sine.ogg ({} bytes)",
        ffmpeg_bytes.len(),
        lewtoff_bytes.len()
    );
}
