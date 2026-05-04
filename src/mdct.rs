//! Forward MDCT for n=2048 and n=256, port of libvorbis 1.3.7 lib/mdct.c.
//!
//! Tables come from src/tables/trig.rs. No runtime transcendentals.

#![allow(clippy::needless_range_loop)]
#![allow(clippy::identity_op)]
#![allow(clippy::eq_op)]
#![allow(clippy::excessive_precision)]
#![allow(clippy::approx_constant)]

use crate::tables::trig::{
    BITREV_128, BITREV_256, BITREV_2048, SCALE_128, SCALE_256, SCALE_2048, TRIG_128, TRIG_256,
    TRIG_2048,
};

// Constants from mdct.h (float mode)
const CPI3_8: f32 = 0.38268343236508977175_f32;
const CPI2_8: f32 = 0.70710678118654752441_f32;
const CPI1_8: f32 = 0.92387953251128675613_f32;

const N: usize = 2048;
const N2: usize = N / 2; // 1024

/* Generic versions that accept a trig table slice */

fn mdct_butterfly_8_g(x: &mut [f32], base: usize) {
    let r0 = x[base + 6] + x[base + 2];
    let r1 = x[base + 6] - x[base + 2];
    let r2 = x[base + 4] + x[base + 0];
    let r3 = x[base + 4] - x[base + 0];

    x[base + 6] = r0 + r2;
    x[base + 4] = r0 - r2;

    let r0 = x[base + 5] - x[base + 1];
    let r2 = x[base + 7] - x[base + 3];
    x[base + 0] = r1 + r0;
    x[base + 2] = r1 - r0;

    let r0 = x[base + 5] + x[base + 1];
    let r1 = x[base + 7] + x[base + 3];
    x[base + 3] = r2 + r3;
    x[base + 1] = r2 - r3;
    x[base + 7] = r1 + r0;
    x[base + 5] = r1 - r0;
}

fn mdct_butterfly_16_g(x: &mut [f32], base: usize) {
    let r0 = x[base + 1] - x[base + 9];
    let r1 = x[base + 0] - x[base + 8];

    x[base + 8] += x[base + 0];
    x[base + 9] += x[base + 1];
    x[base + 0] = (r0 + r1) * CPI2_8;
    x[base + 1] = (r0 - r1) * CPI2_8;

    let r0 = x[base + 3] - x[base + 11];
    let r1 = x[base + 10] - x[base + 2];
    x[base + 10] += x[base + 2];
    x[base + 11] += x[base + 3];
    x[base + 2] = r0;
    x[base + 3] = r1;

    let r0 = x[base + 12] - x[base + 4];
    let r1 = x[base + 13] - x[base + 5];
    x[base + 12] += x[base + 4];
    x[base + 13] += x[base + 5];
    x[base + 4] = (r0 - r1) * CPI2_8;
    x[base + 5] = (r0 + r1) * CPI2_8;

    let r0 = x[base + 14] - x[base + 6];
    let r1 = x[base + 15] - x[base + 7];
    x[base + 14] += x[base + 6];
    x[base + 15] += x[base + 7];
    x[base + 6] = r0;
    x[base + 7] = r1;

    mdct_butterfly_8_g(x, base);
    mdct_butterfly_8_g(x, base + 8);
}

