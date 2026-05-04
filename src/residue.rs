//! Residue 0/1/2 setup, classification, and encoder — literal port of
//! libvorbis 1.3.7 `lib/res0.c` (encode path only).
//!
//! Decoder helpers (`res0_inverse`, `res2_inverse`, `_01inverse`) are NOT
//! ported; lewtoff is encode-only.

#![allow(clippy::needless_range_loop)]
#![allow(clippy::explicit_counter_loop)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::assign_op_pattern)]
#![allow(clippy::absurd_extreme_comparisons)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::ptr_arg)]
#![allow(non_snake_case)]
#![allow(unused_mut)]
#![allow(unused_variables)]
#![allow(unused_assignments)]

use crate::bitpack::{BitReader, BitWriter};
use crate::codebook::Codebook;

use crate::bitpack::ov_ilog;

// ---------------------------------------------------------------------------
// icount — port of icount in res0.c (popcount)
// ---------------------------------------------------------------------------

fn icount(mut v: u32) -> u32 {
    let mut ret = 0u32;
    while v != 0 {
        ret += v & 1;
        v >>= 1;
    }
    ret
}

// ---------------------------------------------------------------------------
// ResidueSetup — mirrors vorbis_info_residue0
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct ResidueSetup {
    pub begin: i64,
    pub end: i64,

    pub grouping: i32,
    pub partitions: i32,
    pub partvals: i32,
    pub groupbook: i32,
    pub secondstages: [i32; 64],
    pub booklist: [i32; 512],

    pub classmetric1: [i32; 64],
    pub classmetric2: [i32; 64],
}

impl ResidueSetup {
    fn zeroed() -> Self {
        ResidueSetup {
            begin: 0,
            end: 0,
            grouping: 0,
            partitions: 0,
            partvals: 0,
            groupbook: 0,
            secondstages: [0; 64],
            booklist: [0; 512],
            classmetric1: [0; 64],
            classmetric2: [-1; 64],
        }
    }
}

// ---------------------------------------------------------------------------
// ResidueLook — mirrors vorbis_look_residue0 (runtime derived from setup)
// ---------------------------------------------------------------------------

pub(crate) struct ResidueLook {
    pub parts: usize,
    pub stages: usize,
    pub phrasebook_dim: usize,
    pub phrasebook_entries: usize,
    /// Index into codebook slice for the group phrasebook.
    pub phrasebook_idx: usize,
    /// partbooks[part][stage] = Some(codebook index) or None
    pub partbooks: Vec<Vec<Option<usize>>>,
    pub partvals: usize,
    /// decodemap[j][k] = partition class index for phrasebook entry j, position k
    pub decodemap: Vec<Vec<i64>>,
}

// ---------------------------------------------------------------------------
// res0_unpack — port of res0_unpack (setup blob decoder side)
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) enum ResidueError {
    Eof,
    BadGroupbook,
    BadBooklist,
    BadMaptype,
    BadDim,
}

pub(crate) fn unpack_residue(
    r: &mut BitReader,
    books_count: usize,
) -> Result<ResidueSetup, ResidueError> {
    let mut info = ResidueSetup::zeroed();

    info.begin = r.read(24) as i64;
    info.end = r.read(24) as i64;
    info.grouping = r.read(24) as i32 + 1;
    info.partitions = r.read(6) as i32 + 1;
    info.groupbook = r.read(8) as i32;

    if info.groupbook < 0 {
        return Err(ResidueError::Eof);
    }

    let mut acc: usize = 0;
    for j in 0..info.partitions as usize {
        let cascade = r.read(3);
        let cflag = r.read(1);
        if cflag == 0xffff_ffff {
            return Err(ResidueError::Eof);
        }
        let cascade = if cflag != 0 {
            let c = r.read(5);
            if c == 0xffff_ffff {
                return Err(ResidueError::Eof);
            }
            cascade | (c << 3)
        } else {
            cascade
        };
        info.secondstages[j] = cascade as i32;
        acc += icount(cascade) as usize;
    }

    for j in 0..acc {
        let book = r.read(8) as i32;
        if book < 0 {
            return Err(ResidueError::Eof);
        }
        info.booklist[j] = book;
    }

    if info.groupbook >= books_count as i32 {
        return Err(ResidueError::BadGroupbook);
    }

    for j in 0..acc {
        if info.booklist[j] >= books_count as i32 {
            return Err(ResidueError::BadBooklist);
        }
    }

    Ok(info)
}

