//! LSB-first bit writer, per Vorbis I §2.1.4.
//!
//! This is the only bit-level I/O the encoder does. Reads come later (Phase 7+
//! when we need to verify the setup-header round-trip via lewton); for now we
//! only emit.

#[derive(Default)]
pub(crate) struct BitWriter {
    bytes: Vec<u8>,
    /// Number of bits already written into the *last* byte of `bytes`. In
    /// `1..=8`. When `bytes` is empty, no bits have been written yet.
    bits_in_last: u8,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Total number of bits written so far.
    pub fn bit_len(&self) -> usize {
        if self.bytes.is_empty() {
            0
        } else {
            (self.bytes.len() - 1) * 8 + self.bits_in_last as usize
        }
    }

    /// Consume the writer and return the underlying bytes.
    /// The final byte is zero-padded in its unused high bits.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Write the low `bits` bits of `value`, LSB-first. `bits` must be `<= 32`.
    pub fn write(&mut self, value: u32, bits: u32) {
        debug_assert!(bits <= 32, "bits must be <= 32, got {bits}");
        if bits == 0 {
            return;
        }
        let mut value = if bits == 32 {
            value
        } else {
            value & ((1u32 << bits) - 1)
        };
        let mut bits_remaining = bits as u8;

        while bits_remaining > 0 {
            if self.bytes.is_empty() || self.bits_in_last == 8 {
                self.bytes.push(0);
                self.bits_in_last = 0;
            }

            let space = 8 - self.bits_in_last;
            let take = bits_remaining.min(space);

            let chunk = (value & ((1u32 << take) - 1)) as u8;
            let last = self.bytes.last_mut().expect("just pushed if needed");
            *last |= chunk << self.bits_in_last;

            self.bits_in_last += take;
            bits_remaining -= take;
            value >>= take;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_writer_has_zero_bit_len_and_no_bytes() {
        let w = BitWriter::new();
        assert_eq!(w.bit_len(), 0);
        assert_eq!(w.into_bytes(), Vec::<u8>::new());
    }

    #[test]
    fn write_low_nibble_lands_in_low_bits_of_first_byte() {
        let mut w = BitWriter::new();
        w.write(0xA, 4);
        assert_eq!(w.bit_len(), 4);
        assert_eq!(w.into_bytes(), vec![0x0A]);
    }

    #[test]
    fn two_nibbles_pack_into_one_byte() {
        let mut w = BitWriter::new();
        w.write(0xA, 4);
        w.write(0x5, 4);
        assert_eq!(w.bit_len(), 8);
        assert_eq!(w.into_bytes(), vec![0x5A]);
    }

    #[test]
    fn write_spans_byte_boundary() {
        let mut w = BitWriter::new();
        w.write(0xF, 4);
        w.write(0xFF, 8);
        assert_eq!(w.bit_len(), 12);
        assert_eq!(w.into_bytes(), vec![0xFF, 0x0F]);
    }

    #[test]
    fn write_u32_emits_little_endian_bytes() {
        let mut w = BitWriter::new();
        w.write(0x12345678, 32);
        assert_eq!(w.bit_len(), 32);
        assert_eq!(w.into_bytes(), vec![0x78, 0x56, 0x34, 0x12]);
    }

    #[test]
    fn writing_zero_bits_is_a_noop() {
        let mut w = BitWriter::new();
        w.write(0xFFFF_FFFF, 0);
        assert_eq!(w.bit_len(), 0);
        assert_eq!(w.into_bytes(), Vec::<u8>::new());
    }

    #[test]
    fn writing_zero_value_advances_position() {
        let mut w = BitWriter::new();
        w.write(0, 8);
        assert_eq!(w.bit_len(), 8);
        assert_eq!(w.into_bytes(), vec![0x00]);
    }

    #[test]
    fn high_bits_above_width_are_discarded() {
        let mut w = BitWriter::new();
        w.write(0xFF, 4);
        assert_eq!(w.bit_len(), 4);
        assert_eq!(w.into_bytes(), vec![0x0F]);
    }
}