fn mdct_butterfly_32_g(x: &mut [f32], base: usize) {
    let r0 = x[base + 30] - x[base + 14];
    let r1 = x[base + 31] - x[base + 15];
    x[base + 30] += x[base + 14];
    x[base + 31] += x[base + 15];
    x[base + 14] = r0;
    x[base + 15] = r1;

    let r0 = x[base + 28] - x[base + 12];
    let r1 = x[base + 29] - x[base + 13];
    x[base + 28] += x[base + 12];
    x[base + 29] += x[base + 13];
    x[base + 12] = r0 * CPI1_8 - r1 * CPI3_8;
    x[base + 13] = r0 * CPI3_8 + r1 * CPI1_8;

    let r0 = x[base + 26] - x[base + 10];
    let r1 = x[base + 27] - x[base + 11];
    x[base + 26] += x[base + 10];
    x[base + 27] += x[base + 11];
    x[base + 10] = (r0 - r1) * CPI2_8;
    x[base + 11] = (r0 + r1) * CPI2_8;

    let r0 = x[base + 24] - x[base + 8];
    let r1 = x[base + 25] - x[base + 9];
    x[base + 24] += x[base + 8];
    x[base + 25] += x[base + 9];
    x[base + 8] = r0 * CPI3_8 - r1 * CPI1_8;
    x[base + 9] = r1 * CPI3_8 + r0 * CPI1_8;

    let r0 = x[base + 22] - x[base + 6];
    let r1 = x[base + 7] - x[base + 23];
    x[base + 22] += x[base + 6];
    x[base + 23] += x[base + 7];
    x[base + 6] = r1;
    x[base + 7] = r0;

    let r0 = x[base + 4] - x[base + 20];
    let r1 = x[base + 5] - x[base + 21];
    x[base + 20] += x[base + 4];
    x[base + 21] += x[base + 5];
    x[base + 4] = r1 * CPI1_8 + r0 * CPI3_8;
    x[base + 5] = r1 * CPI3_8 - r0 * CPI1_8;

    let r0 = x[base + 2] - x[base + 18];
    let r1 = x[base + 3] - x[base + 19];
    x[base + 18] += x[base + 2];
    x[base + 19] += x[base + 3];
    x[base + 2] = (r1 + r0) * CPI2_8;
    x[base + 3] = (r1 - r0) * CPI2_8;

    let r0 = x[base + 0] - x[base + 16];
    let r1 = x[base + 1] - x[base + 17];
    x[base + 16] += x[base + 0];
    x[base + 17] += x[base + 1];
    x[base + 0] = r1 * CPI3_8 + r0 * CPI1_8;
    x[base + 1] = r1 * CPI1_8 - r0 * CPI3_8;

    mdct_butterfly_16_g(x, base);
    mdct_butterfly_16_g(x, base + 16);
}

fn mdct_butterfly_first_g(
    trig: &[f32],
    t_base: usize,
    x: &mut [f32],
    x_base: usize,
    points: usize,
) {
    let mut x1 = x_base + points - 8;
    let mut x2 = x_base + (points >> 1) - 8;
    let mut t = t_base;

    loop {
        let r0 = x[x1 + 6] - x[x2 + 6];
        let r1 = x[x1 + 7] - x[x2 + 7];
        x[x1 + 6] += x[x2 + 6];
        x[x1 + 7] += x[x2 + 7];
        x[x2 + 6] = r1 * trig[t + 1] + r0 * trig[t + 0];
        x[x2 + 7] = r1 * trig[t + 0] - r0 * trig[t + 1];

        let r0 = x[x1 + 4] - x[x2 + 4];
        let r1 = x[x1 + 5] - x[x2 + 5];
        x[x1 + 4] += x[x2 + 4];
        x[x1 + 5] += x[x2 + 5];
        x[x2 + 4] = r1 * trig[t + 5] + r0 * trig[t + 4];
        x[x2 + 5] = r1 * trig[t + 4] - r0 * trig[t + 5];

        let r0 = x[x1 + 2] - x[x2 + 2];
        let r1 = x[x1 + 3] - x[x2 + 3];
        x[x1 + 2] += x[x2 + 2];
        x[x1 + 3] += x[x2 + 3];
        x[x2 + 2] = r1 * trig[t + 9] + r0 * trig[t + 8];
        x[x2 + 3] = r1 * trig[t + 8] - r0 * trig[t + 9];

        let r0 = x[x1 + 0] - x[x2 + 0];
        let r1 = x[x1 + 1] - x[x2 + 1];
        x[x1 + 0] += x[x2 + 0];
        x[x1 + 1] += x[x2 + 1];
        x[x2 + 0] = r1 * trig[t + 13] + r0 * trig[t + 12];
        x[x2 + 1] = r1 * trig[t + 12] - r0 * trig[t + 13];

        if x2 == x_base {
            break;
        }
        x1 -= 8;
        x2 -= 8;
        t += 16;
    }
}

