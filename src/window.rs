//! Vorbis window application and overlap-add buffer.
//!
//! Literal port of libvorbis `lib/window.c` `_vorbis_apply_window` for the
//! long-block-only (n=2048) case used by Q5.
//!
//! For Q5, all blocks are long (n=2048). The window is applied as:
//!   - samples[i]    *= SIN_WINDOW_2048[i]      for i in 0..1024  (left half, forward)
//!   - samples[i]    *= SIN_WINDOW_2048[2047-i]  for i in 1024..2048 (right half, reversed)
//!
//! Overlap-add: each new 1024-sample advance creates a 2048-sample PCM block
//! by concatenating the previous 1024 samples with the new 1024 samples.

use crate::tables::window::SIN_WINDOW_2048;

pub(crate) const BLOCK_SIZE: usize = 2048;
pub(crate) const HALF_BLOCK: usize = BLOCK_SIZE / 2; // 1024

/// Holds the previous 1024-sample block (as f32) for overlap.
pub(crate) struct WindowingBuffer {
    prev: [f32; HALF_BLOCK],
}

impl WindowingBuffer {
    pub(crate) fn new() -> Self {
        Self {
            prev: [0.0f32; HALF_BLOCK],
        }
    }

    /// Produce a 2048-sample windowed block from the previous + current halves.
    ///
    /// `current` is exactly 1024 new PCM samples (already as f32).
    /// Returns the 2048-sample windowed block ready for MDCT.
    pub(crate) fn push_block(&mut self, current: &[f32; HALF_BLOCK]) -> [f32; BLOCK_SIZE] {
        let mut out = [0.0f32; BLOCK_SIZE];

        // Build un-windowed 2048-sample block: [prev | current]
        out[..HALF_BLOCK].copy_from_slice(&self.prev);
        out[HALF_BLOCK..].copy_from_slice(current);

        // Apply window: port of _vorbis_apply_window with lW=1, W=1, nW=1
        // blocksizes[1]=2048, ln=rn=n=2048
        // leftbegin=0, leftend=1024, rightbegin=1024, rightend=2048
        // Left half: out[i] *= windowLW[i] for i in 0..1024
        // Right half: out[i] *= windowNW[rn/2-1 - (i-rightbegin)] for i in 1024..2048
        //   = out[i] *= SIN_WINDOW_2048[1023 - (i - 1024)] = SIN_WINDOW_2048[2047 - i]
        // But our SIN_WINDOW_2048 is the full 2048-point symmetric window,
        // where the left half IS the vwin2048 half-window.
        // SIN_WINDOW_2048[i] = window value at position i in the full block.
        for i in 0..BLOCK_SIZE {
            out[i] *= SIN_WINDOW_2048[i];
        }

        // Save current as previous for next call
        self.prev.copy_from_slice(current);

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::window::SIN_WINDOW_2048;

    #[test]
    fn windowing_all_ones_gives_window_values() {
        let mut buf = WindowingBuffer::new();
        let block = [1.0f32; HALF_BLOCK];

        // First block: prev is zeros, current is ones
        let out = buf.push_block(&block);

        // First half (prev = zeros) should be zero after windowing
        for v in &out[..HALF_BLOCK] {
            assert_eq!(*v, 0.0, "first half should be zero (prev=0 * window)");
        }
        // Second half (current = ones) should equal the window values
        for (idx, (got, exp)) in out[HALF_BLOCK..]
            .iter()
            .zip(&SIN_WINDOW_2048[HALF_BLOCK..])
            .enumerate()
        {
            assert!(
                (got - exp).abs() < 1e-6,
                "out[{}] = {} expected {}",
                HALF_BLOCK + idx,
                got,
                exp
            );
        }
    }

    #[test]
    fn windowing_second_block_all_ones() {
        let mut buf = WindowingBuffer::new();
        let block = [1.0f32; HALF_BLOCK];

        // After first block, prev = [1..1]
        let _first = buf.push_block(&block);

        // Second block: both prev and current are ones → output = window
        let out = buf.push_block(&block);
        for (i, (got, exp)) in out.iter().zip(SIN_WINDOW_2048.iter()).enumerate() {
            assert!(
                (got - exp).abs() < 1e-6,
                "out[{i}] = {} expected {}",
                got,
                exp
            );
        }
    }
}
