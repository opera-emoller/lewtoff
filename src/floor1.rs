//! Floor 1 setup, state, and encoder — literal port of libvorbis 1.3.7
//! `lib/floor1.c` (encode path only).
//!
//! Decoder helpers (`floor1_inverse1`, `floor1_inverse2`) are NOT ported;
//! we only need the encode path for lewtoff.

#![allow(clippy::needless_range_loop)]
#![allow(clippy::manual_clamp)]
#![allow(clippy::explicit_counter_loop)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::excessive_precision)]
#![allow(clippy::assign_op_pattern)]
#![allow(non_snake_case)]
#![allow(unused_mut)]
#![allow(unused_variables)]
#![allow(unused_assignments)]

use crate::bitpack::{BitReader, BitWriter};
use crate::codebook::Codebook;

// ---------------------------------------------------------------------------
// Constants (mirroring backends.h / floor1.c)
// ---------------------------------------------------------------------------

const FLOOR1_RANGEDB: i32 = 140;
const VIF_POSIT: usize = 63;
const VIF_CLASS: usize = 16;
const VIF_PARTS: usize = 31;

// ---------------------------------------------------------------------------
// Floor1Setup — mirrors vorbis_info_floor1
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct Floor1Setup {
    pub partitions: i32,
    pub partitionclass: [i32; VIF_PARTS],

    pub class_dim: [i32; VIF_CLASS],
    pub class_subs: [i32; VIF_CLASS],
    pub class_book: [i32; VIF_CLASS],
    pub class_subbook: [[i32; 8]; VIF_CLASS],

    pub mult: i32,
    pub postlist: [i32; VIF_POSIT + 2],

    // encode side analysis parameters
    pub maxover: f32,
    pub maxunder: f32,
    pub maxerr: f32,
    pub twofitweight: f32,
    pub twofitatten: f32,

    pub n: i32,
}

// ---------------------------------------------------------------------------
// Floor1State — mirrors vorbis_look_floor1
// ---------------------------------------------------------------------------

pub(crate) struct Floor1State {
    pub sorted_index: [i32; VIF_POSIT + 2],
    pub forward_index: [i32; VIF_POSIT + 2],
    pub reverse_index: [i32; VIF_POSIT + 2],

    pub hineighbor: [i32; VIF_POSIT],
    pub loneighbor: [i32; VIF_POSIT],
    pub posts: i32,

    pub n: i32,
    pub quant_q: i32,

    // embed a copy of the setup so encode can access vi->postlist etc.
    pub vi: Floor1Setup,

    // statistics (not used for correctness, but libvorbis maintains them)
    pub phrasebits: i64,
    pub postbits: i64,
    pub frames: i64,
}

// ---------------------------------------------------------------------------
// lsfit_acc — mirrors lsfit_acc in floor1.c
// ---------------------------------------------------------------------------

#[derive(Default, Clone, Copy)]
struct LsfitAcc {
    x0: i32,
    x1: i32,

    xa: i32,
    ya: i32,
    x2a: i32,
    y2a: i32,
    xya: i32,
    an: i32,

    xb: i32,
    yb: i32,
    x2b: i32,
    y2b: i32,
    xyb: i32,
    bn: i32,
}

use crate::bitpack::ov_ilog;

// ---------------------------------------------------------------------------
// unpack_floor1 — port of floor1_unpack (info.c calls this via hook)
//
// The caller (unpack_q5_floors) has already consumed codebooks and the
// time-domain placeholder section; this function reads one floor-1 record
// from the bitstream.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) enum Floor1Error {
    BadPartitionClass,
    BadClassBook,
    BadSubbook,
    BadRangebits,
    BadPostValue,
    DuplicatePost,
    BadMult,
}

pub(crate) fn unpack_floor1(
    r: &mut BitReader,
    books_count: usize,
) -> Result<Floor1Setup, Floor1Error> {
    let mut info = Floor1Setup {
        partitions: 0,
        partitionclass: [0i32; VIF_PARTS],
        class_dim: [0i32; VIF_CLASS],
        class_subs: [0i32; VIF_CLASS],
        class_book: [0i32; VIF_CLASS],
        class_subbook: [[0i32; 8]; VIF_CLASS],
        mult: 0,
        postlist: [0i32; VIF_POSIT + 2],
        maxover: 0.0,
        maxunder: 0.0,
        maxerr: 0.0,
        twofitweight: 0.0,
        twofitatten: 0.0,
        n: 0,
    };

    // read partitions
    info.partitions = r.read(5) as i32; // only 0 to 31 legal
    let mut maxclass: i32 = -1;
    for j in 0..info.partitions as usize {
        info.partitionclass[j] = r.read(4) as i32; // only 0 to 15 legal
        if info.partitionclass[j] < 0 {
            return Err(Floor1Error::BadPartitionClass);
        }
        if maxclass < info.partitionclass[j] {
            maxclass = info.partitionclass[j];
        }
    }

    // read partition classes
    for j in 0..(maxclass + 1) as usize {
        info.class_dim[j] = r.read(3) as i32 + 1; // 1 to 8
        info.class_subs[j] = r.read(2) as i32; // 0,1,2,3 bits
        if info.class_subs[j] < 0 {
            return Err(Floor1Error::BadPartitionClass);
        }
        if info.class_subs[j] != 0 {
            info.class_book[j] = r.read(8) as i32;
        }
        if info.class_book[j] < 0 || info.class_book[j] >= books_count as i32 {
            return Err(Floor1Error::BadClassBook);
        }
        for k in 0..(1usize << info.class_subs[j]) {
            info.class_subbook[j][k] = r.read(8) as i32 - 1;
            if info.class_subbook[j][k] < -1 || info.class_subbook[j][k] >= books_count as i32 {
                return Err(Floor1Error::BadSubbook);
            }
        }
    }

    // read the post list
    info.mult = r.read(2) as i32 + 1; // only 1,2,3,4 legal now
    let rangebits = r.read(4) as i32;
    if rangebits < 0 {
        return Err(Floor1Error::BadRangebits);
    }

    let mut count: usize = 0;
    let mut k: usize = 0;
    for j in 0..info.partitions as usize {
        count += info.class_dim[info.partitionclass[j] as usize] as usize;
        if count > VIF_POSIT {
            return Err(Floor1Error::BadPostValue);
        }
        while k < count {
            let t = r.read(rangebits as u32) as i32;
            info.postlist[k + 2] = t;
            if t < 0 || t >= (1i32 << rangebits) {
                return Err(Floor1Error::BadPostValue);
            }
            k += 1;
        }
    }
    info.postlist[0] = 0;
    info.postlist[1] = 1 << rangebits;

    // don't allow repeated values in post list
    {
        let n = count + 2;
        let mut sortpointer: Vec<usize> = (0..n).collect();
        sortpointer.sort_by_key(|&i| info.postlist[i]);
        for j in 1..n {
            if info.postlist[sortpointer[j - 1]] == info.postlist[sortpointer[j]] {
                return Err(Floor1Error::DuplicatePost);
            }
        }
    }

    Ok(info)
}