// ---------------------------------------------------------------------------
// residue_look — port of res0_look (build runtime lookup from setup)
// ---------------------------------------------------------------------------

pub(crate) fn residue_look(info: &ResidueSetup, books: &[Codebook]) -> ResidueLook {
    let phrasebook_idx = info.groupbook as usize;
    let phrasebook_dim = books[phrasebook_idx].dim;
    let phrasebook_entries = books[phrasebook_idx].entries;

    let parts = info.partitions as usize;

    let mut acc: usize = 0;
    let mut maxstage: usize = 0;
    let mut partbooks: Vec<Vec<Option<usize>>> = Vec::with_capacity(parts);

    for j in 0..parts {
        let stages = ov_ilog(info.secondstages[j] as u32) as usize;
        let mut pb: Vec<Option<usize>> = Vec::with_capacity(stages);
        if stages > 0 {
            if stages > maxstage {
                maxstage = stages;
            }
            for k in 0..stages {
                if info.secondstages[j] & (1 << k) != 0 {
                    pb.push(Some(info.booklist[acc] as usize));
                    acc += 1;
                } else {
                    pb.push(None);
                }
            }
        }
        partbooks.push(pb);
    }

    let mut partvals: usize = 1;
    for _j in 0..phrasebook_dim {
        partvals *= parts;
    }

    let dim = phrasebook_dim;
    let mut decodemap: Vec<Vec<i64>> = Vec::with_capacity(partvals);
    for j in 0..partvals {
        let mut val = j as i64;
        let mut mult = partvals as i64 / parts as i64;
        let mut row: Vec<i64> = Vec::with_capacity(dim);
        for _k in 0..dim {
            let deco = val / mult;
            val -= deco * mult;
            mult /= parts as i64;
            row.push(deco);
        }
        decodemap.push(row);
    }

    ResidueLook {
        parts,
        stages: maxstage,
        phrasebook_dim,
        phrasebook_entries,
        phrasebook_idx,
        partbooks,
        partvals,
        decodemap,
    }
}

// ---------------------------------------------------------------------------
// local_book_besterror — port of local_book_besterror in res0.c
// ---------------------------------------------------------------------------

fn local_book_besterror(book: &Codebook, a: &mut [i32]) -> i32 {
    let dbg_enabled = std::env::var("LW_DEBUG_BESTERR").is_ok();
    let dbg_n = if dbg_enabled {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        if n < 200 {
            eprintln!(
                "R_BE n={} dim={} minval={} delta={} quantvals={} entries={} a={:?}",
                n,
                book.dim,
                book.minval,
                book.delta,
                book.quantvals,
                book.entries,
                &a[..book.dim]
            );
        }
        n
    } else {
        usize::MAX
    };
    let dim = book.dim;
    let minval = book.minval;
    let del = book.delta;
    let qv = book.quantvals as i32;
    let ze = qv >> 1;
    let mut index: i32 = 0;
    let mut p = [0i32; 8];

    if del != 1 {
        let mut o = dim;
        for _i in 0..dim {
            o -= 1;
            let v = (a[o] - minval + (del >> 1)) / del;
            let m = if v < ze {
                ((ze - v) << 1) - 1
            } else {
                (v - ze) << 1
            };
            index = index * qv
                + (if m < 0 {
                    0
                } else if m >= qv {
                    qv - 1
                } else {
                    m
                });
            p[o] = v * del + minval;
        }
    } else {
        let mut o = dim;
        for _i in 0..dim {
            o -= 1;
            let v = a[o] - minval;
            let m = if v < ze {
                ((ze - v) << 1) - 1
            } else {
                (v - ze) << 1
            };
            index = index * qv
                + (if m < 0 {
                    0
                } else if m >= qv {
                    qv - 1
                } else {
                    m
                });
            p[o] = v * del + minval;
        }
    }

    let lengthlist = &book.codeword_lengths;
    if index < 0 || index as usize >= book.entries || lengthlist[index as usize] <= 0 {
        let maxval = book.minval + book.delta * (book.quantvals as i32 - 1);
        let mut best: i32 = -1;
        let mut e = [0i32; 8];
        let mut i = 0usize;
        while i < book.entries {
            if lengthlist[i] > 0 {
                let mut this: i32 = 0;
                for j in 0..dim {
                    let val = e[j] - a[j];
                    this += val * val;
                }
                if best == -1 || this < best {
                    p[..dim].copy_from_slice(&e[..dim]);
                    best = this;
                    index = i as i32;
                }
            }
            let mut j = 0;
            while e[j] >= maxval {
                e[j] = 0;
                j += 1;
                if j >= dim {
                    break;
                }
            }
            if j < dim {
                if e[j] >= 0 {
                    e[j] += book.delta;
                }
                e[j] = -e[j];
            }
            i += 1;
        }
    }

    if index > -1 {
        for i in 0..dim {
            a[i] -= p[i];
        }
    }

    if dbg_enabled && dbg_n < 200 {
        eprintln!("R_BE n={} returned index={}", dbg_n, index);
    }
    index
}

