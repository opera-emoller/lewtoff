use ogg::PacketReader;
use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Combo {
    name: &'static str,
    rate: &'static str,
    channels: &'static str,
}

const COMBOS: &[Combo] = &[
    Combo {
        name: "mono44",
        rate: "44100",
        channels: "1",
    },
    Combo {
        name: "mono48",
        rate: "48000",
        channels: "1",
    },
    Combo {
        name: "stereo44",
        rate: "44100",
        channels: "2",
    },
    Combo {
        name: "stereo48",
        rate: "48000",
        channels: "2",
    },
];

struct FfmpegOutput {
    packets: Vec<Vec<u8>>,
    serial: u32,
    header_pages_bytes: Vec<u8>,
}

fn ogg_page_size(data: &[u8], offset: usize) -> usize {
    assert_eq!(
        &data[offset..offset + 4],
        b"OggS",
        "expected OggS at offset {offset}"
    );
    let nseg = data[offset + 26] as usize;
    let lacing_size: usize = data[offset + 27..offset + 27 + nseg]
        .iter()
        .map(|&b| b as usize)
        .sum();
    27 + nseg + lacing_size
}

fn run_ffmpeg(rate: &str, channels: &str) -> FfmpegOutput {
    let n_channels: usize = channels.parse().unwrap();
    let silence: Vec<u8> = vec![0u8; 1024 * 2 * n_channels];

    let mut child = Command::new("ffmpeg")
        .args([
            "-f",
            "s16le",
            "-ar",
            rate,
            "-ac",
            channels,
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

    let raw = out.stdout;

    let serial = u32::from_le_bytes(raw[14..18].try_into().unwrap());

    let page0_size = ogg_page_size(&raw, 0);
    let page1_size = ogg_page_size(&raw, page0_size);
    let header_end = page0_size + page1_size;
    let header_pages_bytes = raw[..header_end].to_vec();

    let cursor = Cursor::new(raw);
    let mut reader = PacketReader::new(cursor);

    let mut packets = Vec::new();
    for _ in 0..3 {
        let pck = reader
            .read_packet()
            .expect("ogg read error")
            .expect("ogg stream ended before 3 header packets");
        packets.push(pck.data);
    }

    FfmpegOutput {
        packets,
        serial,
        header_pages_bytes,
    }
}

fn main() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let vec_dir = repo_root.join("tests").join("vectors").join("headers");
    std::fs::create_dir_all(&vec_dir).expect("failed to create vectors/headers dir");

    let ogg_dir = repo_root.join("tests").join("vectors").join("ogg");
    std::fs::create_dir_all(&ogg_dir).expect("failed to create vectors/ogg dir");

    for combo in COMBOS {
        println!("Generating combo: {}", combo.name);
        let result = run_ffmpeg(combo.rate, combo.channels);
        let packets = &result.packets;

        let id_path = vec_dir.join(format!("id_{}.bin", combo.name));
        let comment_path = vec_dir.join(format!("comment_{}.bin", combo.name));
        let serial_path = ogg_dir.join(format!("serial_{}.bin", combo.name));
        let headers_ogg_path = ogg_dir.join(format!("headers_{}.ogg", combo.name));

        assert_eq!(packets[0][0], 0x01, "packet 0 should start with 0x01");
        assert_eq!(&packets[0][1..7], b"vorbis", "packet 0 sync check");
        assert_eq!(packets[1][0], 0x03, "packet 1 should start with 0x03");
        assert_eq!(&packets[1][1..7], b"vorbis", "packet 1 sync check");
        assert_eq!(packets[2][0], 0x05, "packet 2 should start with 0x05");
        assert_eq!(&packets[2][1..7], b"vorbis", "packet 2 sync check");

        std::fs::write(&id_path, &packets[0]).expect("failed to write id header");
        std::fs::write(&comment_path, &packets[1]).expect("failed to write comment header");
        std::fs::write(&serial_path, result.serial.to_le_bytes()).expect("failed to write serial");
        std::fs::write(&headers_ogg_path, &result.header_pages_bytes)
            .expect("failed to write headers ogg");

        println!(
            "  id:      {} bytes -> {}",
            packets[0].len(),
            id_path.display()
        );
        println!(
            "  comment: {} bytes -> {}",
            packets[1].len(),
            comment_path.display()
        );
        println!(
            "  serial:  0x{:08x} -> {}",
            result.serial,
            serial_path.display()
        );
        println!(
            "  headers ogg: {} bytes -> {}",
            result.header_pages_bytes.len(),
            headers_ogg_path.display()
        );

        println!("  id header bytes: {:02x?}", &packets[0]);
        println!("  comment header bytes: {:02x?}", &packets[1]);
    }

    println!("Done.");
}
