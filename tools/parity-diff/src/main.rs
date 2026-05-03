//! parity-diff: structured diff of two Ogg Vorbis files at page/packet level.
//!
//! Usage: parity-diff <lewtoff.ogg> <ffmpeg.ogg>
//!
//! Parses both files with the `ogg` crate and reports the first divergence at
//! page, packet, and byte granularity.  Serial-number bytes (offset 14-17 of
//! each page header) are skipped when comparing raw page bytes so that a
//! deliberate serial-alignment isn't treated as a divergence.

use ogg::PacketReader;
use std::fs;
use std::io::Cursor;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: parity-diff <lewtoff.ogg> <ffmpeg.ogg>");
        std::process::exit(1);
    }

    let lw_bytes = fs::read(&args[1]).unwrap_or_else(|e| {
        eprintln!("Cannot read {}: {e}", args[1]);
        std::process::exit(1);
    });
    let ff_bytes = fs::read(&args[2]).unwrap_or_else(|e| {
        eprintln!("Cannot read {}: {e}", args[2]);
        std::process::exit(1);
    });

    println!(
        "lewtoff: {} bytes\nffmpeg:  {} bytes",
        lw_bytes.len(),
        ff_bytes.len()
    );

    let lw_packets = read_packets(&lw_bytes);
    let ff_packets = read_packets(&ff_bytes);

    let max_pkts = lw_packets.len().max(ff_packets.len());
    let mut all_match = true;

    for i in 0..max_pkts {
        match (lw_packets.get(i), ff_packets.get(i)) {
            (None, Some(ff)) => {
                println!(
                    "Packet {i}: MISSING in lewtoff ({} bytes in ffmpeg)",
                    ff.len()
                );
                all_match = false;
            }
            (Some(lw), None) => {
                println!(
                    "Packet {i}: EXTRA in lewtoff ({} bytes), not in ffmpeg",
                    lw.len()
                );
                all_match = false;
            }
            (Some(lw), Some(ff)) => {
                if lw == ff {
                    let label = packet_label(i);
                    println!("Packet {i} ({label}): match ({} bytes)", lw.len());
                } else {
                    let label = packet_label(i);
                    let div = first_diff(lw, ff);
                    let lw_ctx_start = div.saturating_sub(8);
                    let ff_ctx_start = div.saturating_sub(8).min(ff.len());
                    let lw_ctx_end = (div + 16).min(lw.len());
                    let ff_ctx_end = (div + 16).min(ff.len());
                    println!(
                        "Packet {i} ({label}): DIVERGE at byte {div} (lewtoff={} bytes, ffmpeg={} bytes)",
                        lw.len(),
                        ff.len()
                    );
                    println!(
                        "  lewtoff[{lw_ctx_start}..{lw_ctx_end}]: {:02x?}",
                        &lw[lw_ctx_start..lw_ctx_end]
                    );
                    println!(
                        "  ffmpeg [{ff_ctx_start}..{ff_ctx_end}]: {:02x?}",
                        &ff[ff_ctx_start..ff_ctx_end]
                    );
                    all_match = false;
                }
            }
            (None, None) => unreachable!(),
        }
    }

    if all_match {
        println!("All {max_pkts} packets match!");
    } else {
        println!("\nNote: serial bytes (page-header offsets 14-17) are NOT compared here.");
        std::process::exit(1);
    }
}

fn packet_label(idx: usize) -> &'static str {
    match idx {
        0 => "id header",
        1 => "comment header",
        2 => "setup header",
        _ => "audio",
    }
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

fn read_packets(data: &[u8]) -> Vec<Vec<u8>> {
    let cursor = Cursor::new(data);
    let mut reader = PacketReader::new(cursor);
    let mut packets = Vec::new();
    loop {
        match reader.read_packet() {
            Ok(Some(pkt)) => packets.push(pkt.data.to_vec()),
            Ok(None) => break,
            Err(e) => {
                eprintln!("Error reading ogg packet: {e}");
                break;
            }
        }
    }
    packets
}