// ---------------------------------------------------------------------------
// _encodepart — port of _encodepart in res0.c
// ---------------------------------------------------------------------------

fn _encodepart(opb: &mut BitWriter, vec: &mut [i32], n: usize, book: &Codebook) -> i32 {
    let mut bits: i32 = 0;
    let dim = book.dim;
    let step = n / dim;

    let dbg = std::env::var("LW_DEBUG_BESTERR2").is_ok();

    for i in 0..step {
        let mut vec_in_copy = [0i32; 64];
        let copy_dim = dim.min(64);
        if dbg {
            for z in 0..copy_dim {
                vec_in_copy[z] = vec[i * dim + z];
            }
        }

        let entry = local_book_besterror(book, &mut vec[i * dim..]);

        if dbg {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            if n < 400 {
                let mut s = format!(
                    "R_BESTERR n={} dim={} step_i={} entry={} vec_in=[",
                    n, dim, i, entry
                );
                for z in 0..copy_dim {
                    s.push_str(&format!("{} ", vec_in_copy[z]));
                }
                s.push_str("] vec_residual=[");
                for z in 0..copy_dim {
                    s.push_str(&format!("{} ", vec[i * dim + z]));
                }
                s.push(']');
                eprintln!("{}", s);
            }
        }

        bits += vorbis_book_encode(book, entry, opb) as i32;
    }

    bits
}

// ---------------------------------------------------------------------------
// vorbis_book_encode — thin wrapper matching libvorbis signature
// ---------------------------------------------------------------------------

fn vorbis_book_encode(book: &Codebook, entry: i32, opb: &mut BitWriter) -> usize {
    if entry < 0 {
        return 0;
    }
    book.encode(entry as usize, opb)
}

// ---------------------------------------------------------------------------
// _01class — port of _01class in res0.c
// (residue 0/1 classification)
// ---------------------------------------------------------------------------

pub(crate) fn _01class(
    info: &ResidueSetup,
    look: &ResidueLook,
    in_: &[&[i32]],
    ch: usize,
) -> Vec<Vec<i64>> {
    let samples_per_partition = info.grouping as usize;
    let possible_partitions = info.partitions as usize;
    let n = (info.end - info.begin) as usize;

    let partvals = n / samples_per_partition;
    // C: float scale = 100. / samples_per_partition;  (computed in f64, cast to f32).
    // Then `ent *= scale` is int*float in f32. Match exactly.
    let scale: f32 = (100.0_f64 / samples_per_partition as f64) as f32;
    if std::env::var("LW_DEBUG_RES").is_ok() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let _n = N.fetch_add(1, Ordering::Relaxed);
        eprintln!(
            "R_RES_INFO: possible_partitions={} samples_per_part={} begin={} end={} partvals={}",
            possible_partitions, samples_per_partition, info.begin, info.end, partvals
        );
        eprintln!(
            "R_CLASSMETRIC1: {:?}",
            &info.classmetric1[..possible_partitions]
        );
        eprintln!(
            "R_CLASSMETRIC2: {:?}",
            &info.classmetric2[..possible_partitions]
        );
        let offset = info.begin as usize;
        let mut max = 0i32;
        let mut ent = 0i32;
        for k in 0..samples_per_partition {
            let v = in_[0][offset + k].abs();
            if v > max {
                max = v;
            }
            ent += v;
        }
        eprintln!(
            "R_PART0: max={} ent_pre={} ent_post={} scale={}",
            max,
            ent,
            (ent as f32 * scale) as i32,
            scale
        );
    }

    let mut partword: Vec<Vec<i64>> = Vec::with_capacity(ch);
    for _i in 0..ch {
        partword.push(vec![0i64; partvals]);
    }

    for i in 0..partvals {
        let offset = i * samples_per_partition + info.begin as usize;
        for j in 0..ch {
            let mut max: i32 = 0;
            let mut ent: i32 = 0;
            for k in 0..samples_per_partition {
                let v = in_[j][offset + k].abs();
                if v > max {
                    max = v;
                }
                ent += in_[j][offset + k].abs();
            }
            // C: ent *= scale (int *= float → multiplied in f32, truncated to int).
            let ent = (ent as f32 * scale) as i32;

            let mut kk = 0usize;
            while kk < possible_partitions - 1 {
                if max <= info.classmetric1[kk]
                    && (info.classmetric2[kk] < 0 || ent < info.classmetric2[kk])
                {
                    break;
                }
                kk += 1;
            }

            partword[j][i] = kk as i64;
        }
    }

    partword
}