// ---------------------------------------------------------------------------
// floor1_look — build the per-block lookup struct
// ---------------------------------------------------------------------------

pub(crate) fn floor1_look(info: Floor1Setup) -> Floor1State {
    let mut n: i32 = 0;
    for i in 0..info.partitions as usize {
        n += info.class_dim[info.partitionclass[i] as usize];
    }
    n += 2;
    let posts = n;

    let mut sortpointer: Vec<usize> = (0..posts as usize).collect();
    sortpointer.sort_by_key(|&i| info.postlist[i]);

    let mut forward_index = [0i32; VIF_POSIT + 2];
    let mut reverse_index = [0i32; VIF_POSIT + 2];
    let mut sorted_index = [0i32; VIF_POSIT + 2];

    for i in 0..posts as usize {
        forward_index[i] = sortpointer[i] as i32;
    }
    for i in 0..posts as usize {
        reverse_index[forward_index[i] as usize] = i as i32;
    }
    for i in 0..posts as usize {
        sorted_index[i] = info.postlist[forward_index[i] as usize];
    }

    let quant_q = match info.mult {
        1 => 256,
        2 => 128,
        3 => 86,
        4 => 64,
        _ => 256, // shouldn't happen
    };

    // discover neighbours
    let look_n = info.postlist[1];
    let mut hineighbor = [0i32; VIF_POSIT];
    let mut loneighbor = [0i32; VIF_POSIT];

    for i in 0..(posts - 2) as usize {
        let mut lo: i32 = 0;
        let mut hi: i32 = 1;
        let mut lx: i32 = 0;
        let mut hx: i32 = look_n;
        let currentx = info.postlist[i + 2];
        for j in 0..i + 2 {
            let x = info.postlist[j];
            if x > lx && x < currentx {
                lo = j as i32;
                lx = x;
            }
            if x < hx && x > currentx {
                hi = j as i32;
                hx = x;
            }
        }
        loneighbor[i] = lo;
        hineighbor[i] = hi;
    }

    Floor1State {
        sorted_index,
        forward_index,
        reverse_index,
        hineighbor,
        loneighbor,
        posts,
        n: look_n,
        quant_q,
        vi: info,
        phrasebits: 0,
        postbits: 0,
        frames: 0,
    }
}

// ---------------------------------------------------------------------------
// FLOOR1_fromdB_LOOKUP — see crate::tables::lookup::FLOOR1_FROMDB_LOOKUP.
// ---------------------------------------------------------------------------

use crate::tables::lookup::FLOOR1_FROMDB_LOOKUP as FLOOR1_fromdB_LOOKUP;

// ---------------------------------------------------------------------------
// render_point — port of render_point in floor1.c
// ---------------------------------------------------------------------------

pub(crate) fn render_point(x0: i32, x1: i32, y0: i32, y1: i32, x: i32) -> i32 {
    let y0 = y0 & 0x7fff; // mask off flag
    let y1 = y1 & 0x7fff;

    let dy = y1 - y0;
    let adx = x1 - x0;
    let ady = dy.abs();
    let err = ady * (x - x0);

    let off = err / adx;
    if dy < 0 {
        y0 - off
    } else {
        y0 + off
    }
}

// ---------------------------------------------------------------------------
// render_line — port of render_line in floor1.c (multiply path)
// ---------------------------------------------------------------------------

pub(crate) fn render_line(n: i32, x0: i32, x1: i32, y0: i32, y1: i32, d: &mut [f32]) {
    let dy = y1 - y0;
    let adx = x1 - x0;
    let ady = dy.abs();
    let base = dy / adx;
    let sy = if dy < 0 { base - 1 } else { base + 1 };
    let mut x = x0;
    let mut y = y0;
    let mut err: i32 = 0;
    let mut ady = ady - (base * adx).abs();

    let mut n = n;
    if n > x1 {
        n = x1;
    }

    if x < n {
        d[x as usize] *= FLOOR1_fromdB_LOOKUP[y as usize];
    }

    loop {
        x += 1;
        if x >= n {
            break;
        }
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base;
        }
        d[x as usize] *= FLOOR1_fromdB_LOOKUP[y as usize];
    }
}

// ---------------------------------------------------------------------------
// render_line0 — port of render_line0 in floor1.c (integer assign path)
// ---------------------------------------------------------------------------