fn mdct_butterfly_generic_g(
    trig: &[f32],
    t_base: usize,
    x: &mut [f32],
    x_base: usize,
    points: usize,
    trigint: usize,
) {
    let mut x1 = x_base + points - 8;
    let mut x2 = x_base + (points >> 1) - 8;
    let mut t = t_base;

    loop {
        let r0 = x[x1 + 6] - x[x2 + 6];
        let r1 = x[x1 + 7] - x[x2 + 7];
        x[x1 + 6] += x[x2 + 6];
        x[x1 + 7] += x[x2 + 7];
        x[x2 + 6] = r1 * trig[t + 1] + r0 * trig[t + 0];
        x[x2 + 7] = r1 * trig[t + 0] - r0 * trig[t + 1];

        t += trigint;

        let r0 = x[x1 + 4] - x[x2 + 4];
        let r1 = x[x1 + 5] - x[x2 + 5];
        x[x1 + 4] += x[x2 + 4];
        x[x1 + 5] += x[x2 + 5];
        x[x2 + 4] = r1 * trig[t + 1] + r0 * trig[t + 0];
        x[x2 + 5] = r1 * trig[t + 0] - r0 * trig[t + 1];

        t += trigint;

        let r0 = x[x1 + 2] - x[x2 + 2];
        let r1 = x[x1 + 3] - x[x2 + 3];
        x[x1 + 2] += x[x2 + 2];
        x[x1 + 3] += x[x2 + 3];
        x[x2 + 2] = r1 * trig[t + 1] + r0 * trig[t + 0];
        x[x2 + 3] = r1 * trig[t + 0] - r0 * trig[t + 1];

        t += trigint;

        let r0 = x[x1 + 0] - x[x2 + 0];
        let r1 = x[x1 + 1] - x[x2 + 1];
        x[x1 + 0] += x[x2 + 0];
        x[x1 + 1] += x[x2 + 1];
        x[x2 + 0] = r1 * trig[t + 1] + r0 * trig[t + 0];
        x[x2 + 1] = r1 * trig[t + 0] - r0 * trig[t + 1];

        t += trigint;

        if x2 == x_base {
            break;
        }
        x1 -= 8;
        x2 -= 8;
    }
}

fn mdct_butterflies_g(trig: &[f32], log2n: usize, x: &mut [f32], x_base: usize, points: usize) {
    let mut stages: i32 = (log2n - 5) as i32;

    if {
        stages -= 1;
        stages
    } > 0
    {
        mdct_butterfly_first_g(trig, 0, x, x_base, points);
    }

    let mut i: usize = 1;
    loop {
        stages -= 1;
        if stages <= 0 {
            break;
        }
        for j in 0..(1usize << i) {
            mdct_butterfly_generic_g(trig, 0, x, x_base + (points >> i) * j, points >> i, 4 << i);
        }
        i += 1;
    }

    let mut j = x_base;
    while j < x_base + points {
        mdct_butterfly_32_g(x, j);
        j += 32;
    }
}

fn mdct_bitreverse_g(trig: &[f32], bitrev: &[u32], n: usize, w: &mut [f32]) {
    let n2 = n / 2;
    let mut w0: usize = 0;
    let mut w1: usize = n2;
    let mut t: usize = n;
    let mut bit: usize = 0;

    loop {
        let x0 = n2 + bitrev[bit] as usize;
        let x1 = n2 + bitrev[bit + 1] as usize;

        let r0 = w[x0 + 1] - w[x1 + 1];
        let r1 = w[x0 + 0] + w[x1 + 0];
        let r2 = r1 * trig[t + 0] + r0 * trig[t + 1];
        let r3 = r1 * trig[t + 1] - r0 * trig[t + 0];

        w1 -= 4;

        let r0 = (w[x0 + 1] + w[x1 + 1]) * 0.5_f32;
        let r1 = (w[x0 + 0] - w[x1 + 0]) * 0.5_f32;

        w[w0 + 0] = r0 + r2;
        w[w1 + 2] = r0 - r2;
        w[w0 + 1] = r1 + r3;
        w[w1 + 3] = r3 - r1;

        let x0 = n2 + bitrev[bit + 2] as usize;
        let x1 = n2 + bitrev[bit + 3] as usize;

        let r0 = w[x0 + 1] - w[x1 + 1];
        let r1 = w[x0 + 0] + w[x1 + 0];
        let r2 = r1 * trig[t + 2] + r0 * trig[t + 3];
        let r3 = r1 * trig[t + 3] - r0 * trig[t + 2];

        let r0 = (w[x0 + 1] + w[x1 + 1]) * 0.5_f32;
        let r1 = (w[x0 + 0] - w[x1 + 0]) * 0.5_f32;

        w[w0 + 2] = r0 + r2;
        w[w1 + 0] = r0 - r2;
        w[w0 + 3] = r1 + r3;
        w[w1 + 1] = r3 - r1;

        t += 4;
        bit += 4;
        w0 += 4;

        if w0 >= w1 {
            break;
        }
    }
}