// ---------------------------------------------------------------------------
// _2class — port of _2class in res0.c
// (residue 2 classification — interleaved stereo)
// ---------------------------------------------------------------------------

pub(crate) fn _2class(
    info: &ResidueSetup,
    look: &ResidueLook,
    in_: &[&[i32]],
    ch: usize,
) -> Vec<Vec<i64>> {
    let samples_per_partition = info.grouping as usize;
    let possible_partitions = info.partitions as usize;
    let n = (info.end - info.begin) as usize;

    let partvals = n / samples_per_partition;
    let dbg_part = std::env::var("LW_DEBUG_PARTWORD").is_ok();
    if dbg_part {
        eprintln!(
            "R_2CLASS_IN partvals={} cm1={:?} cm2={:?}",
            partvals,
            &info.classmetric1[..possible_partitions],
            &info.classmetric2[..possible_partitions]
        );
    }
    let mut partword: Vec<Vec<i64>> = vec![vec![0i64; partvals]];

    let mut l = (info.begin as usize) / ch;
    for i in 0..partvals {
        let mut magmax: i32 = 0;
        let mut angmax: i32 = 0;
        let mut j = 0;
        while j < samples_per_partition {
            let v = in_[0][l].abs();
            if v > magmax {
                magmax = v;
            }
            for k in 1..ch {
                let v = in_[k][l].abs();
                if v > angmax {
                    angmax = v;
                }
            }
            l += 1;
            j += ch;
        }

        let mut jj = 0usize;
        while jj < possible_partitions - 1 {
            if magmax <= info.classmetric1[jj] && angmax <= info.classmetric2[jj] {
                break;
            }
            jj += 1;
        }

        if dbg_part && i < 16 {
            eprintln!(
                "R_2CLASS i={} magmax={} angmax={} class={}",
                i, magmax, angmax, jj
            );
        }
        partword[0][i] = jj as i64;
    }

    partword
}

// ---------------------------------------------------------------------------
// _01forward — port of _01forward in res0.c
// ---------------------------------------------------------------------------

fn _01forward(
    opb: &mut BitWriter,
    info: &ResidueSetup,
    look: &ResidueLook,
    in_: &mut [&mut [i32]],
    ch: usize,
    partword: &[Vec<i64>],
    books: &[Codebook],
) -> i32 {
    let samples_per_partition = info.grouping as usize;
    let possible_partitions = info.partitions as usize;
    let partitions_per_word = look.phrasebook_dim;
    let n = (info.end - info.begin) as usize;

    let partvals = n / samples_per_partition;

    let mut resbits = [0i64; 128];
    let mut resvals = [0i64; 128];

    for s in 0..look.stages {
        let mut i = 0usize;
        while i < partvals {
            // first we encode a partition codeword for each channel
            if s == 0 {
                for j in 0..ch {
                    let mut val = partword[j][i];
                    for k in 1..partitions_per_word {
                        val *= possible_partitions as i64;
                        if i + k < partvals {
                            val += partword[j][i + k];
                        }
                    }

                    if val < look.phrasebook_entries as i64 {
                        let phrasebook = &books[look.phrasebook_idx];
                        look_phrasebits_add(vorbis_book_encode(phrasebook, val as i32, opb) as i64);
                    }
                }
            }

            // now we encode interleaved residual values for the partitions
            let mut k = 0usize;
            while k < partitions_per_word && i < partvals {
                let offset = i * samples_per_partition + info.begin as usize;

                for j in 0..ch {
                    if s == 0 {
                        resvals[partword[j][i] as usize] += samples_per_partition as i64;
                    }
                    let pw = partword[j][i] as usize;
                    if info.secondstages[pw] & (1 << s) != 0 {
                        if pw < look.partbooks.len() && s < look.partbooks[pw].len() {
                            if let Some(book_idx) = look.partbooks[pw][s] {
                                let statebook = &books[book_idx];
                                let ret = _encodepart(
                                    opb,
                                    &mut in_[j][offset..offset + samples_per_partition],
                                    samples_per_partition,
                                    statebook,
                                );
                                look_postbits_add(ret as i64);
                                resbits[pw] += ret as i64;
                            }
                        }
                    }
                }
                k += 1;
                i += 1;
            }
        }
    }

    0
}