fn render_line0(n: i32, x0: i32, x1: i32, y0: i32, y1: i32, d: &mut [i32]) {
    let dy = y1 - y0;
    let adx = x1 - x0;
    let ady = dy.abs();
    let base = dy / adx;
    let sy = if dy < 0 { base - 1 } else { base + 1 };
    let mut x = x0;
    let mut y = y0;
    let mut err: i32 = 0;
    let ady = ady - (base * adx).abs();

    let mut n = n;
    if n > x1 {
        n = x1;
    }

    if x < n {
        d[x as usize] = y;
    }

    loop {
        x += 1;
        if x >= n {
            break;
        }
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base;
        }
        d[x as usize] = y;
    }
}

// ---------------------------------------------------------------------------
// vorbis_dBquant — port of vorbis_dBquant in floor1.c
// ---------------------------------------------------------------------------

fn vorbis_dBquant(x: f32) -> i32 {
    let i = x * 7.3142857_f32 + 1023.5_f32;
    let i = i as i32;
    if i > 1023 {
        return 1023;
    }
    if i < 0 {
        return 0;
    }
    i
}

// ---------------------------------------------------------------------------
// accumulate_fit — port of accumulate_fit in floor1.c
// ---------------------------------------------------------------------------

fn accumulate_fit(
    flr: &[f32],
    mdct: &[f32],
    x0: i32,
    x1: i32,
    a: &mut LsfitAcc,
    n: i32,
    info: &Floor1Setup,
) -> i32 {
    let mut xa = 0i32;
    let mut ya = 0i32;
    let mut x2a = 0i32;
    let mut y2a = 0i32;
    let mut xya = 0i32;
    let mut na = 0i32;
    let mut xb = 0i32;
    let mut yb = 0i32;
    let mut x2b = 0i32;
    let mut y2b = 0i32;
    let mut xyb = 0i32;
    let mut nb = 0i32;

    *a = LsfitAcc::default();
    a.x0 = x0;
    a.x1 = x1;
    let x1 = if x1 >= n { n - 1 } else { x1 };

    let mut i = x0;
    while i <= x1 {
        let quantized = vorbis_dBquant(flr[i as usize]);
        if quantized != 0 {
            if mdct[i as usize] + info.twofitatten >= flr[i as usize] {
                xa += i;
                ya += quantized;
                x2a += i * i;
                y2a += quantized * quantized;
                xya += i * quantized;
                na += 1;
            } else {
                xb += i;
                yb += quantized;
                x2b += i * i;
                y2b += quantized * quantized;
                xyb += i * quantized;
                nb += 1;
            }
        }
        i += 1;
    }

    a.xa = xa;
    a.ya = ya;
    a.x2a = x2a;
    a.y2a = y2a;
    a.xya = xya;
    a.an = na;

    a.xb = xb;
    a.yb = yb;
    a.x2b = x2b;
    a.y2b = y2b;
    a.xyb = xyb;
    a.bn = nb;

    na
}

// ---------------------------------------------------------------------------
// fit_line — port of fit_line in floor1.c
// ---------------------------------------------------------------------------

fn fit_line(a: &[LsfitAcc], fits: usize, y0: &mut i32, y1: &mut i32, info: &Floor1Setup) -> i32 {
    let mut xb: f64 = 0.0;
    let mut yb: f64 = 0.0;
    let mut x2b: f64 = 0.0;
    let mut y2b: f64 = 0.0;
    let mut xyb: f64 = 0.0;
    let mut bn: f64 = 0.0;

    let x0 = a[0].x0;
    let x1 = a[fits - 1].x1;

    for i in 0..fits {
        // libvorbis: `double weight = (a[i].bn+a[i].an) * info->twofitweight
        //   / (a[i].an+1) + 1.;`
        // info->twofitweight is float, so the int*float multiplication and the
        // float/int division both happen in f32. Only the final +1. promotes
        // the result to f64. Mirror that ordering exactly.
        let weight_f32 = (a[i].bn + a[i].an) as f32 * info.twofitweight / (a[i].an + 1) as f32;
        let weight = weight_f32 as f64 + 1.0_f64;

        xb += a[i].xb as f64 + a[i].xa as f64 * weight;
        yb += a[i].yb as f64 + a[i].ya as f64 * weight;
        x2b += a[i].x2b as f64 + a[i].x2a as f64 * weight;
        y2b += a[i].y2b as f64 + a[i].y2a as f64 * weight;
        xyb += a[i].xyb as f64 + a[i].xya as f64 * weight;
        bn += a[i].bn as f64 + a[i].an as f64 * weight;
    }

    if std::env::var("LW_DEBUG_FITLINE").is_ok() && fits >= 20 {
        eprintln!(
            "LW_FITLINE fits={} x0={} x1={}: xb={:.1} yb={:.1} x2b={:.1} xyb={:.1} bn={:.1}",
            fits, x0, x1, xb, yb, x2b, xyb, bn
        );
    }

    if *y0 >= 0 {
        xb += x0 as f64;
        yb += *y0 as f64;
        x2b += (x0 * x0) as f64;
        y2b += (*y0 * *y0) as f64;
        xyb += (*y0 * x0) as f64;
        bn += 1.0_f64;
    }

    if *y1 >= 0 {
        xb += x1 as f64;
        yb += *y1 as f64;
        x2b += (x1 * x1) as f64;
        y2b += (*y1 * *y1) as f64;
        xyb += (*y1 * x1) as f64;
        bn += 1.0_f64;
    }

    {
        let denom = bn * x2b - xb * xb;

        if denom > 0.0_f64 {
            let a_coeff = (yb * x2b - xyb * xb) / denom;
            let b_coeff = (bn * xyb - xb * yb) / denom;
            // C's rint() rounds half-to-even (banker's rounding); f64::round() in
            // Rust rounds half-away-from-zero. Use round_ties_even to match C.
            *y0 = (a_coeff + b_coeff * x0 as f64).round_ties_even() as i32;
            *y1 = (a_coeff + b_coeff * x1 as f64).round_ties_even() as i32;

            // limit to our range!
            if *y0 > 1023 {
                *y0 = 1023;
            }
            if *y1 > 1023 {
                *y1 = 1023;
            }
            if *y0 < 0 {
                *y0 = 0;
            }
            if *y1 < 0 {
                *y1 = 0;
            }

            0
        } else {
            *y0 = 0;
            *y1 = 0;
            1
        }
    }
}

