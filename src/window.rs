//! Vorbis window application and overlap-add buffer.

#![allow(clippy::needless_range_loop)]
#![allow(clippy::explicit_counter_loop)]
//!
//! Port of libvorbis `lib/window.c` `_vorbis_apply_window` supporting both
//! long (n=2048) and short (n=256) blocks and their transitions.
//!
//! Window transition logic:
//!   _vorbis_apply_window(d, winno, blocksizes, lW, W, nW):
//!     - lW=(W?lW:0), nW=(W?nW:0)
//!     - windowLW = vwin[winno[lW]], windowNW = vwin[winno[nW]]
//!     - n = blocksizes[W]; ln = blocksizes[lW]; rn = blocksizes[nW]
//!     - leftbegin = n/4 - ln/4, leftend = leftbegin + ln/2
//!     - rightbegin = n/2 + n/4 - rn/4, rightend = rightbegin + rn/2
//!     - d[0..leftbegin] = 0
//!     - d[leftbegin..leftend] *= windowLW[0..ln/2]
//!     - d[rightbegin..rightend] *= windowNW[rn/2-1..0] (reversed)
//!     - d[rightend..n] = 0
//!
//! For Q5 (blocksizes[0]=256, blocksizes[1]=2048):
//!   winno[0] = ilog(256)-7 = 1 → vwin[1] = vwin128 (128 values, our WIN_HALF_256)
//!   winno[1] = ilog(2048)-7 = 4 → vwin[4] = vwin1024 (1024 values, our WIN_HALF_2048)
//!
//! LIMITATION: Only supports the "first block short, all subsequent long"
//! pattern. No transient detection. Suitable for non-transient test corpora
//! (silence, sine, ramp). Real-world audio with attacks would require full
//! transient detection from libvorbis vorbis_analysis_blockout.

use crate::tables::window::{WIN_HALF_2048, WIN_HALF_256};

pub(crate) const LONG_BLOCK: usize = 2048;
pub(crate) const SHORT_BLOCK: usize = 256;
pub(crate) const LONG_HALF: usize = LONG_BLOCK / 2; // 1024
pub(crate) const SHORT_HALF: usize = SHORT_BLOCK / 2; // 128

pub(crate) const BLOCK_SIZE: usize = LONG_BLOCK;
pub(crate) const HALF_BLOCK: usize = LONG_HALF;

/// Apply _vorbis_apply_window for a block of size `n`.
///
/// `lW`: is the previous block long? (false=short, true=long)
/// `w`: is the current block long? (false=short, true=long)
/// `nW`: is the next block long? (false=short, true=long)
///
/// For short blocks (W=0), lW and nW are forced to 0 regardless.
/// The `d` slice must have length n.
fn apply_window(d: &mut [f32], lw: bool, w: bool, nw: bool) {
    let lw = if w { lw } else { false };
    let nw = if w { nw } else { false };

    let n = if w { LONG_BLOCK } else { SHORT_BLOCK };
    let ln = if lw { LONG_BLOCK } else { SHORT_BLOCK };
    let rn = if nw { LONG_BLOCK } else { SHORT_BLOCK };

    let window_lw: &[f32] = if lw { &WIN_HALF_2048 } else { &WIN_HALF_256 };
    let window_nw: &[f32] = if nw { &WIN_HALF_2048 } else { &WIN_HALF_256 };

    let leftbegin = n / 4 - ln / 4;
    let leftend = leftbegin + ln / 2;
    let rightbegin = n / 2 + n / 4 - rn / 4;
    let rightend = rightbegin + rn / 2;

    for i in 0..leftbegin {
        d[i] = 0.0;
    }

    let mut p: usize = 0;
    for i in leftbegin..leftend {
        d[i] *= window_lw[p];
        p += 1;
    }

    let mut p: usize = rn / 2 - 1;
    for i in rightbegin..rightend {
        d[i] *= window_nw[p];
        p = p.wrapping_sub(1);
    }

    for i in rightend..n {
        d[i] = 0.0;
    }
}

