use ogg::PacketReader;
use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn main() {
    // 1024 samples of silence at 44100 Hz mono s16le
    let silence: Vec<u8> = vec![0u8; 1024 * 2];

    let mut child = Command::new("ffmpeg")
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
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn ffmpeg");

    {
        let stdin = child.stdin.as_mut().expect("failed to open stdin");
        stdin.write_all(&silence).expect("failed to write silence");
    }

    let out = child.wait_with_output().expect("failed to wait on ffmpeg");
    if !out.status.success() {
        eprintln!("ffmpeg failed with status: {}", out.status);
        std::process::exit(1);
    }

    let buf = out.stdout;
    let cursor = Cursor::new(buf);
    let mut reader = PacketReader::new(cursor);

    let mut packet_idx = 0usize;
    let setup_packet = loop {
        let pck = reader
            .read_packet()
            .expect("ogg read error")
            .expect("ogg stream ended before setup header");
        if packet_idx == 2 {
            break pck;
        }
        packet_idx += 1;
    };

    let data = &setup_packet.data;

    assert_eq!(
        &data[0..7],
        b"\x05vorbis",
        "packet 2 does not start with 0x05 'vorbis' sync pattern"
    );

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let out_path = repo_root.join("src").join("setup_blob.bin");

    std::fs::write(&out_path, data).expect("failed to write setup_blob.bin");

    println!("OK: wrote {} bytes to src/setup_blob.bin", data.len());
}