// ---------------------------------------------------------------------------
// inspect_error — port of inspect_error in floor1.c
// ---------------------------------------------------------------------------

fn inspect_error(
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    mask: &[f32],
    mdct: &[f32],
    info: &Floor1Setup,
) -> i32 {
    let dy = y1 - y0;
    let adx = x1 - x0;
    let ady = dy.abs();
    let base = dy / adx;
    let sy = if dy < 0 { base - 1 } else { base + 1 };
    let mut x = x0;
    let mut y = y0;
    let mut err: i32 = 0;
    let val = vorbis_dBquant(mask[x as usize]);
    let mut mse: i32 = 0;
    let mut n: i32 = 0;

    let ady = ady - (base * adx).abs();

    mse = y - val;
    mse *= mse;
    n += 1;
    if mdct[x as usize] + info.twofitatten >= mask[x as usize] {
        if y + (info.maxover as i32) < val {
            return 1;
        }
        if y - (info.maxunder as i32) > val {
            return 1;
        }
    }

    loop {
        x += 1;
        if x >= x1 {
            break;
        }
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base;
        }

        let val = vorbis_dBquant(mask[x as usize]);
        mse += (y - val) * (y - val);
        n += 1;
        if mdct[x as usize] + info.twofitatten >= mask[x as usize] {
            if val != 0 {
                if y + (info.maxover as i32) < val {
                    return 1;
                }
                if y - (info.maxunder as i32) > val {
                    return 1;
                }
            }
        }
    }

    if info.maxover as i32 * info.maxover as i32 / n > info.maxerr as i32 {
        return 0;
    }
    if info.maxunder as i32 * info.maxunder as i32 / n > info.maxerr as i32 {
        return 0;
    }
    if mse / n > info.maxerr as i32 {
        return 1;
    }
    0
}

// ---------------------------------------------------------------------------
// post_Y — port of post_Y in floor1.c
// ---------------------------------------------------------------------------

fn post_Y(a: &[i32], b: &[i32], pos: usize) -> i32 {
    if a[pos] < 0 {
        return b[pos];
    }
    if b[pos] < 0 {
        return a[pos];
    }
    (a[pos] + b[pos]) >> 1
}

// ---------------------------------------------------------------------------
// floor1_fit — port of floor1_fit in floor1.c
// ---------------------------------------------------------------------------

