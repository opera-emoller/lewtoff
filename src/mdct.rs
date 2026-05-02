//! Forward MDCT for n=2048, port of libvorbis 1.3.7 lib/mdct.c.
//!
//! Tables come from src/tables/trig.rs. No runtime transcendentals.

#![allow(clippy::needless_range_loop)]
#![allow(clippy::identity_op)]
#![allow(clippy::eq_op)]
#![allow(clippy::excessive_precision)]
#![allow(clippy::approx_constant)]

use crate::tables::trig::{BITREV_2048, SCALE_2048, TRIG_2048};

// Constants from mdct.h (float mode)
const CPI3_8: f32 = 0.38268343236508977175_f32;
const CPI2_8: f32 = 0.70710678118654752441_f32;
const CPI1_8: f32 = 0.92387953251128675613_f32;

const N: usize = 2048;
const N2: usize = N / 2; // 1024
const N4: usize = N / 4; // 512
const N8: usize = N / 8; // 256

// T = TRIG_2048
// T[0..N2)      : butterfly_first / butterfly_generic trig (cos/sin pairs at offsets 0..N2)
// T[N2..N)      : initial rotation + final rotation trig (cos/sin pairs at offsets N2..N)
// T[N..N+N4)    : bitreverse trig (cos/sin pairs at offsets N..N+N4)

/* 8 point butterfly (in place) */
#[allow(clippy::many_single_char_names)]
fn mdct_butterfly_8(x: &mut [f32], base: usize) {
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

/* 16 point butterfly (in place) */
#[allow(clippy::many_single_char_names)]
fn mdct_butterfly_16(x: &mut [f32], base: usize) {
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

    mdct_butterfly_8(x, base);
    mdct_butterfly_8(x, base + 8);
}

/* 32 point butterfly (in place) */
#[allow(clippy::many_single_char_names)]
fn mdct_butterfly_32(x: &mut [f32], base: usize) {
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

    mdct_butterfly_16(x, base);
    mdct_butterfly_16(x, base + 16);
}

/* N point first stage butterfly (in place, 2 register)
   T starts at index 0 into TRIG_2048, advances by 16 per outer iteration.
   x1 starts at x_base + points - 8, x2 starts at x_base + points/2 - 8,
   both decrement by 8 per iteration until x2 < x_base.
*/
fn mdct_butterfly_first(t_base: usize, x: &mut [f32], x_base: usize, points: usize) {
    // x1 = x_base + points - 8
    // x2 = x_base + points/2 - 8
    // loop: x1 -= 8, x2 -= 8, T += 16, while x2 >= x_base
    let mut x1 = x_base + points - 8;
    let mut x2 = x_base + (points >> 1) - 8;
    let mut t = t_base;

    loop {
        let r0 = x[x1 + 6] - x[x2 + 6];
        let r1 = x[x1 + 7] - x[x2 + 7];
        x[x1 + 6] += x[x2 + 6];
        x[x1 + 7] += x[x2 + 7];
        x[x2 + 6] = r1 * TRIG_2048[t + 1] + r0 * TRIG_2048[t + 0];
        x[x2 + 7] = r1 * TRIG_2048[t + 0] - r0 * TRIG_2048[t + 1];

        let r0 = x[x1 + 4] - x[x2 + 4];
        let r1 = x[x1 + 5] - x[x2 + 5];
        x[x1 + 4] += x[x2 + 4];
        x[x1 + 5] += x[x2 + 5];
        x[x2 + 4] = r1 * TRIG_2048[t + 5] + r0 * TRIG_2048[t + 4];
        x[x2 + 5] = r1 * TRIG_2048[t + 4] - r0 * TRIG_2048[t + 5];

        let r0 = x[x1 + 2] - x[x2 + 2];
        let r1 = x[x1 + 3] - x[x2 + 3];
        x[x1 + 2] += x[x2 + 2];
        x[x1 + 3] += x[x2 + 3];
        x[x2 + 2] = r1 * TRIG_2048[t + 9] + r0 * TRIG_2048[t + 8];
        x[x2 + 3] = r1 * TRIG_2048[t + 8] - r0 * TRIG_2048[t + 9];

        let r0 = x[x1 + 0] - x[x2 + 0];
        let r1 = x[x1 + 1] - x[x2 + 1];
        x[x1 + 0] += x[x2 + 0];
        x[x1 + 1] += x[x2 + 1];
        x[x2 + 0] = r1 * TRIG_2048[t + 13] + r0 * TRIG_2048[t + 12];
        x[x2 + 1] = r1 * TRIG_2048[t + 12] - r0 * TRIG_2048[t + 13];

        if x2 == x_base {
            break;
        }
        x1 -= 8;
        x2 -= 8;
        t += 16;
    }
}

/* N/stage point generic N stage butterfly (in place, 2 register) */
fn mdct_butterfly_generic(
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
        x[x2 + 6] = r1 * TRIG_2048[t + 1] + r0 * TRIG_2048[t + 0];
        x[x2 + 7] = r1 * TRIG_2048[t + 0] - r0 * TRIG_2048[t + 1];

        t += trigint;

        let r0 = x[x1 + 4] - x[x2 + 4];
        let r1 = x[x1 + 5] - x[x2 + 5];
        x[x1 + 4] += x[x2 + 4];
        x[x1 + 5] += x[x2 + 5];
        x[x2 + 4] = r1 * TRIG_2048[t + 1] + r0 * TRIG_2048[t + 0];
        x[x2 + 5] = r1 * TRIG_2048[t + 0] - r0 * TRIG_2048[t + 1];

        t += trigint;

        let r0 = x[x1 + 2] - x[x2 + 2];
        let r1 = x[x1 + 3] - x[x2 + 3];
        x[x1 + 2] += x[x2 + 2];
        x[x1 + 3] += x[x2 + 3];
        x[x2 + 2] = r1 * TRIG_2048[t + 1] + r0 * TRIG_2048[t + 0];
        x[x2 + 3] = r1 * TRIG_2048[t + 0] - r0 * TRIG_2048[t + 1];

        t += trigint;

        let r0 = x[x1 + 0] - x[x2 + 0];
        let r1 = x[x1 + 1] - x[x2 + 1];
        x[x1 + 0] += x[x2 + 0];
        x[x1 + 1] += x[x2 + 1];
        x[x2 + 0] = r1 * TRIG_2048[t + 1] + r0 * TRIG_2048[t + 0];
        x[x2 + 1] = r1 * TRIG_2048[t + 0] - r0 * TRIG_2048[t + 1];

        t += trigint;

        if x2 == x_base {
            break;
        }
        x1 -= 8;
        x2 -= 8;
    }
}