fn mdct_forward_generic(
    input: &[f32],
    output: &mut [f32],
    n: usize,
    log2n: usize,
    trig: &[f32],
    bitrev: &[u32],
    scale: f32,
) {
    let n2 = n / 2;
    let n4 = n / 4;
    let n8 = n / 8;

    // Stack-allocated scratch (n <= N=2048). Avoids per-call heap alloc;
    // wastes some stack for short / envelope MDCTs but the absolute sizes
    // are small.
    let mut w_storage = [0f32; N];
    let w = &mut w_storage[..n];

    let mut x0_idx: usize = n2 + n4;
    let mut x1_idx: usize = n2 + n4 + 1;
    let mut t_idx: usize = n2;

    let mut i = 0;
    while i < n8 {
        x0_idx -= 4;
        t_idx -= 2;

        let r0 = input[x0_idx + 2] + input[x1_idx];
        let r1 = input[x0_idx + 0] + input[x1_idx + 2];
        w[n2 + i] = r1 * trig[t_idx + 1] + r0 * trig[t_idx + 0];
        w[n2 + i + 1] = r1 * trig[t_idx + 0] - r0 * trig[t_idx + 1];

        x1_idx += 4;
        i += 2;
    }

    x1_idx = 1;

    while i < n2 - n8 {
        x0_idx -= 4;
        t_idx -= 2;

        let r0 = input[x0_idx + 2] - input[x1_idx];
        let r1 = input[x0_idx + 0] - input[x1_idx + 2];
        w[n2 + i] = r1 * trig[t_idx + 1] + r0 * trig[t_idx + 0];
        w[n2 + i + 1] = r1 * trig[t_idx + 0] - r0 * trig[t_idx + 1];

        x1_idx += 4;
        i += 2;
    }

    x0_idx = n;

    while i < n2 {
        x0_idx -= 4;
        t_idx -= 2;

        let r0 = -input[x0_idx + 2] - input[x1_idx];
        let r1 = -input[x0_idx + 0] - input[x1_idx + 2];
        w[n2 + i] = r1 * trig[t_idx + 1] + r0 * trig[t_idx + 0];
        w[n2 + i + 1] = r1 * trig[t_idx + 0] - r0 * trig[t_idx + 1];

        x1_idx += 4;
        i += 2;
    }

    mdct_butterflies_g(trig, log2n, w, n2, n2);
    mdct_bitreverse_g(trig, bitrev, n, w);

    let mut t_idx: usize = n2;
    let mut w_idx: usize = 0;

    for k in 0..n4 {
        let x0_pos = n2 - 1 - k;
        output[k] = (w[w_idx] * trig[t_idx] + w[w_idx + 1] * trig[t_idx + 1]) * scale;
        output[x0_pos] = (w[w_idx] * trig[t_idx + 1] - w[w_idx + 1] * trig[t_idx]) * scale;
        w_idx += 2;
        t_idx += 2;
    }
}

pub fn mdct_forward(input: &[f32; N], output: &mut [f32; N2]) {
    mdct_forward_long(input, output)
}

pub(crate) fn mdct_forward_long(input: &[f32; N], output: &mut [f32; N2]) {
    mdct_forward_generic(input, output, N, 11, &TRIG_2048, &BITREV_2048, SCALE_2048);
}

pub(crate) fn mdct_forward_short(input: &[f32; 256], output: &mut [f32; 128]) {
    mdct_forward_generic(input, output, 256, 8, &TRIG_256, &BITREV_256, SCALE_256);
}

/// MDCT for envelope detection (n=128, used by `_ve_amp`).
pub(crate) fn mdct_forward_envelope(input: &[f32; 128], output: &mut [f32; 64]) {
    mdct_forward_generic(input, output, 128, 7, &TRIG_128, &BITREV_128, SCALE_128);
}
