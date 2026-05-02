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
    pub fn write(&mut self, _value: u32, _bits: u32) {
        unimplemented!("Phase 1 Task 1.2")
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
}