// No-op stat helpers (libvorbis tracks these in the look struct; we elide them
// since they're only used for training/stats, not for correctness).
#[inline(always)]
fn look_phrasebits_add(_bits: i64) {}
#[inline(always)]
fn look_postbits_add(_bits: i64) {}

// ---------------------------------------------------------------------------
// res1_class — port of res1_class
// ---------------------------------------------------------------------------

pub(crate) fn res1_class(
    info: &ResidueSetup,
    look: &ResidueLook,
    in_: &mut Vec<&[i32]>,
    nonzero: &[bool],
    ch: usize,
) -> Option<Vec<Vec<i64>>> {
    let mut used = 0usize;
    let mut used_in: Vec<&[i32]> = Vec::new();
    for i in 0..ch {
        if nonzero[i] {
            used_in.push(in_[i]);
            used += 1;
        }
    }
    if used > 0 {
        Some(_01class(info, look, &used_in, used))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// res1_forward — port of res1_forward
// ---------------------------------------------------------------------------

pub(crate) fn res1_forward(
    opb: &mut BitWriter,
    info: &ResidueSetup,
    look: &ResidueLook,
    in_: &mut [&mut [i32]],
    nonzero: &[bool],
    ch: usize,
    partword: &[Vec<i64>],
    books: &[Codebook],
) -> i32 {
    let mut used = 0usize;
    let mut used_in: Vec<&mut [i32]> = Vec::new();

    // We need to compact the channels; collect indices first to avoid borrow issues
    // (literal port: in[used++]=in[i] for nonzero[i])
    let mut indices: Vec<usize> = Vec::new();
    for i in 0..ch {
        if nonzero[i] {
            indices.push(i);
        }
    }
    used = indices.len();

    if used > 0 {
        // Safety: `in_` contains mutable slices; we split by index.
        // We build a Vec of raw mut pointers, then reborrow as mut slices.
        // This is pure safe Rust via repeated split_at_mut-style logic.
        // Instead of pointer tricks we just pass the full slice and a mask.
        _01forward_indexed(opb, info, look, in_, &indices, partword, books)
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// res2_class — port of res2_class
// ---------------------------------------------------------------------------

pub(crate) fn res2_class(
    info: &ResidueSetup,
    look: &ResidueLook,
    in_: &[&[i32]],
    nonzero: &[bool],
    ch: usize,
) -> Option<Vec<Vec<i64>>> {
    let mut used = 0usize;
    for i in 0..ch {
        if nonzero[i] {
            used += 1;
        }
    }
    if used > 0 {
        Some(_2class(info, look, in_, ch))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// res2_forward — port of res2_forward
// ---------------------------------------------------------------------------

pub(crate) fn res2_forward(
    opb: &mut BitWriter,
    info: &ResidueSetup,
    look: &ResidueLook,
    in_: &[&[i32]],
    nonzero: &[bool],
    ch: usize,
    n: usize,
    partword: &[Vec<i64>],
    books: &[Codebook],
) -> i32 {
    let mut used = 0usize;

    let mut work: Vec<i32> = vec![0i32; ch * n];
    for i in 0..ch {
        let pcm = in_[i];
        if nonzero[i] {
            used += 1;
        }
        let mut k = i;
        for j in 0..n {
            work[k] = pcm[j];
            k += ch;
        }
    }

    if used > 0 {
        let mut work_ref: &mut [i32] = &mut work;
        let mut wrapped: [&mut [i32]; 1] = [work_ref];
        _01forward(opb, info, look, &mut wrapped, 1, partword, books)
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// _01forward_indexed — helper that compacts channels before calling _01forward
// ---------------------------------------------------------------------------

fn _01forward_indexed(
    opb: &mut BitWriter,
    info: &ResidueSetup,
    look: &ResidueLook,
    in_: &mut [&mut [i32]],
    indices: &[usize],
    partword: &[Vec<i64>],
    books: &[Codebook],
) -> i32 {
    let used = indices.len();

    // Partition word slices: reindex by compacted channel order
    // partword[j] corresponds to original channel indices[j]
    let compacted_pw: Vec<Vec<i64>> = indices
        .iter()
        .map(|&idx| {
            if idx < partword.len() {
                partword[idx].clone()
            } else {
                Vec::new()
            }
        })
        .collect();

    // We need to call _01forward with compacted in_ channels.
    // Since we can't easily compact &mut slices without unsafe, we use the
    // full in_ slice with direct index remapping via a local inline expansion.
    // This mirrors the C `in[used++]=in[i]` pointer compaction.

    let samples_per_partition = info.grouping as usize;
    let possible_partitions = info.partitions as usize;
    let partitions_per_word = look.phrasebook_dim;
    let n = (info.end - info.begin) as usize;
    let partvals = n / samples_per_partition;

    let mut resbits = [0i64; 128];
    let mut resvals = [0i64; 128];

    for s in 0..look.stages {
        let mut i = 0usize;
        while i < partvals {
            if s == 0 {
                for j in 0..used {
                    let mut val = compacted_pw[j][i];
                    for k in 1..partitions_per_word {
                        val *= possible_partitions as i64;
                        if i + k < partvals {
                            val += compacted_pw[j][i + k];
                        }
                    }
                    if val < look.phrasebook_entries as i64 {
                        if std::env::var("LW_DEBUG_PHRASE").is_ok() {
                            use std::sync::atomic::{AtomicUsize, Ordering};
                            static N: AtomicUsize = AtomicUsize::new(0);
                            let n = N.fetch_add(1, Ordering::Relaxed);
                            if n < 30 {
                                eprintln!(
                                    "R_PHRASE n={} s={} i={} j={} val={} stages={} ppw={} partvals={}",
                                    n, s, i, j, val, look.stages, partitions_per_word, partvals
                                );
                            }
                        }
                        let phrasebook = &books[look.phrasebook_idx];
                        vorbis_book_encode(phrasebook, val as i32, opb);
                    }
                }
            }

            let mut k = 0usize;
            while k < partitions_per_word && i < partvals {
                let offset = i * samples_per_partition + info.begin as usize;

                for j in 0..used {
                    let orig_ch = indices[j];
                    if s == 0 {
                        resvals[compacted_pw[j][i] as usize] += samples_per_partition as i64;
                    }
                    let pw = compacted_pw[j][i] as usize;
                    if info.secondstages[pw] & (1 << s) != 0 {
                        if pw < look.partbooks.len() && s < look.partbooks[pw].len() {
                            if let Some(book_idx) = look.partbooks[pw][s] {
                                let statebook = &books[book_idx];
                                let ret = _encodepart(
                                    opb,
                                    &mut in_[orig_ch][offset..offset + samples_per_partition],
                                    samples_per_partition,
                                    statebook,
                                );
                                resbits[pw] += ret as i64;
                            }
                        }
                    }
                }
                k += 1;
                i += 1;
            }
        }
    }

    0
}

// ---------------------------------------------------------------------------
// unpack_q5_residues — parse residue configs from the setup blob.
//
// Called after codebooks + time + floors have been consumed.
// Reads: (residues-1) in 6 bits, then for each: 16-bit type + res0_unpack body.
// ---------------------------------------------------------------------------

pub(crate) fn unpack_q5_residues(
    r: &mut BitReader,
    books_count: usize,
) -> Result<Vec<(u16, ResidueSetup)>, ResidueError> {
    let residues = r.read(6) as usize + 1;
    let mut result = Vec::with_capacity(residues);
    for _ in 0..residues {
        let residue_type = r.read(16) as u16;
        let setup = unpack_residue(r, books_count)?;
        result.push((residue_type, setup));
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
    use crate::codebook::unpack_codebook;
    use crate::floor1::unpack_q5_floors;
    use crate::setup_blob::Q5_SETUP_BLOB;

    fn parse_blob_up_to_residues() -> (usize, Vec<(u16, ResidueSetup)>) {
        let blob = Q5_SETUP_BLOB;
        assert_eq!(&blob[0..7], b"\x05vorbis");
        let mut r = BitReader::new(&blob[7..]);

        let count = r.read(8) as usize + 1;
        for _ in 0..count {
            unpack_codebook(&mut r).expect("codebook unpack");
        }

        let _floors = unpack_q5_floors(&mut r, count).expect("floor unpack");
        let residues = unpack_q5_residues(&mut r, count).expect("residue unpack");
        (count, residues)
    }

    #[test]
    fn q5_blob_residues_unpack() {
        let (_count, residues) = parse_blob_up_to_residues();
        assert!(!residues.is_empty(), "expected at least 1 residue, got 0");
        for (i, (rtype, setup)) in residues.iter().enumerate() {
            assert!(*rtype <= 2, "residue {i}: unexpected type {rtype}");
            assert!(
                setup.partitions > 0,
                "residue {i}: partitions should be > 0"
            );
            assert!(setup.grouping > 0, "residue {i}: grouping should be > 0");
            assert!(
                setup.end > setup.begin,
                "residue {i}: end ({}) must be > begin ({})",
                setup.end,
                setup.begin
            );
        }
    }

    #[test]
    fn q5_residue_count_is_expected() {
        let (_count, residues) = parse_blob_up_to_residues();
        // Q5 stereo has 2 residues (one per submap), mono has 1.
        // The Q5 blob embedded here should have >=1 and <=4.
        assert!(
            !residues.is_empty() && residues.len() <= 4,
            "unexpected residue count: {}",
            residues.len()
        );
    }

    #[test]
    fn icount_popcount_correct() {
        assert_eq!(icount(0), 0);
        assert_eq!(icount(1), 1);
        assert_eq!(icount(0b1010_1010), 4);
        assert_eq!(icount(0b1111_1111), 8);
        assert_eq!(icount(u32::MAX), 32);
    }

    #[test]
    fn residue_look_builds_without_panic() {
        let blob = Q5_SETUP_BLOB;
        let mut r = BitReader::new(&blob[7..]);
        let count = r.read(8) as usize + 1;
        let mut books = Vec::with_capacity(count);
        for _ in 0..count {
            books.push(unpack_codebook(&mut r).expect("codebook"));
        }
        let _floors = unpack_q5_floors(&mut r, count).expect("floors");
        let residues = unpack_q5_residues(&mut r, count).expect("residues");

        for (_rtype, setup) in &residues {
            let look = residue_look(setup, &books);
            assert!(look.parts > 0, "look.parts must be > 0");
            assert!(look.phrasebook_dim > 0, "phrasebook_dim must be > 0");
        }
    }

    #[test]
    fn synthetic_unpack_residue_roundtrip() {
        use crate::bitpack::BitWriter;
        // Pack a minimal residue setup by hand and verify unpack recovers it.
        // We need a "books_count" large enough so groupbook validation passes.
        let books_count = 10usize;

        let mut w = BitWriter::new();
        // begin=0 (24 bits)
        w.write(0, 24);
        // end=16 (24 bits)
        w.write(16, 24);
        // grouping-1=7 (24 bits) → grouping=8
        w.write(7, 24);
        // partitions-1=1 (6 bits) → partitions=2
        w.write(1, 6);
        // groupbook=0 (8 bits)
        w.write(0, 8);

        // partition 0: cascade=1 (3 bits), cflag=0 (1 bit)
        w.write(1, 3);
        w.write(0, 1);
        // partition 1: cascade=0 (3 bits), cflag=0 (1 bit)
        w.write(0, 3);
        w.write(0, 1);

        // acc = icount(1) + icount(0) = 1
        // booklist[0] = 1
        w.write(1, 8);

        let bytes = w.into_bytes();
        let mut r = BitReader::new(&bytes);
        let setup = unpack_residue(&mut r, books_count).expect("unpack_residue failed");

        assert_eq!(setup.begin, 0);
        assert_eq!(setup.end, 16);
        assert_eq!(setup.grouping, 8);
        assert_eq!(setup.partitions, 2);
        assert_eq!(setup.groupbook, 0);
        assert_eq!(setup.secondstages[0], 1);
        assert_eq!(setup.secondstages[1], 0);
        assert_eq!(setup.booklist[0], 1);
    }
}