fn mdct_butterflies(x: &mut [f32], x_base: usize, points: usize) {
    // T = TRIG_2048 starting at 0
    // stages = log2n - 5 = 11 - 5 = 6
    let log2n: usize = 11;
    let mut stages: i32 = (log2n - 5) as i32; // 6

    if {
        stages -= 1;
        stages
    } > 0
    {
        // stages now 5
        mdct_butterfly_first(0, x, x_base, points);
    }

    // i starts at 1, stages decrements each outer iteration
    let mut i: usize = 1;
    loop {
        stages -= 1;
        if stages <= 0 {
            break;
        }
        for j in 0..(1usize << i) {
            mdct_butterfly_generic(0, x, x_base + (points >> i) * j, points >> i, 4 << i);
        }
        i += 1;
    }

    let mut j = x_base;
    while j < x_base + points {
        mdct_butterfly_32(x, j);
        j += 32;
    }
}

fn mdct_bitreverse(w: &mut [f32]) {
    // bit = BITREV_2048[0..]
    // w0 = w[0], w1 = w[N2] initially, w1 decrements by 4 each iter
    // T = TRIG_2048[N..] (offset N=2048 into the trig table)
    // x = w[N2..] as the base for bit-indexed lookups

    let mut w0: usize = 0;
    let mut w1: usize = N2; // w1 starts at N2, decrements by 4
    let mut t: usize = N; // T = TRIG_2048 + N
    let mut bit: usize = 0; // index into BITREV_2048

    loop {
        // x0 = x + bit[0], x1 = x + bit[1]  (x = w[N2..])
        let x0 = N2 + BITREV_2048[bit] as usize;
        let x1 = N2 + BITREV_2048[bit + 1] as usize;

        let r0 = w[x0 + 1] - w[x1 + 1];
        let r1 = w[x0 + 0] + w[x1 + 0];
        let r2 = r1 * TRIG_2048[t + 0] + r0 * TRIG_2048[t + 1];
        let r3 = r1 * TRIG_2048[t + 1] - r0 * TRIG_2048[t + 0];

        w1 -= 4;

        let r0 = (w[x0 + 1] + w[x1 + 1]) * 0.5_f32;
        let r1 = (w[x0 + 0] - w[x1 + 0]) * 0.5_f32;

        w[w0 + 0] = r0 + r2;
        w[w1 + 2] = r0 - r2;
        w[w0 + 1] = r1 + r3;
        w[w1 + 3] = r3 - r1;

        let x0 = N2 + BITREV_2048[bit + 2] as usize;
        let x1 = N2 + BITREV_2048[bit + 3] as usize;

        let r0 = w[x0 + 1] - w[x1 + 1];
        let r1 = w[x0 + 0] + w[x1 + 0];
        let r2 = r1 * TRIG_2048[t + 2] + r0 * TRIG_2048[t + 3];
        let r3 = r1 * TRIG_2048[t + 3] - r0 * TRIG_2048[t + 2];

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

pub fn mdct_forward(input: &[f32; N], output: &mut [f32; N2]) {
    // Port of mdct_forward in lib/mdct.c, specialized to n=2048.
    let n8 = N8;
    let n4 = N4;

    // Working space: w[0..N], w2 = w[N2..N]
    let mut w = [0f32; N];

    // rotate: window + rotate + step 1
    // x0 = in + n2 + n4 = input[n2+n4]  (pointer, decrements by 4)
    // x1 = x0 + 1      = input[n2+n4+1] (pointer, increments by 4)
    // T  = trig + n2   = TRIG_2048[N2]  (pointer, decrements by 2)
    // w2 = w + n2      = w[N2..]

    // First loop: i=0..n8 step 2
    //   x0 -= 4; T -= 2;
    //   r0 = x0[2] + x1[0]; r1 = x0[0] + x1[2]
    //   w2[i]   = r1*T[1] + r0*T[0]
    //   w2[i+1] = r1*T[0] - r0*T[1]
    //   x1 += 4

    // x0 starts at n2+n4 as pointer (but we subtract 4 before reading x0[2]),
    // so first read x0[2] is at n2+n4-4+2 = n2+n4-2
    // x1 starts at n2+n4+1, x1[0] = input[n2+n4+1]

    let mut x0_idx: usize = N2 + N4; // initial pointer position
    let mut x1_idx: usize = N2 + N4 + 1;
    let mut t_idx: usize = N2; // T = TRIG_2048 + N2

    let mut i = 0;
    while i < n8 {
        // x0 -= 4 and T -= 2 at start of loop
        x0_idx -= 4;
        t_idx -= 2;

        let r0 = input[x0_idx + 2] + input[x1_idx];
        let r1 = input[x0_idx + 0] + input[x1_idx + 2];
        w[N2 + i] = r1 * TRIG_2048[t_idx + 1] + r0 * TRIG_2048[t_idx + 0];
        w[N2 + i + 1] = r1 * TRIG_2048[t_idx + 0] - r0 * TRIG_2048[t_idx + 1];

        x1_idx += 4;
        i += 2;
    }

    // Second loop: i=n8..n2-n8 step 2
    //   x1 = in + 1 (reset)
    //   x0 -= 4; T -= 2;
    //   r0 = x0[2] - x1[0]; r1 = x0[0] - x1[2]
    x1_idx = 1;

    while i < N2 - n8 {
        x0_idx -= 4;
        t_idx -= 2;

        let r0 = input[x0_idx + 2] - input[x1_idx];
        let r1 = input[x0_idx + 0] - input[x1_idx + 2];
        w[N2 + i] = r1 * TRIG_2048[t_idx + 1] + r0 * TRIG_2048[t_idx + 0];
        w[N2 + i + 1] = r1 * TRIG_2048[t_idx + 0] - r0 * TRIG_2048[t_idx + 1];

        x1_idx += 4;
        i += 2;
    }

    // Third loop: i=n2-n8..n2 step 2
    //   x0 = in + n (reset)
    //   x0 -= 4; T -= 2;
    //   r0 = -x0[2] - x1[0]; r1 = -x0[0] - x1[2]
    x0_idx = N;

    while i < N2 {
        x0_idx -= 4;
        t_idx -= 2;

        let r0 = -input[x0_idx + 2] - input[x1_idx];
        let r1 = -input[x0_idx + 0] - input[x1_idx + 2];
        w[N2 + i] = r1 * TRIG_2048[t_idx + 1] + r0 * TRIG_2048[t_idx + 0];
        w[N2 + i + 1] = r1 * TRIG_2048[t_idx + 0] - r0 * TRIG_2048[t_idx + 1];

        x1_idx += 4;
        i += 2;
    }

    mdct_butterflies(&mut w, N2, N2);
    mdct_bitreverse(&mut w);

    // rotate + window: final output
    // T = trig + n2 = TRIG_2048[N2]
    // x0 = out + n2 (pointer, decrements by 1)
    // w advances by 2 per iteration, T advances by 2 per iteration
    let mut t_idx: usize = N2;
    let mut w_idx: usize = 0;

    for k in 0..n4 {
        let x0_pos = N2 - 1 - k; // x0-- before use: x0 starts at out+n2, decrements each iter
        output[k] =
            (w[w_idx] * TRIG_2048[t_idx] + w[w_idx + 1] * TRIG_2048[t_idx + 1]) * SCALE_2048;
        output[x0_pos] =
            (w[w_idx] * TRIG_2048[t_idx + 1] - w[w_idx + 1] * TRIG_2048[t_idx]) * SCALE_2048;
        w_idx += 2;
        t_idx += 2;
    }
}
