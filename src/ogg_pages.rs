use std::io::Cursor;

use ogg::writing::{PacketWriteEndInfo, PacketWriter};

pub struct OggStreamWriter {
    writer: PacketWriter<'static, Cursor<Vec<u8>>>,
    serial: u32,
}

impl OggStreamWriter {
    pub fn new(serial: u32) -> Self {
        Self {
            writer: PacketWriter::new(Cursor::new(Vec::new())),
            serial,
        }
    }

    pub fn write_packet(&mut self, bytes: &[u8], granule: u64, eos: bool, force_flush: bool) {
        let end_info = if eos {
            PacketWriteEndInfo::EndStream
        } else if force_flush {
            PacketWriteEndInfo::EndPage
        } else {
            PacketWriteEndInfo::NormalPacket
        };
        self.writer
            .write_packet(bytes.to_vec(), self.serial, end_info, granule)
            .expect("ogg write failed");
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.writer.into_inner().into_inner()
    }
}