/// Holds state for windowing between blocks.
pub(crate) struct WindowingBuffer {
    /// Previous 1024 samples (long block overlap buffer).
    prev_long: [f32; LONG_HALF],
    /// Previous 128 samples (short block overlap buffer).
    prev_short: [f32; SHORT_HALF],
    /// Was the previous block a long block?
    prev_is_long: bool,
}

impl WindowingBuffer {
    pub(crate) fn new() -> Self {
        Self {
            prev_long: [0.0f32; LONG_HALF],
            prev_short: [0.0f32; SHORT_HALF],
            prev_is_long: false,
        }
    }

    /// Produce a windowed long (2048-sample) block.
    ///
    /// `current` is the next LONG_HALF = 1024 PCM samples.
    /// `nw_is_long`: is the next block long?
    pub(crate) fn push_long_block(
        &mut self,
        current: &[f32; LONG_HALF],
        nw_is_long: bool,
    ) -> [f32; LONG_BLOCK] {
        let lw = self.prev_is_long;
        let mut out = [0.0f32; LONG_BLOCK];

        if lw {
            // prev was long: copy prev_long into left half
            out[..LONG_HALF].copy_from_slice(&self.prev_long);
        } else {
            // prev was short: center the short overlap in the left half
            // leftbegin = 2048/4 - 256/4 = 512 - 64 = 448
            // leftend = 448 + 128 = 576
            // The overlap region from the short block occupies [448..576]
            let leftbegin = LONG_BLOCK / 4 - SHORT_BLOCK / 4;
            let leftend = leftbegin + SHORT_HALF;
            out[leftbegin..leftend].copy_from_slice(&self.prev_short);
            // Samples [0..448] and [576..1024] remain 0 (already zero-initialized)
        }

        // Right half: current samples
        out[LONG_HALF..].copy_from_slice(current);

        // Apply the transition window
        apply_window(&mut out, lw, true, nw_is_long);

        // Update state
        self.prev_long.copy_from_slice(current);
        self.prev_is_long = true;

        out
    }

    /// Produce a windowed short (256-sample) block.
    ///
    /// `current` is the next SHORT_HALF = 128 PCM samples.
    pub(crate) fn push_short_block(&mut self, current: &[f32; SHORT_HALF]) -> [f32; SHORT_BLOCK] {
        let mut out = [0.0f32; SHORT_BLOCK];

        // Build un-windowed 256-sample block: [prev_short | current]
        out[..SHORT_HALF].copy_from_slice(&self.prev_short);
        out[SHORT_HALF..].copy_from_slice(current);

        // Apply window: W=0 → lW=0, nW=0 (forced by _vorbis_apply_window)
        apply_window(&mut out, false, false, false);

        // Update state
        self.prev_short.copy_from_slice(current);
        self.prev_is_long = false;

        out
    }

    #[cfg(test)]
    pub(crate) fn push_block(&mut self, current: &[f32; HALF_BLOCK]) -> [f32; BLOCK_SIZE] {
        self.push_long_block(current, true)
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

    #[test]
    fn short_block_window_is_symmetric() {
        let mut buf = WindowingBuffer::new();
        let block = [1.0f32; SHORT_HALF];

        let out = buf.push_short_block(&block);

        // First half is prev=zeros, second half is ones
        // After windowing:
        //   leftbegin=0, leftend=128, rightbegin=128, rightend=256
        //   d[0..128] *= WIN_HALF_256[0..128]  (but d[0..128] = 0, so stays 0)
        //   d[128..256] *= WIN_HALF_256[127..0] reversed
        for v in &out[..SHORT_HALF] {
            assert_eq!(*v, 0.0, "short block first half should be zero (prev=0)");
        }
        // Second half should equal WIN_HALF_256 reversed
        for (i, &v) in out[SHORT_HALF..].iter().enumerate() {
            let exp = WIN_HALF_256[SHORT_HALF - 1 - i];
            assert!(
                (v - exp).abs() < 1e-6,
                "short block out[{}] = {} expected {}",
                SHORT_HALF + i,
                v,
                exp
            );
        }
    }
}
