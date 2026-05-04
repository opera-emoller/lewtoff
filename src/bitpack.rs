//! LSB-first bit packer/unpacker, per Vorbis I §2.1.4.

/// libvorbis `ov_ilog` (lib/sharedbook.c). Returns ⌊log2(v)⌋ + 1 for v > 0,
/// and 0 for v == 0. Used for bitwidth/codeword sizing throughout the codec.
pub(crate) fn ov_ilog(mut v: u32) -> u32 {
    let mut ret = 0u32;
    while v != 0 {
        ret += 1;
        v >>= 1;
    }
    ret
}

#[derive(Default)]
pub struct BitWriter {
    bytes: Vec<u8>,
    /// Number of bits already written into the *last* byte of `bytes`. In
    /// `1..=8`. When `bytes` is empty, no bits have been written yet.
    bits_in_last: u8,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bit_len(&self) -> usize {
        if self.bytes.is_empty() {
            0
        } else {
            (self.bytes.len() - 1) * 8 + self.bits_in_last as usize
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// LSB-first; `bits` must be `<= 32`.
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

// ---------------------------------------------------------------------------
// BitReader
// ---------------------------------------------------------------------------

/// LSB-first bit reader, counterpart to [`BitWriter`].
pub(crate) struct BitReader<'a> {
    bytes: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, bit_pos: 0 }
    }

    /// Read `bits` bits (LSB-first) and return as a `u32`. `bits` must be <= 32.
    pub fn read(&mut self, bits: u32) -> u32 {
        debug_assert!(bits <= 32, "bits must be <= 32, got {bits}");
        let mut acc: u32 = 0;
        for i in 0..bits {
            let byte_idx = self.bit_pos / 8;
            if byte_idx >= self.bytes.len() {
                // past-end reads return 0xFFFF_FFFF in libvorbis — return max bits
                // to signal EOF (callers check for -1 cast to long in C).
                // Return the partial accumulation ORed with all-ones in remaining bits.
                let remaining = bits - i;
                acc |= ((1u32 << remaining) - 1) << i;
                self.bit_pos += remaining as usize;
                return acc;
            }
            let bit = (self.bytes[byte_idx] >> (self.bit_pos % 8)) & 1;
            acc |= (bit as u32) << i;
            self.bit_pos += 1;
        }
        acc
    }

    /// Read `bits` bits and sign-extend to `i32`.
    #[cfg(test)]
    pub fn read_signed(&mut self, bits: u32) -> i32 {
        let v = self.read(bits);
        if bits == 0 {
            return 0;
        }
        let shift = 32 - bits;
        ((v << shift) as i32) >> shift
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

    #[test]
    fn round_trip_against_reader() {
        let cases: Vec<(u32, u32)> = vec![
            (0, 0),
            (0b1, 1),
            (0b101, 3),
            (0xA, 4),
            (0x5A, 8),
            (0x1234, 16),
            (0xDEADBEEF, 32),
            (0b1, 1),
            (0b11_1111_1111, 10),
            (0, 7),
            (0xFFFF_FFFF, 32),
            (0xFF, 4),
        ];

        let mut w = BitWriter::new();
        let mut total_bits = 0u32;
        for (v, b) in &cases {
            w.write(*v, *b);
            total_bits += *b;
        }
        assert_eq!(w.bit_len(), total_bits as usize);

        let bytes = w.into_bytes();
        let mut r = BitReader::new(&bytes);
        for (v, b) in &cases {
            let got = r.read(*b);
            let expected = if *b == 32 { *v } else { *v & ((1u32 << b) - 1) };
            assert_eq!(
                got, expected,
                "round-trip mismatch for write({v:#x}, {b}): got {got:#x}, expected {expected:#x}"
            );
        }
    }

    #[test]
    fn reader_read_zero_bits_returns_zero() {
        let bytes = vec![0xFFu8];
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read(0), 0);
    }

    #[test]
    fn reader_reads_lsb_first() {
        // byte 0xA = 0b0000_1010 → bits: 0,1,0,1,0,0,0,0
        let bytes = vec![0x0Au8];
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read(1), 0); // bit 0
        assert_eq!(r.read(1), 1); // bit 1
        assert_eq!(r.read(1), 0); // bit 2
        assert_eq!(r.read(1), 1); // bit 3
    }

    #[test]
    fn reader_read_signed_sign_extends() {
        // Write 0b11111 (5 bits) = 31, sign-extended as i32 = -1
        let mut w = BitWriter::new();
        w.write(0b11111, 5);
        let bytes = w.into_bytes();
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read_signed(5), -1);
    }

    #[test]
    fn reader_read_signed_positive() {
        let mut w = BitWriter::new();
        w.write(0b01010, 5); // 10 in 5 bits
        let bytes = w.into_bytes();
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read_signed(5), 10);
    }
}