pub(crate) fn floor1_fit(look: &Floor1State, logmdct: &[f32], logmask: &[f32]) -> Option<Vec<i32>> {
    let info = &look.vi;
    let n = look.n;
    let posts = look.posts;
    let mut nonzero: i32 = 0;
    let mut fits = vec![LsfitAcc::default(); VIF_POSIT + 1];
    let mut fit_valueA = [-200i32; VIF_POSIT + 2];
    let mut fit_valueB = [-200i32; VIF_POSIT + 2];

    let mut loneighbor = [0i32; VIF_POSIT + 2];
    let mut hineighbor = [1i32; VIF_POSIT + 2];
    let mut memo = [-1i32; VIF_POSIT + 2];

    // quantize the relevant floor points and collect them into line fit
    // structures (one per minimal division) at the same time
    if posts == 0 {
        nonzero += accumulate_fit(logmask, logmdct, 0, n, &mut fits[0], n, info);
    } else {
        for i in 0..(posts - 1) as usize {
            nonzero += accumulate_fit(
                logmask,
                logmdct,
                look.sorted_index[i],
                look.sorted_index[i + 1],
                &mut fits[i],
                n,
                info,
            );
        }
    }

    if nonzero != 0 {
        // start by fitting the implicit base case....
        let mut y0 = -200i32;
        let mut y1 = -200i32;
        fit_line(&fits, (posts - 1) as usize, &mut y0, &mut y1, info);

        if std::env::var("LW_DEBUG_FLOOR1FIT").is_ok() {
            eprintln!("LW_FLOOR1FIT base: y0={} y1={} n={}", y0, y1, n);
        }

        fit_valueA[0] = y0;
        fit_valueB[0] = y0;
        fit_valueB[1] = y1;
        fit_valueA[1] = y1;

        // Non degenerate case
        // start progressive splitting.  This is a greedy, non-optimal
        // algorithm, but simple and close enough to the best answer.
        for i in 2..posts as usize {
            let sortpos = look.reverse_index[i] as usize;
            let ln = loneighbor[sortpos] as usize;
            let hn = hineighbor[sortpos] as usize;

            // eliminate repeat searches of a particular range with a memo
            if memo[ln] != hn as i32 {
                // haven't performed this error search yet
                let lsortpos = look.reverse_index[ln] as usize;
                let hsortpos = look.reverse_index[hn] as usize;
                memo[ln] = hn as i32;

                {
                    // A note: we want to bound/minimize *local*, not global, error
                    let lx = info.postlist[ln];
                    let hx = info.postlist[hn];
                    let ly = post_Y(&fit_valueA, &fit_valueB, ln);
                    let hy = post_Y(&fit_valueA, &fit_valueB, hn);

                    if ly == -1 || hy == -1 {
                        // This mirrors the C `exit(1)` — in practice shouldn't happen
                        // with valid input, but we match the control flow literally.
                        panic!("floor1_fit: post_Y returned -1 (internal error)");
                    }

                    if inspect_error(lx, hx, ly, hy, logmask, logmdct, info) != 0 {
                        // outside error bounds/begin search area.  Split it.
                        let mut ly0 = -200i32;
                        let mut ly1 = -200i32;
                        let mut hy0 = -200i32;
                        let mut hy1 = -200i32;
                        let ret0 = fit_line(
                            &fits[lsortpos..],
                            sortpos - lsortpos,
                            &mut ly0,
                            &mut ly1,
                            info,
                        );
                        let ret1 = fit_line(
                            &fits[sortpos..],
                            hsortpos - sortpos,
                            &mut hy0,
                            &mut hy1,
                            info,
                        );

                        if ret0 != 0 {
                            ly0 = ly;
                            ly1 = hy0;
                        }
                        if ret1 != 0 {
                            hy0 = ly1;
                            hy1 = hy;
                        }

                        if ret0 != 0 && ret1 != 0 {
                            fit_valueA[i] = -200;
                            fit_valueB[i] = -200;
                        } else {
                            // store new edge values
                            fit_valueB[ln] = ly0;
                            if ln == 0 {
                                fit_valueA[ln] = ly0;
                            }
                            fit_valueA[i] = ly1;
                            fit_valueB[i] = hy0;
                            fit_valueA[hn] = hy1;
                            if hn == 1 {
                                fit_valueB[hn] = hy1;
                            }

                            if ly1 >= 0 || hy0 >= 0 {
                                // store new neighbor values
                                let mut j = sortpos as i32 - 1;
                                while j >= 0 {
                                    if hineighbor[j as usize] == hn as i32 {
                                        hineighbor[j as usize] = i as i32;
                                    } else {
                                        break;
                                    }
                                    j -= 1;
                                }
                                let mut j = sortpos + 1;
                                while j < posts as usize {
                                    if loneighbor[j] == ln as i32 {
                                        loneighbor[j] = i as i32;
                                    } else {
                                        break;
                                    }
                                    j += 1;
                                }
                            }
                        }
                    } else {
                        fit_valueA[i] = -200;
                        fit_valueB[i] = -200;
                    }
                }
            }
        }

        let mut output = vec![0i32; posts as usize];

        output[0] = post_Y(&fit_valueA, &fit_valueB, 0);
        output[1] = post_Y(&fit_valueA, &fit_valueB, 1);

        // fill in posts marked as not using a fit; we will zero
        // back out to 'unused' when encoding them so long as curve
        // interpolation doesn't force them into use
        for i in 2..posts as usize {
            let ln = look.loneighbor[i - 2] as usize;
            let hn = look.hineighbor[i - 2] as usize;
            let x0 = info.postlist[ln];
            let x1 = info.postlist[hn];
            let y0 = output[ln];
            let y1 = output[hn];

            let predicted = render_point(x0, x1, y0, y1, info.postlist[i]);
            let vx = post_Y(&fit_valueA, &fit_valueB, i);

            if vx >= 0 && predicted != vx {
                output[i] = vx;
            } else {
                output[i] = predicted | 0x8000;
            }
        }

        Some(output)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// floor1_encode — port of floor1_encode in floor1.c
//
// Inputs:
//   opb     — BitWriter to emit into
//   look    — mutable look (we update phrasebits/postbits/frames)
//   post    — fitted post array from floor1_fit (mutable, values modified)
//   ilogmask — output: integer log-mask array (length = pcmend/2)
//   books   — full codebook slice (same order as setup)
//   pcmend  — block size (long block = 2048, so pcmend/2 = 1024)
//   w_val   — long block flag (0 = short, 1 = long); maps to ci->blocksizes[vb->W]
//
// Returns 1 (nontrivial floor) or 0 (zero floor / post==None).
// ---------------------------------------------------------------------------

pub(crate) fn floor1_encode(
    opb: &mut BitWriter,
    look: &mut Floor1State,
    post: Option<&mut Vec<i32>>,
    ilogmask: &mut Vec<i32>,
    books: &[Codebook],
    pcmend: usize,
) -> i32 {
    let info = &look.vi.clone();
    let posts = look.posts;

    if let Some(post) = post {
        // quantize values to multiplier spec
        for i in 0..posts as usize {
            let mut val = post[i] & 0x7fff;
            match info.mult {
                1 => {
                    val >>= 2;
                }
                2 => {
                    val >>= 3;
                }
                3 => {
                    val /= 12;
                }
                4 => {
                    val >>= 4;
                }
                _ => {}
            }
            post[i] = val | (post[i] & 0x8000);
        }

        let mut out = [0i32; VIF_POSIT + 2];
        out[0] = post[0];
        out[1] = post[1];

        // find prediction values for each post and subtract them
        for i in 2..posts as usize {
            let ln = look.loneighbor[i - 2] as usize;
            let hn = look.hineighbor[i - 2] as usize;
            let x0 = info.postlist[ln];
            let x1 = info.postlist[hn];
            let y0 = post[ln];
            let y1 = post[hn];

            let predicted = render_point(x0, x1, y0, y1, info.postlist[i]);

            if (post[i] & 0x8000) != 0 || predicted == post[i] {
                post[i] = predicted | 0x8000; // in case there was roundoff jitter
                                              // in interpolation
                out[i] = 0;
            } else {
                let headroom = if look.quant_q - predicted < predicted {
                    look.quant_q - predicted
                } else {
                    predicted
                };

                let mut val = post[i] - predicted;

                // at this point the 'deviation' value is in the range +/- max
                // range, but the real, unique range can always be mapped to
                // only [0-maxrange).  So we want to wrap the deviation into
                // this limited range, but do it in the way that least screws
                // an essentially gaussian probability distribution.

                if val < 0 {
                    if val < -headroom {
                        val = headroom - val - 1;
                    } else {
                        val = -1 - (val * 2);
                    }
                } else {
                    if val >= headroom {
                        val = val + headroom;
                    } else {
                        val <<= 1;
                    }
                }

                out[i] = val;
                post[ln] &= 0x7fff;
                post[hn] &= 0x7fff;
            }
        }

        if std::env::var("LW_DEBUG_FLOOR_PART").is_ok() {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static FIRED: AtomicUsize = AtomicUsize::new(0);
            let n = FIRED.fetch_add(1, Ordering::Relaxed);
            if n < 10 {
                let mut s = format!("R_FLOOR_OUT posts={}:", posts);
                for z in 0..posts as usize {
                    s.push_str(&format!(" {}", out[z]));
                }
                eprintln!("{}", s);
                let mut s = format!("R_FLOOR_POSTQ posts={}:", posts);
                for z in 0..posts as usize {
                    s.push_str(&format!(" {}", post[z]));
                }
                eprintln!("{}", s);
            }
        }

        // we have everything we need. pack it out
        // mark nontrivial floor
        opb.write(1, 1);

        // beginning/end post
        look.frames += 1;
        let quant_bits = ov_ilog((look.quant_q - 1) as u32);
        look.postbits += (quant_bits * 2) as i64;
        opb.write(out[0] as u32, quant_bits);
        opb.write(out[1] as u32, quant_bits);

        // partition by partition
        let mut j: usize = 2;
        for i in 0..info.partitions as usize {
            let class = info.partitionclass[i] as usize;
            let cdim = info.class_dim[class] as usize;
            let csubbits = info.class_subs[class] as usize;
            let csub = 1usize << csubbits;
            let mut bookas = [0usize; 8];
            let mut cval: usize = 0;
            let mut cshift: usize = 0;

            let part_dbg = std::env::var("LW_DEBUG_FLOOR_PART").is_ok();
            if part_dbg {
                use std::sync::atomic::{AtomicUsize, Ordering};
                static N: AtomicUsize = AtomicUsize::new(0);
                let n = N.fetch_add(1, Ordering::Relaxed);
                if n < 200 {
                    eprintln!(
                        "R_FLOOR_PART i={} class={} cdim={} csubbits={}",
                        i, class, cdim, csubbits
                    );
                }
            }

            // generate the partition's first stage cascade value
            if csubbits != 0 {
                let mut maxval = [0usize; 8];
                for k in 0..csub {
                    let booknum = info.class_subbook[class][k];
                    if booknum < 0 {
                        maxval[k] = 1;
                    } else {
                        maxval[k] = books[booknum as usize].entries;
                    }
                }
                for k in 0..cdim {
                    for l in 0..csub {
                        let val = out[j + k];
                        if val < maxval[l] as i32 {
                            bookas[k] = l;
                            break;
                        }
                    }
                    cval |= bookas[k] << cshift;
                    cshift += csubbits;
                }
                if part_dbg {
                    use std::sync::atomic::{AtomicUsize, Ordering};
                    static N: AtomicUsize = AtomicUsize::new(0);
                    let n = N.fetch_add(1, Ordering::Relaxed);
                    if n < 200 {
                        eprintln!(
                            "R_FLOOR_CVAL i={} class={} cval={} class_book={}",
                            i, class, cval, info.class_book[class]
                        );
                    }
                }
                // write it
                let phrase_bits = books[info.class_book[class] as usize].encode(cval, opb);
                look.phrasebits += phrase_bits as i64;
            }

            // write post values
            for k in 0..cdim {
                let book = info.class_subbook[class][bookas[k]];
                if book >= 0 {
                    let book = book as usize;
                    if part_dbg {
                        use std::sync::atomic::{AtomicUsize, Ordering};
                        static N: AtomicUsize = AtomicUsize::new(0);
                        let n = N.fetch_add(1, Ordering::Relaxed);
                        if n < 400 {
                            eprintln!(
                                "R_FLOOR_POSTV i={} k={} cdim={} j+k={} out={} book={} entries={}",
                                i,
                                k,
                                cdim,
                                j + k,
                                out[j + k],
                                book,
                                books[book].entries
                            );
                        }
                    }
                    // hack to allow training with 'bad' books
                    if out[j + k] < books[book].entries as i32 {
                        if part_dbg {
                            let entry = out[j + k] as usize;
                            let cw = books[book].codewords[entry];
                            let cl = books[book].codeword_lengths[entry];
                            use std::sync::atomic::{AtomicUsize, Ordering};
                            static N: AtomicUsize = AtomicUsize::new(0);
                            let n = N.fetch_add(1, Ordering::Relaxed);
                            if n < 8 {
                                eprintln!(
                                    "R_POSTV_CW i={} k={} book={} entry={} codeword=0x{:08x} len={}",
                                    i, k, book, entry, cw, cl
                                );
                            }
                        }
                        let post_bits = books[book].encode(out[j + k] as usize, opb);
                        look.postbits += post_bits as i64;
                    }
                }
            }
            j += cdim;
        }

        {
            // generate quantized floor equivalent to what we'd unpack in decode
            // render the lines
            let mut hx: i32 = 0;
            let mut lx: i32 = 0;
            let mut ly = post[0] * info.mult;
            let n = pcmend / 2;

            // ensure ilogmask is large enough
            if ilogmask.len() < n {
                ilogmask.resize(n, 0);
            }

            for jj in 1..look.posts as usize {
                let current = look.forward_index[jj] as usize;
                let hy = post[current] & 0x7fff;
                if hy == post[current] {
                    let hy = hy * info.mult;
                    hx = info.postlist[current];

                    render_line0(n as i32, lx, hx, ly, hy, ilogmask);

                    lx = hx;
                    ly = hy;
                }
            }
            // be certain
            for jj in hx as usize..pcmend / 2 {
                ilogmask[jj] = ly;
            }
        }

        1
    } else {
        opb.write(0, 1);
        let n = pcmend / 2;
        if ilogmask.len() < n {
            ilogmask.resize(n, 0);
        }
        for v in ilogmask[..n].iter_mut() {
            *v = 0;
        }
        0
    }
}

// ---------------------------------------------------------------------------
// unpack_q5_floors — parse the floor1 configs from the setup blob.
//
// The setup blob order is: codebooks → time-domain → floors → ...
// The caller must have already consumed the codebook section (and the 7-byte
// header).  This function takes a BitReader positioned just after the
// codebooks, consumes the time-domain placeholder, then reads the floor
// configs.
// ---------------------------------------------------------------------------

pub(crate) fn unpack_q5_floors(
    r: &mut BitReader,
    books_count: usize,
) -> Result<Vec<Floor1Setup>, Floor1Error> {
    // time backend settings (hooks are unused — just consume the bits)
    // times = oggpack_read(opb,6)+1
    let times = r.read(6) as usize + 1;
    for _ in 0..times {
        // each entry is a 16-bit floor type (always 0 for time-domain)
        let _test = r.read(16);
    }

    // floor backend settings
    let floors = r.read(6) as usize + 1;
    let mut result = Vec::with_capacity(floors);
    for _ in 0..floors {
        let floor_type = r.read(16);
        if floor_type != 1 {
            // We only support floor type 1 (the only type used in practice)
            // For type 0, we'd need a different parser — just return error for now.
            return Err(Floor1Error::BadPartitionClass);
        }
        result.push(unpack_floor1(r, books_count)?);
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitpack::BitReader;
    use crate::setup_blob::Q5_SETUP_BLOB;

    // -----------------------------------------------------------------------
    // render_point tests — hand-computed from spec
    // -----------------------------------------------------------------------

    #[test]
    fn render_point_horizontal_line() {
        // y0==y1 → all points return y0
        assert_eq!(render_point(0, 10, 5, 5, 5), 5);
        assert_eq!(render_point(0, 10, 5, 5, 3), 5);
        assert_eq!(render_point(0, 10, 5, 5, 0), 5);
    }

    #[test]
    fn render_point_rising_line() {
        // x0=0 x1=4 y0=0 y1=4, at x=2: dy=4 adx=4 ady=4 err=4*2=8 off=8/4=2 → 0+2=2
        assert_eq!(render_point(0, 4, 0, 4, 2), 2);
        // at x=1: err=4*1=4 off=4/4=1 → 1
        assert_eq!(render_point(0, 4, 0, 4, 1), 1);
    }

    #[test]
    fn render_point_falling_line() {
        // x0=0 x1=4 y0=4 y1=0, at x=2: dy=-4 adx=4 ady=4 err=4*2=8 off=2 → 4-2=2
        assert_eq!(render_point(0, 4, 4, 0, 2), 2);
        // at x=1: off=1 → 4-1=3
        assert_eq!(render_point(0, 4, 4, 0, 1), 3);
    }

    #[test]
    fn render_point_masks_flag_bits() {
        // The flag bit 0x8000 should be masked off before arithmetic
        // y0=5|0x8000, y1=5 — same as y0=5, y1=5
        assert_eq!(render_point(0, 10, 5 | 0x8000, 5, 5), 5);
    }

    // -----------------------------------------------------------------------
    // render_line tests
    // -----------------------------------------------------------------------

    #[test]
    fn render_line_horizontal_multiplies_by_lookup() {
        // A horizontal line at y=255 (last entry of table = 1.0) should be a no-op
        let mut d = vec![2.0f32; 10];
        render_line(10, 0, 10, 255, 255, &mut d);
        for v in &d {
            assert!((*v - 2.0).abs() < 1e-5, "expected 2.0, got {v}");
        }
    }

    #[test]
    fn render_line_only_fills_up_to_x1() {
        // render_line should stop at min(n, x1)
        let mut d = vec![1.0f32; 10];
        render_line(10, 0, 5, 255, 255, &mut d);
        // Positions 5..10 should be untouched (still 1.0)
        for i in 5..10usize {
            assert!((d[i] - 1.0).abs() < 1e-5, "d[{i}] = {} should be 1.0", d[i]);
        }
    }

    // -----------------------------------------------------------------------
    // vorbis_dBquant tests
    // -----------------------------------------------------------------------

    #[test]
    fn dBquant_clamps_correctly() {
        // At x = 0.0: 0.0*7.3142857 + 1023.5 = 1023.5 → 1023
        assert_eq!(vorbis_dBquant(0.0), 1023);
        // At very large negative value should clamp to 0
        assert_eq!(vorbis_dBquant(-1000.0), 0);
        // At very large positive value should clamp to 1023
        assert_eq!(vorbis_dBquant(1000.0), 1023);
    }

    // -----------------------------------------------------------------------
    // Q5 setup blob floor1 round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn q5_blob_floor1_unpack_roundtrip() {
        // Parse codebooks first (they must be consumed before floors)
        let blob = Q5_SETUP_BLOB;
        assert_eq!(&blob[0..7], b"\x05vorbis");
        let mut r = BitReader::new(&blob[7..]);

        // codebooks
        let count = r.read(8) as usize + 1;
        for _ in 0..count {
            crate::codebook::unpack_codebook(&mut r).expect("codebook unpack failed");
        }

        // floors
        let floors = unpack_q5_floors(&mut r, count).expect("floor1 unpack failed");

        assert!(
            !floors.is_empty(),
            "expected at least one floor1 config, got 0"
        );
        for (i, f) in floors.iter().enumerate() {
            assert!(
                f.postlist[1] > 0,
                "floor {i}: postlist[1] (n) should be positive, got {}",
                f.postlist[1]
            );
            assert!(
                f.partitions > 0,
                "floor {i}: partitions should be > 0, got {}",
                f.partitions
            );
            // Q5 floor1 typically has ~30 posts (including the two implicit ones)
            let mut post_count = 0usize;
            for pi in 0..f.partitions as usize {
                post_count += f.class_dim[f.partitionclass[pi] as usize] as usize;
            }
            post_count += 2; // implicit posts at 0 and n
            assert!(
                post_count >= 10,
                "floor {i}: expected >=10 posts, got {post_count}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // floor1_look round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn q5_floor1_look_builds_correctly() {
        let blob = Q5_SETUP_BLOB;
        let mut r = BitReader::new(&blob[7..]);

        let count = r.read(8) as usize + 1;
        for _ in 0..count {
            crate::codebook::unpack_codebook(&mut r).expect("codebook");
        }

        let floors = unpack_q5_floors(&mut r, count).expect("floors");
        let setup = floors.into_iter().next().expect("at least one floor");
        let state = floor1_look(setup);

        assert!(state.posts > 0);
        assert!(state.n > 0);
        assert!(state.quant_q > 0);
    }

    #[test]
    fn q5_floor1_postlist_print() {
        let blob = Q5_SETUP_BLOB;
        let mut r = BitReader::new(&blob[7..]);

        let count = r.read(8) as usize + 1;
        for _ in 0..count {
            crate::codebook::unpack_codebook(&mut r).expect("codebook");
        }

        let floors = unpack_q5_floors(&mut r, count).expect("floors");
        for (fi, setup) in floors.iter().enumerate() {
            let total_posts: usize = (0..setup.partitions as usize)
                .map(|i| setup.class_dim[setup.partitionclass[i] as usize] as usize)
                .sum::<usize>()
                + 2;
            let state = floor1_look(setup.clone());
            let postlist: Vec<i32> = (0..total_posts).map(|i| state.vi.postlist[i]).collect();
            eprintln!(
                "floor {fi}: posts={total_posts} mult={} n={} quant_q={}",
                setup.mult, setup.postlist[1], state.quant_q
            );
            eprintln!("  postlist={:?}", postlist);
            eprintln!(
                "  partitions={} partitionclass={:?}",
                setup.partitions,
                &setup.partitionclass[..setup.partitions as usize]
            );
            eprintln!(
                "  class_dim={:?}",
                &setup.class_dim[..(setup.partitions as usize).min(16)]
            );
            eprintln!(
                "  class_subs={:?}",
                &setup.class_subs[..(setup.partitions as usize).min(16)]
            );
            eprintln!(
                "  class_book={:?}",
                &setup.class_book[..(setup.partitions as usize).min(16)]
            );
            for k in 0..(setup.partitions as usize).min(16) {
                let cls = setup.partitionclass[k] as usize;
                let cdim = setup.class_dim[cls] as usize;
                let csubs = setup.class_subs[cls];
                eprintln!(
                    "  partition[{k}]: class={cls} cdim={cdim} csubs={csubs} subbooks={:?}",
                    &setup.class_subbook[cls][..1 << csubs]
                );
            }
            eprintln!("  sorted_index={:?}", &state.sorted_index[..total_posts]);
        }
    }

    // -----------------------------------------------------------------------
    // accumulate_fit on a synthetic flat spectrum
    // -----------------------------------------------------------------------

    #[test]
    fn accumulate_fit_flat_spectrum() {
        // A flat spectrum at -20 dB: all flr values the same, all mdct well below flr
        let n = 32i32;
        let flr_val = -20.0f32;
        let mdct_val = flr_val - 10.0f32; // well below mask → goes into "b" bucket
        let flr: Vec<f32> = vec![flr_val; n as usize];
        let mdct: Vec<f32> = vec![mdct_val; n as usize];

        let info = Floor1Setup {
            partitions: 0,
            partitionclass: [0; VIF_PARTS],
            class_dim: [1; VIF_CLASS],
            class_subs: [0; VIF_CLASS],
            class_book: [0; VIF_CLASS],
            class_subbook: [[0; 8]; VIF_CLASS],
            mult: 1,
            postlist: {
                let mut p = [0i32; VIF_POSIT + 2];
                p[1] = n;
                p
            },
            maxover: 4.0,
            maxunder: 4.0,
            maxerr: 100.0,
            twofitweight: 10.0,
            twofitatten: 4.0,
            n,
        };

        let mut a = LsfitAcc::default();
        let na = accumulate_fit(&flr, &mdct, 0, n - 1, &mut a, n, &info);

        // All samples have mdct + twofitatten < flr → should be in bucket b
        // (quantized flr_val = vorbis_dBquant(-20.0) which is nonzero)
        assert_eq!(a.x0, 0);
        assert_eq!(a.x1, n - 1);
        let q = vorbis_dBquant(flr_val);
        assert!(q > 0, "expected nonzero quantized floor");
        // na should be 0 (nothing in bucket a)
        assert_eq!(na, 0);
        // bn > 0
        assert!(a.bn > 0);
    }
}
