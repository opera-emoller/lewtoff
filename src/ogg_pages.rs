use std::io::Cursor;

use ogg::writing::{PacketWriteEndInfo, PacketWriter};

/// libogg `nfill` default — pages flush when the accumulated body in the
/// current page would exceed this size after the just-added packet, AND at
/// least 4 packets have been completed (libogg `ogg_stream_flush_i`).
const PAGE_NFILL: usize = 4096;

pub struct OggStreamWriter {
    writer: PacketWriter<'static, Cursor<Vec<u8>>>,
    serial: u32,
    /// Bytes of packet body accumulated in the current page (not yet flushed).
    page_body_bytes: usize,
    /// Packets fully completed in the current page.
    page_packets_done: usize,
}

impl OggStreamWriter {
    pub fn new(serial: u32) -> Self {
        Self {
            writer: PacketWriter::new(Cursor::new(Vec::new())),
            serial,
            page_body_bytes: 0,
            page_packets_done: 0,
        }
    }

    pub fn write_packet(&mut self, bytes: &[u8], granule: u64, eos: bool, force_flush: bool) {
        // Decide whether this packet should end its page. Mirror libogg's
        // ogg_stream_flush_i rule: after a packet completes, if accumulated
        // body > nfill AND at least 4 packets are done, flush.
        let new_body = self.page_body_bytes + bytes.len();
        let new_packets = self.page_packets_done + 1;
        let auto_flush = new_body > PAGE_NFILL && new_packets >= 4;

        let end_info = if eos {
            PacketWriteEndInfo::EndStream
        } else if force_flush || auto_flush {
            PacketWriteEndInfo::EndPage
        } else {
            PacketWriteEndInfo::NormalPacket
        };
        self.writer
            .write_packet(bytes.to_vec(), self.serial, end_info, granule)
            .expect("ogg write failed");

        if matches!(
            end_info,
            PacketWriteEndInfo::EndPage | PacketWriteEndInfo::EndStream
        ) {
            self.page_body_bytes = 0;
            self.page_packets_done = 0;
        } else {
            self.page_body_bytes = new_body;
            self.page_packets_done = new_packets;
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.writer.into_inner().into_inner()
    }
}
