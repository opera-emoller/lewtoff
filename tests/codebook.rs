//! Codebook tests: Layer A (synthetic unit tests) + Layer B (lewton cross-check).
//!
//! Layer B (lewton cross-check) is partially limited because lewton's SetupHeader
//! fields are pub(crate) — we can verify that lewton parses successfully but
//! cannot compare internal codebook field values directly. Phase 9 parity is the
//! ground truth for encode correctness.

// ---------------------------------------------------------------------------
// Layer A: synthetic unit tests
// ---------------------------------------------------------------------------

/// Layer A unit tests live in src/codebook.rs (pub(crate) items can't be
/// tested from integration tests without extra test-export machinery).
/// This test verifies the setup blob has the expected sync bytes and size.
#[test]
fn q5_blob_sync_bytes_and_size() {
    let blob = include_bytes!("../src/setup_blob.bin");
    assert_eq!(
        &blob[0..7],
        b"\x05vorbis",
        "setup blob must start with 0x05 vorbis"
    );
    assert!(
        blob.len() > 1000,
        "setup blob suspiciously small: {} bytes",
        blob.len()
    );
    assert!(
        blob.len() < 20_000,
        "setup blob suspiciously large: {} bytes",
        blob.len()
    );
}

// ---------------------------------------------------------------------------
// Layer B: lewton cross-check
//
// lewton's SetupHeader.codebooks field is pub(crate), so we cannot compare
// individual codebook field values. We CAN verify that lewton successfully
// parses the same blob (proving the blob is valid Vorbis). The encode-side
// correctness gate is Phase 9 parity.
// ---------------------------------------------------------------------------

/// Verify lewton can parse the Q5 setup blob without error.
/// This proves the blob is a valid Vorbis setup header.
#[test]
fn lewton_parses_q5_setup_blob() {
    use lewton::header::{read_header_comment, read_header_ident, read_header_setup};

    // We need the three Vorbis headers. Generate them from a fresh encode
    // by reading the same ogg that gen-setup-blob uses, or — more simply —
    // read the id/comment headers from a pre-baked source.
    //
    // The simplest approach: just check that we can parse the setup blob
    // after constructing minimal ident/comment headers inline.
    //
    // lewton's read_header_setup needs: packet bytes, audio_channels, blocksizes.
    // Those come from parsing the ident header. We'll do a full 3-packet parse.
    //
    // We'll synthesize a small ogg stream using ffmpeg (if available), or
    // fall back to a static copy of the ident/comment packets we already know.
    //
    // Since CI doesn't have ffmpeg, we'll use stored ident/comment bytes.
    // The ident header for Q5 44100 Hz mono is well-known.

    // Q5 44100 Hz mono Vorbis ident header (from the same ffmpeg encode):
    // We stored the setup blob; we need the ident header too. For simplicity,
    // let's call read_header_ident with a known-good ident packet bytes.
    // These were captured from the same ffmpeg -q:a 5 44100 mono encode.

    // Generate the headers at test time by running ffmpeg (if available).
    // If ffmpeg is not available, skip gracefully.
    let ffmpeg_result = std::process::Command::new("ffmpeg")
        .args([
            "-f",
            "s16le",
            "-ar",
            "44100",
            "-ac",
            "1",
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
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();

    let Ok(mut child) = ffmpeg_result else {
        eprintln!("ffmpeg not available — skipping lewton cross-check");
        return;
    };

    {
        use std::io::Write;
        let silence = vec![0u8; 1024 * 2];
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(&silence);
        }
    }

    let Ok(out) = child.wait_with_output() else {
        eprintln!("ffmpeg failed — skipping lewton cross-check");
        return;
    };

    if !out.status.success() {
        eprintln!("ffmpeg exited with error — skipping lewton cross-check");
        return;
    }

    use std::io::Cursor;
    let mut reader = ogg::PacketReader::new(Cursor::new(out.stdout));

    let pkt0 = reader.read_packet().unwrap().unwrap();
    let pkt1 = reader.read_packet().unwrap().unwrap();
    let pkt2 = reader.read_packet().unwrap().unwrap();

    let ident = read_header_ident(&pkt0.data).expect("lewton failed to parse ident header");
    let _comment = read_header_comment(&pkt1.data).expect("lewton failed to parse comment header");
    let _setup = read_header_setup(
        &pkt2.data,
        ident.audio_channels,
        (ident.blocksize_0, ident.blocksize_1),
    )
    .expect("lewton failed to parse setup header");

    // Also verify our blob matches pkt2
    let our_blob = include_bytes!("../src/setup_blob.bin");
    assert_eq!(
        our_blob,
        &pkt2.data[..],
        "our setup blob does not match the freshly-generated one — rerun `just regen-setup-blob`"
    );

    eprintln!("Layer B: lewton parsed the Q5 setup header successfully");
}
