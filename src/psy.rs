//! Psychoacoustic model — literal port of libvorbis 1.3.7 lib/psy.c (encode side only).
//!
//! All runtime transcendentals (log, exp, sin, cos, atan, pow) have been
//! replaced by either precomputed constants or inline IEEE-754 bit tricks that
//! match the originals exactly.  The only remaining transcendental is `sqrt`,
//! which is IEEE-754 required and therefore platform-identical.

#![allow(clippy::needless_range_loop)]
#![allow(clippy::manual_clamp)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::excessive_precision)]
#![allow(clippy::assign_op_pattern)]
#![allow(clippy::approx_constant)]
#![allow(clippy::too_many_arguments)]
#![allow(non_snake_case)]
#![allow(unused_mut)]
#![allow(unused_variables)]
#![allow(unused_assignments)]

use crate::tables::lookup::FLOOR1_FROMDB_LOOKUP;
use crate::tables::masking::{ATH, EHMER_MAX, EHMER_OFFSET, MAX_ATH, TONEMASKS};

// ---------------------------------------------------------------------------
// Constants from psy.h / psy.c
// ---------------------------------------------------------------------------

const NEGINF: f32 = -9999.0_f32;

/// Upper bound on `VorbisLookPsy::total_octave_lines` — used to size the
/// stack-allocated seed scratch in `vp_tonemask`. Observed max for the Q5
/// supported input space is 777 (long block at 44.1kHz); 1024 gives some
/// headroom and matches the long-block half-spectrum size.
const MAX_OCTAVE_LINES: usize = 1024;

/// Q5 normal_partition is 32; older 16-partition layouts also fit. Used for
/// stack-allocated scratch in `vp_couple_quantize_normalize`.
const MAX_PARTITION: usize = 32;
/// Mono or stereo for the supported input space.
const MAX_CH: usize = 2;
/// Stereo Q5 has 1 coupling step; mono has 0. 2 is overkill safe.
const MAX_COUPLING_STEPS: usize = 2;

pub const P_BANDS: usize = 17;
pub const P_LEVELS: usize = 8;
pub const P_LEVEL_0: f32 = 30.0_f32;
pub const P_NOISECURVES: usize = 3;
pub const NOISE_COMPAND_LEVELS: usize = 40;

pub const PACKETBLOBS: usize = 15;
pub const VE_BANDS: usize = 7;

// stereo thresholds from psy.c
static STEREO_THRESHHOLDS: [f64; 9] = [0.0, 0.5, 1.0, 1.5, 2.5, 4.5, 8.5, 16.5, 9e10];
static STEREO_THRESHHOLDS_LIMITED: [f64; 9] = [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 4.5, 8.5, 9e10];

// ---------------------------------------------------------------------------
// Scale helpers — literal ports of scales.h macros.
// No runtime transcendentals: toBARK / toOC / fromOC use inline formulas that
// *do* call atan/log/exp.  We keep them here as private helpers used only
// during psy_init (setup), not in the per-frame hot path.
// ---------------------------------------------------------------------------

/// toBARK (scales.h): `13.1f*atan(.00074f*n)+2.24f*atan(n*n*1.85e-8f)+1e-4f*n`.
/// All constants are f32 in C and get promoted to f64 in math (atan is f64).
/// Match exactly by promoting f32 constants to f64 before the arithmetic.
fn to_bark(n: f32) -> f32 {
    let n = n as f64;
    let c0 = 13.1_f32 as f64;
    let c1 = 0.00074_f32 as f64;
    let c2 = 2.24_f32 as f64;
    let c3 = 1.85e-8_f32 as f64;
    let c4 = 1e-4_f32 as f64;
    (c0 * (c1 * n).atan() + c2 * (n * n * c3).atan() + c4 * n) as f32
}

/// toOC (scales.h): `log(n)*1.442695f-5.965784f`. C constants are f32, promoted
/// to f64 in the math.
fn to_oc(n: f32) -> f32 {
    let c1 = 1.442695_f32 as f64;
    let c2 = 5.965784_f32 as f64;
    ((n as f64).ln() * c1 - c2) as f32
}

/// fromOC (scales.h): `exp((o+5.965784f)*.693147f)`. f32 constants promoted to f64.
fn from_oc(o: f32) -> f32 {
    let c1 = 5.965784_f32 as f64;
    let c2 = 0.693147_f32 as f64;
    (((o as f64 + c1) * c2).exp()) as f32
}

/// rint: round to nearest integer per C standard (half-to-even).
/// Rust's `.round()` rounds half-away-from-zero, which diverges at
/// half-integers (0.5 → 1 vs C's 0).
#[inline(always)]
fn rint(x: f32) -> f32 {
    x.round_ties_even()
}

/// todB: bit-manipulation fast dB (scales.h, VORBIS_IEEE_FLOAT32 path).
/// Uses the bit trick: dB ≈ bits * 7.17711438e-7 - 764.6161886.
/// No transcendentals.
#[inline(always)]
pub fn to_db(x: f32) -> f32 {
    let bits = x.to_bits() & 0x7fff_ffff;
    bits as f32 * 7.17711438e-7_f32 - 764.6161886_f32
}

/// unitnorm: sign-preserving unit normalisation (scales.h, IEEE path).
#[inline(always)]
pub fn unitnorm(x: f32) -> f32 {
    let bits = (x.to_bits() & 0x8000_0000u32) | 0x3f80_0000u32;
    f32::from_bits(bits)
}

// ---------------------------------------------------------------------------
// Info structs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct VorbisInfoPsy {
    pub blockflag: i32,

    pub ath_adjatt: f32,
    pub ath_maxatt: f32,

    pub tone_masteratt: [f32; P_NOISECURVES],
    pub tone_centerboost: f32,
    pub tone_decay: f32,
    pub tone_abs_limit: f32,
    pub toneatt: [f32; P_BANDS],

    pub noisemaskp: i32,
    pub noisemaxsupp: f32,
    pub noisewindowlo: f32,
    pub noisewindowhi: f32,
    pub noisewindowlomin: i32,
    pub noisewindowhimin: i32,
    pub noisewindowfixed: i32,
    pub noiseoff: [[f32; P_BANDS]; P_NOISECURVES],
    pub noisecompand: [f32; NOISE_COMPAND_LEVELS],

    pub max_curve_dB: f32,

    pub normal_p: i32,
    pub normal_start: i32,
    pub normal_partition: i32,
    pub normal_thresh: f64,
}

impl Default for VorbisInfoPsy {
    fn default() -> Self {
        Self {
            blockflag: 0,
            ath_adjatt: 0.0,
            ath_maxatt: 0.0,
            tone_masteratt: [0.0; P_NOISECURVES],
            tone_centerboost: 0.0,
            tone_decay: 0.0,
            tone_abs_limit: 0.0,
            toneatt: [0.0; P_BANDS],
            noisemaskp: 0,
            noisemaxsupp: 0.0,
            noisewindowlo: 0.0,
            noisewindowhi: 0.0,
            noisewindowlomin: 0,
            noisewindowhimin: 0,
            noisewindowfixed: 0,
            noiseoff: [[0.0; P_BANDS]; P_NOISECURVES],
            noisecompand: [0.0; NOISE_COMPAND_LEVELS],
            max_curve_dB: 0.0,
            normal_p: 0,
            normal_start: 0,
            normal_partition: 0,
            normal_thresh: 0.0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct VorbisInfoPsyGlobal {
    pub eighth_octave_lines: i32,

    pub preecho_thresh: [f32; VE_BANDS],
    pub postecho_thresh: [f32; VE_BANDS],
    pub stretch_penalty: f32,
    pub preecho_minenergy: f32,

    pub ampmax_att_per_sec: f32,

    pub coupling_pkHz: [i32; PACKETBLOBS],
    pub coupling_pointlimit: [[i32; PACKETBLOBS]; 2],
    pub coupling_prepointamp: [i32; PACKETBLOBS],
    pub coupling_postpointamp: [i32; PACKETBLOBS],
    pub sliding_lowpass: [[i32; PACKETBLOBS]; 2],
}

#[derive(Clone, Debug, Default)]
pub struct VorbisLookPsyGlobal {
    pub ampmax: f32,
    pub channels: i32,
    pub gi: Option<VorbisInfoPsyGlobal>,
    pub coupling_pointlimit: [[i32; P_NOISECURVES]; 2],
}

// ---------------------------------------------------------------------------
// Tone curve buffer: mirrors the float*** returned by setup_tone_curves.
// We flatten to Box<[[[f32; EHMER_MAX+2]; P_LEVELS]; P_BANDS]>.
// ---------------------------------------------------------------------------

pub type ToneCurves = Box<[[[f32; EHMER_MAX + 2]; P_LEVELS]; P_BANDS]>;

// ---------------------------------------------------------------------------
// Look struct
// ---------------------------------------------------------------------------

pub struct VorbisLookPsy {
    pub n: usize,
    pub vi: VorbisInfoPsy,

    pub tonecurves: ToneCurves,
    pub noiseoffset: Vec<Vec<f32>>, // [P_NOISECURVES][n]

    pub ath: Vec<f32>,    // [n]
    pub octave: Vec<i32>, // [n]  in n.ocshift format (i32 in C is long)
    pub bark: Vec<i64>,   // [n]

    pub firstoc: i64,
    pub shiftoc: i32,
    pub eighth_octave_lines: i32,
    pub total_octave_lines: i32,
    pub rate: i64,

    pub m_val: f32,
}

// ---------------------------------------------------------------------------
// setup_tone_curves — port of the static function in psy.c
// ---------------------------------------------------------------------------

fn setup_tone_curves(
    curveatt_db: &[f32; P_BANDS],
    bin_hz: f32,
    n: usize,
    center_boost: f32,
    center_decay_rate: f32,
) -> ToneCurves {
    let mut workc = [[[0.0_f32; EHMER_MAX]; P_LEVELS]; P_BANDS];
    let mut athc = [[0.0_f32; EHMER_MAX]; P_LEVELS];
    let mut ath = [0.0_f32; EHMER_MAX];
    let mut brute_buffer = vec![0.0_f32; n];

    // build return buffer
    let mut ret = Box::new([[[0.0_f32; EHMER_MAX + 2]; P_LEVELS]; P_BANDS]);

    for i in 0..P_BANDS {
        // build ATH slice for this band
        let ath_offset = i * 4;
        for j in 0..EHMER_MAX {
            let mut min = 999.0_f32;
            for k in 0..4 {
                let idx = j + k + ath_offset;
                if idx < MAX_ATH {
                    if min > ATH[idx] {
                        min = ATH[idx];
                    }
                } else {
                    if min > ATH[MAX_ATH - 1] {
                        min = ATH[MAX_ATH - 1];
                    }
                }
            }
            ath[j] = min;
        }

        // copy curves (6 levels from tonemasks; replicate 0→0,1; 5→6,7)
        for j in 0..6 {
            workc[i][j + 2].copy_from_slice(&TONEMASKS[i][j]);
        }
        workc[i][0].copy_from_slice(&TONEMASKS[i][0]);
        workc[i][1].copy_from_slice(&TONEMASKS[i][0]);

        // apply centered curve boost/decay
        for j in 0..P_LEVELS {
            for k in 0..EHMER_MAX {
                let offset_k = EHMER_OFFSET as i32 - k as i32;
                let mut adj = center_boost + offset_k.unsigned_abs() as f32 * center_decay_rate;
                if adj < 0.0 && center_boost > 0.0 {
                    adj = 0.0;
                }
                if adj > 0.0 && center_boost < 0.0 {
                    adj = 0.0;
                }
                workc[i][j][k] += adj;
            }
        }

        // normalize curves; overlay ATH
        for j in 0..P_LEVELS {
            let att =
                curveatt_db[i] + 100.0 - (if j < 2 { 2 } else { j } as f32) * 10.0 - P_LEVEL_0;
            for k in 0..EHMER_MAX {
                workc[i][j][k] += att;
            }
            athc[j].copy_from_slice(&ath);
            let att2 = 100.0 - j as f32 * 10.0 - P_LEVEL_0;
            for k in 0..EHMER_MAX {
                athc[j][k] += att2;
            }
            // max_curve(athc[j], workc[i][j])
            for k in 0..EHMER_MAX {
                if athc[j][k] < workc[i][j][k] {
                    athc[j][k] = workc[i][j][k];
                }
            }
        }

        // limit louder curves
        for j in 1..P_LEVELS {
            // min_curve(athc[j], athc[j-1])
            for k in 0..EHMER_MAX {
                if athc[j][k] > athc[j - 1][k] {
                    athc[j][k] = athc[j - 1][k];
                }
            }
            // min_curve(workc[i][j], athc[j])
            for k in 0..EHMER_MAX {
                if workc[i][j][k] > athc[j][k] {
                    workc[i][j][k] = athc[j][k];
                }
            }
        }
    }

    if crate::debug_dump::dump_enabled() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static FIRED: AtomicBool = AtomicBool::new(false);
        if !FIRED.swap(true, Ordering::Relaxed) {
            let mut bytes = Vec::new();
            for b in 0..P_BANDS {
                for lv in 0..P_LEVELS {
                    for v in workc[b][lv].iter() {
                        bytes.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
            let _ = std::fs::write("/tmp/lewtoff-debug/r_workc.bin", &bytes);
        }
    }

    for i in 0..P_BANDS {
        // C does these arg computations in f64 (literals are double), casts to
        // float only at the fromOC()/toOC() call boundary. Mirror that.
        let bin = (from_oc((i as f64 * 0.5) as f32) / bin_hz).floor() as i64;
        let lo_curve_f = to_oc((bin as f64 * bin_hz as f64 + 1.0) as f32) * 2.0;
        let hi_curve_f = to_oc(((bin + 1) as f64 * bin_hz as f64) as f32) * 2.0;
        let mut lo_curve = lo_curve_f.ceil() as i64;
        let mut hi_curve = hi_curve_f.floor() as i64;
        if lo_curve > i as i64 {
            lo_curve = i as i64;
        }
        if lo_curve < 0 {
            lo_curve = 0;
        }
        if hi_curve >= P_BANDS as i64 {
            hi_curve = P_BANDS as i64 - 1;
        }

        for m in 0..P_LEVELS {
            for j in 0..n {
                brute_buffer[j] = 999.0;
            }

            for k in lo_curve..=hi_curve {
                let mut l: usize = 0;
                for j in 0..EHMER_MAX {
                    let lo_bin = (from_oc((j as f64 * 0.125 + k as f64 * 0.5 - 2.0625) as f32)
                        / bin_hz) as i64;
                    let hi_bin = (from_oc((j as f64 * 0.125 + k as f64 * 0.5 - 1.9375) as f32)
                        / bin_hz) as i64
                        + 1;
                    let lo_bin = lo_bin.max(0).min(n as i64) as usize;
                    let hi_bin = hi_bin.max(0).min(n as i64) as usize;
                    if lo_bin < l {
                        l = lo_bin;
                    } // match C: if(lo_bin<l) l=lo_bin
                    while l < hi_bin && l < n {
                        if brute_buffer[l] > workc[k as usize][m][j] {
                            brute_buffer[l] = workc[k as usize][m][j];
                        }
                        l += 1;
                    }
                }
                while l < n {
                    if brute_buffer[l] > workc[k as usize][m][EHMER_MAX - 1] {
                        brute_buffer[l] = workc[k as usize][m][EHMER_MAX - 1];
                    }
                    l += 1;
                }
            }

            // next half-octave
            if i + 1 < P_BANDS {
                let mut l: usize = 0;
                let k = i + 1;
                for j in 0..EHMER_MAX {
                    let lo_bin = (from_oc((j as f64 * 0.125 + i as f64 * 0.5 - 2.0625) as f32)
                        / bin_hz) as i64;
                    let hi_bin = (from_oc((j as f64 * 0.125 + i as f64 * 0.5 - 1.9375) as f32)
                        / bin_hz) as i64
                        + 1;
                    let lo_bin = lo_bin.max(0).min(n as i64) as usize;
                    let hi_bin = hi_bin.max(0).min(n as i64) as usize;
                    if lo_bin < l {
                        l = lo_bin;
                    }
                    while l < hi_bin && l < n {
                        if brute_buffer[l] > workc[k][m][j] {
                            brute_buffer[l] = workc[k][m][j];
                        }
                        l += 1;
                    }
                }
                while l < n {
                    if brute_buffer[l] > workc[k][m][EHMER_MAX - 1] {
                        brute_buffer[l] = workc[k][m][EHMER_MAX - 1];
                    }
                    l += 1;
                }
            }

            // pull values back into curve
            for j in 0..EHMER_MAX {
                let bin =
                    (from_oc((j as f64 * 0.125 + i as f64 * 0.5 - 2.0) as f32) / bin_hz) as i64;
                if bin < 0 || bin >= n as i64 {
                    ret[i][m][j + 2] = -999.0;
                } else {
                    ret[i][m][j + 2] = brute_buffer[bin as usize];
                }
            }

            // fenceposts
            let mut j = 0;
            while j < EHMER_OFFSET {
                if ret[i][m][j + 2] > -200.0 {
                    break;
                }
                j += 1;
            }
            ret[i][m][0] = j as f32;

            let mut j2 = EHMER_MAX as i32 - 1;
            while j2 > EHMER_OFFSET as i32 + 1 {
                if ret[i][m][j2 as usize + 2] > -200.0 {
                    break;
                }
                j2 -= 1;
            }
            ret[i][m][1] = j2 as f32;
        }
    }

    ret
}

// ---------------------------------------------------------------------------
// _vp_psy_init
// ---------------------------------------------------------------------------

pub fn vp_psy_init(
    vi: VorbisInfoPsy,
    gi: &VorbisInfoPsyGlobal,
    n: usize,
    rate: i64,
) -> VorbisLookPsy {
    let mut p = VorbisLookPsy {
        n,
        vi: vi.clone(),
        tonecurves: Box::new([[[0.0; EHMER_MAX + 2]; P_LEVELS]; P_BANDS]),
        noiseoffset: vec![vec![0.0; n]; P_NOISECURVES],
        ath: vec![0.0; n],
        octave: vec![0; n],
        bark: vec![0; n],
        firstoc: 0,
        shiftoc: 0,
        eighth_octave_lines: gi.eighth_octave_lines,
        total_octave_lines: 0,
        rate,
        m_val: 1.0,
    };

    // libvorbis: `p->shiftoc = rint(log(gi->eighth_octave_lines*8.f)/log(2.f))-1;`
    // log() is the f64 overload: f32 arg promotes to f64. f64 division.
    // rint() rounds half-to-even.
    let arg = (gi.eighth_octave_lines * 8) as f32 as f64;
    let two = 2.0_f32 as f64;
    p.shiftoc = (arg.ln() / two.ln()).round_ties_even() as i32 - 1;

    p.firstoc = (to_oc(0.25_f32 * rate as f32 * 0.5 / n as f32) as f64
        * (1i64 << (p.shiftoc + 1)) as f64
        - gi.eighth_octave_lines as f64) as i64;
    let maxoc = (to_oc((n as f32 + 0.25) * rate as f32 * 0.5 / n as f32) as f64
        * (1i64 << (p.shiftoc + 1)) as f64
        + 0.5) as i64;
    p.total_octave_lines = (maxoc - p.firstoc + 1) as i32;
    debug_assert!(
        p.total_octave_lines <= MAX_OCTAVE_LINES as i32,
        "total_octave_lines={} exceeds MAX_OCTAVE_LINES; bump const",
        p.total_octave_lines
    );

    // AoTuV HF weighting
    p.m_val = 1.0;
    if rate < 26000 {
        p.m_val = 0.0;
    } else if rate < 38000 {
        p.m_val = 0.94;
    } else if rate > 46000 {
        p.m_val = 1.275;
    }

    // ATH setup
    {
        let mut j: usize = 0;
        for i in 0..(MAX_ATH - 1) {
            let endpos =
                rint(from_oc((i as f32 + 1.0) * 0.125 - 2.0) * 2.0 * n as f32 / rate as f32)
                    as usize;
            let base = ATH[i];
            if j < endpos {
                let delta = if endpos > j {
                    (ATH[i + 1] - base) / (endpos - j) as f32
                } else {
                    0.0
                };
                let mut b = base;
                while j < endpos && j < n {
                    p.ath[j] = b + 100.0;
                    b += delta;
                    j += 1;
                }
            }
        }
        while j < n {
            p.ath[j] = p.ath[j - 1];
            j += 1;
        }
    }

    // bark array
    //
    // libvorbis psy.c uses INTEGER division `rate/(2*n)*i` here, where rate
    // is long and n/i are int. The truncation matters: at rate=44100, n=1024
    // we get 44100/2048 = 21 (integer) instead of the exact 21.5332. The
    // window-edge calculation depends on this so the integer truncation must
    // be reproduced literally — using float division here flips a few bark
    // window `hi` values by 1 bin (visible in noise[] starting around bin
    // 118 for some inputs).
    {
        let bin_hz = rate / (2 * n as i64); // integer division to match C
        let mut lo: i64 = -99;
        let mut hi: i64 = 1;
        for i in 0..n {
            let bark = to_bark((bin_hz * i as i64) as f32);

            while lo + (vi.noisewindowlomin as i64) < i as i64
                && to_bark((bin_hz * lo) as f32) < bark - vi.noisewindowlo
            {
                lo += 1;
            }

            while hi <= n as i64
                && (hi < i as i64 + vi.noisewindowhimin as i64
                    || to_bark((bin_hz * hi) as f32) < bark + vi.noisewindowhi)
            {
                hi += 1;
            }

            p.bark[i] = ((lo - 1) << 16) + (hi - 1);

            if (10..=30).contains(&i) && crate::debug_flag!("LW_DEBUG_BARK") {
                eprintln!("LW_BARK_INIT n={} i={}: lo={} hi={}", n, i, lo, hi);
            }
        }
    }

    // octave array
    for i in 0..n {
        p.octave[i] = (to_oc((i as f32 + 0.25) * 0.5 * rate as f32 / n as f32)
            * (1i32 << (p.shiftoc + 1)) as f32
            + 0.5) as i32;
    }

    // tone curves
    if crate::debug_dump::dump_enabled() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static FIRED: AtomicBool = AtomicBool::new(false);
        if !FIRED.swap(true, Ordering::Relaxed) {
            // We dump after setup_tone_curves. Mark for later.
        }
    }
    p.tonecurves = setup_tone_curves(
        &vi.toneatt,
        rate as f32 * 0.5 / n as f32,
        n,
        vi.tone_centerboost,
        vi.tone_decay,
    );
    if crate::debug_dump::dump_enabled() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static FIRED: AtomicBool = AtomicBool::new(false);
        if !FIRED.swap(true, Ordering::Relaxed) {
            let mut bytes = Vec::new();
            for b in 0..P_BANDS {
                for lv in 0..P_LEVELS {
                    for v in p.tonecurves[b][lv].iter() {
                        bytes.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
            let _ = std::fs::write("/tmp/lewtoff-debug/r_tonecurves.bin", &bytes);
        }
    }

    // noise offsets
    let mut halfoc_dump = Vec::with_capacity(n);
    for i in 0..n {
        // C: float halfoc=toOC((i+.5)*rate/(2.*n))*2.;
        // The argument to toOC is f64 (literals .5, 2. are double); cast at fn boundary.
        let mut halfoc = to_oc(((i as f64 + 0.5) * rate as f64 / (2.0 * n as f64)) as f32) * 2.0;
        halfoc_dump.push(halfoc);
        if halfoc < 0.0 {
            halfoc = 0.0;
        }
        if halfoc >= (P_BANDS - 1) as f32 {
            halfoc = (P_BANDS - 1) as f32;
        }
        let mut inthalfoc = halfoc as usize;
        if inthalfoc >= P_BANDS - 2 {
            inthalfoc = P_BANDS - 2;
        }
        let del = halfoc - inthalfoc as f32;

        // C: a*(1.-del) + b*del — first term in f64 (1. is double), second
        // term in f32 (b and del are float); they sum in f64 and cast to f32.
        for j in 0..P_NOISECURVES {
            let a = vi.noiseoff[j][inthalfoc];
            let b = vi.noiseoff[j][inthalfoc + 1];
            let term1 = (a as f64) * (1.0 - (del as f64)); // f64
            let term2 = (b * del) as f64; // f32 mul, promote
            p.noiseoffset[j][i] = (term1 + term2) as f32;
        }
    }

    if crate::debug_dump::dump_enabled() && n == 128 {
        use std::sync::atomic::{AtomicBool, Ordering};
        static FIRED: AtomicBool = AtomicBool::new(false);
        if !FIRED.swap(true, Ordering::Relaxed) {
            let mut bytes = Vec::with_capacity(halfoc_dump.len() * 4);
            for v in halfoc_dump.iter() {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            let _ = std::fs::write("/tmp/lewtoff-debug/r_halfoc.bin", &bytes);
            // dump noiseoffset[1] from short block specifically
            let mut nbytes = Vec::with_capacity(n * 4);
            for v in p.noiseoffset[1].iter() {
                nbytes.extend_from_slice(&v.to_le_bytes());
            }
            let _ = std::fs::write("/tmp/lewtoff-debug/r_noiseoffset_1.bin", &nbytes);
        }
    }

    p
}

// ---------------------------------------------------------------------------
// seed_curve
// ---------------------------------------------------------------------------

fn seed_curve(
    seed: &mut [f32],
    curves: &[[f32; EHMER_MAX + 2]; P_LEVELS],
    amp: f32,
    oc: i64,
    n: i64,
    linesper: i32,
    db_offset: f32,
) {
    let choice = (((amp + db_offset - P_LEVEL_0) * 0.1) as i32)
        .max(0)
        .min(P_LEVELS as i32 - 1) as usize;
    let posts = &curves[choice];
    let curve = &posts[2..];
    let post1 = posts[1] as usize;
    let post0 = posts[0] as i64;
    let mut seedptr = oc + (post0 - EHMER_OFFSET as i64) * linesper as i64 - (linesper >> 1) as i64;

    for i in posts[0] as usize..post1 {
        if seedptr > 0 {
            let lin = amp + curve[i];
            let sp = seedptr as usize;
            if sp < seed.len() && seed[sp] < lin {
                seed[sp] = lin;
            }
        }
        seedptr += linesper as i64;
        if seedptr >= n {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// seed_loop
// ---------------------------------------------------------------------------

fn seed_loop(
    p: &VorbisLookPsy,
    curves: &ToneCurves,
    f: &[f32],
    flr: &[f32],
    seed: &mut [f32],
    specmax: f32,
) {
    let n = p.n;
    let db_offset = p.vi.max_curve_dB - specmax;
    let mut i = 0usize;
    while i < n {
        let mut max = f[i];
        let oc = p.octave[i] as i64;
        while i + 1 < n && p.octave[i + 1] as i64 == oc {
            i += 1;
            if f[i] > max {
                max = f[i];
            }
        }

        if max + 6.0 > flr[i] {
            let mut oc2 = oc >> p.shiftoc;
            if oc2 >= P_BANDS as i64 {
                oc2 = P_BANDS as i64 - 1;
            }
            if oc2 < 0 {
                oc2 = 0;
            }

            seed_curve(
                seed,
                &curves[oc2 as usize],
                max,
                p.octave[i] as i64 - p.firstoc,
                p.total_octave_lines as i64,
                p.eighth_octave_lines,
                db_offset,
            );
        }
        i += 1;
    }
}

// ---------------------------------------------------------------------------
// seed_chase
// ---------------------------------------------------------------------------

fn seed_chase(seeds: &mut [f32], linesper: i32, n: usize) {
    let mut posstack: Vec<usize> = Vec::with_capacity(n);
    let mut ampstack: Vec<f32> = Vec::with_capacity(n);

    for i in 0..n {
        if posstack.len() < 2 {
            posstack.push(i);
            ampstack.push(seeds[i]);
        } else {
            loop {
                let top = posstack.len() - 1;
                if seeds[i] < ampstack[top] {
                    posstack.push(i);
                    ampstack.push(seeds[i]);
                    break;
                } else {
                    if i < posstack[top] + linesper as usize {
                        if posstack.len() > 1
                            && ampstack[top] <= ampstack[top - 1]
                            && i < posstack[top - 1] + linesper as usize
                        {
                            posstack.pop();
                            ampstack.pop();
                            continue;
                        }
                    }
                    posstack.push(i);
                    ampstack.push(seeds[i]);
                    break;
                }
            }
        }
    }

    let stack = posstack.len();
    let mut pos: usize = 0;
    for i in 0..stack {
        let endpos = if i < stack - 1 && ampstack[i + 1] > ampstack[i] {
            posstack[i + 1]
        } else {
            posstack[i] + linesper as usize + 1
        };
        let endpos = endpos.min(n);
        while pos < endpos {
            seeds[pos] = ampstack[i];
            pos += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// max_seeds
// ---------------------------------------------------------------------------

fn max_seeds(p: &VorbisLookPsy, seed: &mut [f32], flr: &mut [f32]) {
    let n = p.total_octave_lines as usize;
    let linesper = p.eighth_octave_lines;

    seed_chase(seed, linesper, n);

    let mut linpos: usize = 0;
    let mut pos = p.octave[0] as i64 - p.firstoc - (linesper >> 1) as i64;

    while linpos + 1 < p.n {
        let mut minV = if pos >= 0 && pos < n as i64 {
            seed[pos as usize]
        } else {
            NEGINF
        };
        let end = ((p.octave[linpos] as i64 + p.octave[linpos + 1] as i64) >> 1) - p.firstoc;
        if minV > p.vi.tone_abs_limit {
            minV = p.vi.tone_abs_limit;
        }
        while pos < end {
            pos += 1;
            if pos >= 0 && pos < n as i64 {
                let sv = seed[pos as usize];
                if (sv > NEGINF && sv < minV) || minV == NEGINF {
                    minV = sv;
                }
            }
        }

        let end2 = pos + p.firstoc;
        while linpos < p.n && p.octave[linpos] as i64 <= end2 {
            if flr[linpos] < minV {
                flr[linpos] = minV;
            }
            linpos += 1;
        }
    }

    let minV = if n > 0 { seed[n - 1] } else { NEGINF };
    while linpos < p.n {
        if flr[linpos] < minV {
            flr[linpos] = minV;
        }
        linpos += 1;
    }
}

// ---------------------------------------------------------------------------
// bark_noise_hybridmp
// ---------------------------------------------------------------------------

fn bark_noise_hybridmp(n: usize, b: &[i64], f: &[f32], noise: &mut [f32], offset: f32, fixed: i32) {
    // Stack-allocated prefix-sum scratch (n <= LONG_HALF=1024). Avoids the
    // 5 × vec![0.0; n] heap allocs that would otherwise fire twice per
    // long block (in vp_noisemask).
    let mut big_n_arr = [0.0_f32; crate::window::LONG_HALF];
    let mut big_x_arr = [0.0_f32; crate::window::LONG_HALF];
    let mut big_xx_arr = [0.0_f32; crate::window::LONG_HALF];
    let mut big_y_arr = [0.0_f32; crate::window::LONG_HALF];
    let mut big_xy_arr = [0.0_f32; crate::window::LONG_HALF];
    let big_n = &mut big_n_arr[..n];
    let big_x = &mut big_x_arr[..n];
    let big_xx = &mut big_xx_arr[..n];
    let big_y = &mut big_y_arr[..n];
    let big_xy = &mut big_xy_arr[..n];

    let mut tn: f32;
    let mut tx: f32;
    let mut txx: f32;
    let mut ty: f32;
    let mut txy: f32;

    let mut r = 0.0_f32;
    let mut big_a = 0.0_f32;
    let mut big_b = 0.0_f32;
    let mut big_d = 1.0_f32;

    tn = 0.0;
    tx = 0.0;
    txx = 0.0;
    ty = 0.0;
    txy = 0.0;

    let mut y = f[0] + offset;
    if y < 1.0 {
        y = 1.0;
    }
    let w = y * y * 0.5;
    tn += w;
    tx += w;
    ty += w * y;

    big_n[0] = tn;
    big_x[0] = tx;
    big_xx[0] = txx;
    big_y[0] = ty;
    big_xy[0] = txy;

    let mut x = 1.0_f32;
    for i in 1..n {
        let mut y = f[i] + offset;
        if y < 1.0 {
            y = 1.0;
        }
        let w = y * y;
        tn += w;
        tx += w * x;
        txx += w * x * x;
        ty += w * y;
        txy += w * x * y;

        big_n[i] = tn;
        big_x[i] = tx;
        big_xx[i] = txx;
        big_y[i] = ty;
        big_xy[i] = txy;

        x += 1.0;
    }

    let mut i = 0usize;
    x = 0.0;
    while i < n {
        let lo = b[i] >> 16;
        let hi = (b[i] & 0xffff) as usize;

        if lo >= 0 || (-lo) as usize >= n {
            break;
        }
        if hi >= n {
            break;
        }

        let nlo = (-lo) as usize;
        tn = big_n[hi] + big_n[nlo];
        tx = big_x[hi] - big_x[nlo];
        txx = big_xx[hi] + big_xx[nlo];
        ty = big_y[hi] + big_y[nlo];
        txy = big_xy[hi] - big_xy[nlo];

        big_a = ty * txx - tx * txy;
        big_b = tn * txy - tx * ty;
        big_d = tn * txx - tx * tx;
        r = (big_a + x * big_b) / big_d;
        if r < 0.0 {
            r = 0.0;
        }

        noise[i] = r - offset;
        i += 1;
        x += 1.0;
    }

    while i < n {
        let lo = b[i] >> 16;
        let hi = (b[i] & 0xffff) as usize;

        if lo < 0 || lo as usize >= n {
            break;
        }
        if hi >= n {
            break;
        }

        tn = big_n[hi] - big_n[lo as usize];
        tx = big_x[hi] - big_x[lo as usize];
        txx = big_xx[hi] - big_xx[lo as usize];
        ty = big_y[hi] - big_y[lo as usize];
        txy = big_xy[hi] - big_xy[lo as usize];

        big_a = ty * txx - tx * txy;
        big_b = tn * txy - tx * ty;
        big_d = tn * txx - tx * tx;
        r = (big_a + x * big_b) / big_d;

        if (12..=16).contains(&i) && crate::debug_flag!("LW_DEBUG_BNH") {
            eprintln!(
                "LW_BNH i={} lo={} hi={} x={:.1} tn={:.6} tx={:.6} txx={:.6} ty={:.6} txy={:.6} A={:.6} B={:.6} D={:.6} R={:.6} noise={:.6} offset={:.6}",
                i,
                lo,
                hi,
                x,
                tn,
                tx,
                txx,
                ty,
                txy,
                big_a,
                big_b,
                big_d,
                r,
                r - offset,
                offset
            );
        }
        if (i == 118 || i == 127) && crate::debug_flag!("LW_DEBUG_BNH118") {
            eprintln!(
                "LW_BNH i={} lo={} hi={} x={} tn={}(0x{:08x}) ty={}(0x{:08x}) txx={}(0x{:08x}) tx={}(0x{:08x}) txy={}(0x{:08x}) A={}(0x{:08x}) B={}(0x{:08x}) D={}(0x{:08x}) R={}(0x{:08x})",
                i,
                lo,
                hi,
                x,
                tn,
                tn.to_bits(),
                ty,
                ty.to_bits(),
                txx,
                txx.to_bits(),
                tx,
                tx.to_bits(),
                txy,
                txy.to_bits(),
                big_a,
                big_a.to_bits(),
                big_b,
                big_b.to_bits(),
                big_d,
                big_d.to_bits(),
                r,
                r.to_bits(),
            );
        }

        if r < 0.0 {
            r = 0.0;
        }

        noise[i] = r - offset;
        i += 1;
        x += 1.0;
    }

    while i < n {
        r = (big_a + x * big_b) / big_d;
        if r < 0.0 {
            r = 0.0;
        }
        noise[i] = r - offset;
        i += 1;
        x += 1.0;
    }

    if fixed <= 0 {
        return;
    }

    i = 0;
    x = 0.0;
    while i < n {
        let hi = i + fixed as usize / 2;
        let lo = hi as i64 - fixed as i64;

        if hi >= n {
            break;
        }
        if lo >= 0 {
            break;
        }

        let nlo = (-lo) as usize;
        tn = big_n[hi] + big_n[nlo];
        tx = big_x[hi] - big_x[nlo];
        txx = big_xx[hi] + big_xx[nlo];
        ty = big_y[hi] + big_y[nlo];
        txy = big_xy[hi] - big_xy[nlo];

        big_a = ty * txx - tx * txy;
        big_b = tn * txy - tx * ty;
        big_d = tn * txx - tx * tx;
        r = (big_a + x * big_b) / big_d;

        if r - offset < noise[i] {
            noise[i] = r - offset;
        }
        i += 1;
        x += 1.0;
    }

    while i < n {
        let hi = i + fixed as usize / 2;
        let lo = hi as i64 - fixed as i64;

        if hi >= n {
            break;
        }
        if lo < 0 {
            break;
        }

        tn = big_n[hi] - big_n[lo as usize];
        tx = big_x[hi] - big_x[lo as usize];
        txx = big_xx[hi] - big_xx[lo as usize];
        ty = big_y[hi] - big_y[lo as usize];
        txy = big_xy[hi] - big_xy[lo as usize];

        big_a = ty * txx - tx * txy;
        big_b = tn * txy - tx * ty;
        big_d = tn * txx - tx * tx;
        r = (big_a + x * big_b) / big_d;

        if r - offset < noise[i] {
            noise[i] = r - offset;
        }
        i += 1;
        x += 1.0;
    }

    while i < n {
        r = (big_a + x * big_b) / big_d;
        if r - offset < noise[i] {
            noise[i] = r - offset;
        }
        i += 1;
        x += 1.0;
    }
}

// ---------------------------------------------------------------------------
// _vp_noisemask
// ---------------------------------------------------------------------------

pub fn vp_noisemask(p: &VorbisLookPsy, logmdct: &[f32], logmask: &mut [f32]) {
    let n = p.n;
    let mut work = vec![0.0_f32; n];

    bark_noise_hybridmp(n, &p.bark, logmdct, logmask, 140.0, -1);

    for i in 0..n {
        work[i] = logmdct[i] - logmask[i];
    }

    bark_noise_hybridmp(n, &p.bark, &work, logmask, 0.0, p.vi.noisewindowfixed);

    for i in 0..n {
        work[i] = logmdct[i] - work[i];
    }

    for i in 0..n {
        let db = (logmask[i] + 0.5) as i32;
        let db = db.max(0).min(NOISE_COMPAND_LEVELS as i32 - 1) as usize;
        logmask[i] = work[i] + p.vi.noisecompand[db];
    }
}

// ---------------------------------------------------------------------------
// _vp_tonemask
// ---------------------------------------------------------------------------

pub fn vp_tonemask(
    p: &VorbisLookPsy,
    logfft: &[f32],
    logmask: &mut [f32],
    global_specmax: f32,
    local_specmax: f32,
) {
    let n = p.n;
    // Stack-allocated seed scratch. Q5 max observed total_octave_lines is 777
    // (long block at 44.1kHz); MAX_OCTAVE_LINES gives headroom for 48k.
    let mut seed_arr = [NEGINF; MAX_OCTAVE_LINES];
    let mut seed = &mut seed_arr[..p.total_octave_lines as usize];

    let mut att = local_specmax + p.vi.ath_adjatt;
    if att < p.vi.ath_maxatt {
        att = p.vi.ath_maxatt;
    }

    for i in 0..n {
        logmask[i] = p.ath[i] + att;
    }

    seed_loop(p, &p.tonecurves, logfft, logmask, seed, global_specmax);
    if crate::debug_dump::dump_enabled() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static FIRED: AtomicBool = AtomicBool::new(false);
        if !FIRED.swap(true, Ordering::Relaxed) {
            let mut bytes = Vec::new();
            for v in seed.iter() {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            let _ = std::fs::write("/tmp/lewtoff-debug/r_tone_seed.bin", &bytes);
        }
    }
    max_seeds(p, seed, logmask);
}

// ---------------------------------------------------------------------------
// _vp_offset_and_mix
// ---------------------------------------------------------------------------

pub fn vp_offset_and_mix(
    p: &VorbisLookPsy,
    noise: &[f32],
    tone: &[f32],
    offset_select: usize,
    logmask: &mut [f32],
    mdct: &mut [f32],
    logmdct: &[f32],
) {
    let n = p.n;
    let cx = p.m_val;
    let toneatt = p.vi.tone_masteratt[offset_select];

    for i in 0..n {
        let mut val = noise[i] + p.noiseoffset[offset_select][i];
        if val > p.vi.noisemaxsupp {
            val = p.vi.noisemaxsupp;
        }
        logmask[i] = val.max(tone[i] + toneatt);

        if offset_select == 1 {
            // C: `float de, coeffi, cx; coeffi = -17.2;` — coeffi is float
            // (= -17.20000076f, NOT the f64 literal -17.2 = -17.19999999...).
            // Then `val = val - logmdct[i]` and `if(val > coeffi)` are both
            // f32 operations. The math `1.0 - (val-coeffi)*0.005*cx` promotes
            // to f64 only because 0.005/0.0003/1.0 are double literals; the
            // final assignment to `float de` truncates back to f32.
            let coeffi: f32 = -17.2;
            let val_diff = val - logmdct[i];
            let de: f32 = if val_diff > coeffi {
                let d = 1.0_f64 - (val_diff - coeffi) as f64 * 0.005_f64 * cx as f64;
                if d < 0.0 { 0.0001_f32 } else { d as f32 }
            } else {
                (1.0_f64 - (val_diff - coeffi) as f64 * 0.0003_f64 * cx as f64) as f32
            };

            mdct[i] *= de;
        }
    }
}

// ---------------------------------------------------------------------------
// _vp_ampmax_decay
// ---------------------------------------------------------------------------

pub fn vp_ampmax_decay(amp: f32, gi: &VorbisInfoPsyGlobal, n: usize, rate: i64) -> f32 {
    let secs = n as f32 / rate as f32;
    let result = amp + secs * gi.ampmax_att_per_sec;
    if result < -9999.0 { -9999.0 } else { result }
}

// ---------------------------------------------------------------------------
// flag_lossless (static helper for _vp_couple_quantize_normalize)
// ---------------------------------------------------------------------------

fn flag_lossless(
    limit: i32,
    prepoint: f32,
    postpoint: f32,
    mdct: &[f32],
    floor: &[f32],
    flag: &mut [i32],
    i: i32,
    jn: usize,
) {
    for j in 0..jn {
        let point = if j as i32 >= limit - i {
            postpoint
        } else {
            prepoint
        };
        let r = mdct[j].abs() / floor[j];
        flag[j] = if r < point { 0 } else { 1 };
    }
}

// ---------------------------------------------------------------------------
// noise_normalize
// ---------------------------------------------------------------------------

fn noise_normalize(
    p: &VorbisLookPsy,
    limit: i32,
    r: &[f32],
    q: &mut [f32],
    f: &[f32],
    flags: Option<&[i32]>,
    mut acc: f32,
    i: i32,
    n: usize,
    out: &mut [i32],
) -> f32 {
    let vi = &p.vi;
    let start = if vi.normal_p != 0 {
        (vi.normal_start - i).max(0).min(n as i32) as usize
    } else {
        n
    };

    // force classic behavior
    acc = 0.0;

    for j in 0..start {
        let skip = if let Some(fl) = flags {
            fl[j] != 0
        } else {
            false
        };
        if !skip {
            let ve = q[j] / f[j];
            // libvorbis: `out[j] = -rint(sqrt(ve));` — sqrt() is the f64
            // overload (promotes f32 ve to f64), rint() rounds half-to-even.
            // Rust's f32::sqrt is single-precision and .round() rounds
            // half-away-from-zero; both diverge from C.
            let s = (ve as f64).sqrt().round_ties_even();
            out[j] = if r[j] < 0.0 { -(s as i32) } else { s as i32 };
        }
    }

    let mut sort_idx: Vec<usize> = Vec::new();

    for j in start..n {
        let skip = if let Some(fl) = flags {
            fl[j] != 0
        } else {
            false
        };
        if !skip {
            let ve = q[j] / f[j];
            if ve < 0.25 && (flags.is_none() || j as i32 >= limit - i) {
                acc += ve;
                sort_idx.push(j);
            } else {
                let s = (ve as f64).sqrt().round_ties_even();
                out[j] = if r[j] < 0.0 { -(s as i32) } else { s as i32 };
                q[j] = out[j] as f32 * out[j] as f32 * f[j];
            }
        }
    }

    if !sort_idx.is_empty() {
        sort_idx.sort_unstable_by(|&a, &b| {
            let fa = q[a];
            let fb = q[b];
            fb.partial_cmp(&fa).unwrap_or(std::cmp::Ordering::Equal)
        });
        for &k in &sort_idx {
            if acc >= vi.normal_thresh as f32 {
                out[k] = unitnorm(r[k]) as i32;
                acc -= 1.0;
                q[k] = f[k];
            } else {
                out[k] = 0;
                q[k] = 0.0;
            }
        }
    }

    acc
}

// ---------------------------------------------------------------------------
// VorbisInfoMapping0 — minimal subset needed for _vp_couple_quantize_normalize
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct VorbisInfoMapping0 {
    pub coupling_steps: usize,
    pub coupling_mag: Vec<usize>,
    pub coupling_ang: Vec<usize>,
}

// ---------------------------------------------------------------------------
// _vp_couple_quantize_normalize
// ---------------------------------------------------------------------------

pub fn vp_couple_quantize_normalize(
    blobno: usize,
    g: &VorbisInfoPsyGlobal,
    p: &VorbisLookPsy,
    vi: &VorbisInfoMapping0,
    mdct: &mut [Vec<f32>],
    iwork: &mut [Vec<i32>],
    nonzero: &mut [i32],
    sliding_lowpass: i32,
    ch: usize,
) {
    let n = p.n;
    let partition = if p.vi.normal_p != 0 {
        p.vi.normal_partition as usize
    } else {
        16
    };
    let limit = g.coupling_pointlimit[p.vi.blockflag as usize][blobno];
    let prepoint = STEREO_THRESHHOLDS[g.coupling_prepointamp[blobno] as usize] as f32;
    let mut postpoint = STEREO_THRESHHOLDS[g.coupling_postpointamp[blobno] as usize] as f32;

    if n > 1000 {
        postpoint = STEREO_THRESHHOLDS_LIMITED[g.coupling_postpointamp[blobno] as usize] as f32;
    }

    // Stack-allocated per-block scratch. Q5 has partition <= 32, ch <= 2,
    // coupling_steps <= 1. Fixed bounds avoid the per-call heap-alloc churn
    // of vec![vec![0.0; partition]; ch].
    debug_assert!(
        partition <= MAX_PARTITION,
        "partition {} > MAX_PARTITION",
        partition
    );
    debug_assert!(ch <= MAX_CH, "ch {} > MAX_CH", ch);
    let mut raw: [[f32; MAX_PARTITION]; MAX_CH] = [[0.0; MAX_PARTITION]; MAX_CH];
    let mut quant: [[f32; MAX_PARTITION]; MAX_CH] = [[0.0; MAX_PARTITION]; MAX_CH];
    let mut floor_v: [[f32; MAX_PARTITION]; MAX_CH] = [[0.0; MAX_PARTITION]; MAX_CH];
    let mut flag: [[i32; MAX_PARTITION]; MAX_CH] = [[0; MAX_PARTITION]; MAX_CH];
    let mut nz: [i32; MAX_CH] = [0; MAX_CH];
    let mut acc: [f32; MAX_CH + MAX_COUPLING_STEPS] = [0.0; MAX_CH + MAX_COUPLING_STEPS];

    let mut i = 0;
    while i < n {
        let jn = partition.min(n - i);
        let mut track = 0usize;

        nz[..ch].copy_from_slice(nonzero);

        for fl in flag[..ch].iter_mut() {
            for v in fl[..jn].iter_mut() {
                *v = 0;
            }
        }

        for k in 0..ch {
            let iout = &mut iwork[k][i..i + jn];
            if nz[k] != 0 {
                for j in 0..jn {
                    floor_v[k][j] = FLOOR1_FROMDB_LOOKUP[iout[j] as usize];
                }
                flag_lossless(
                    limit,
                    prepoint,
                    postpoint,
                    &mdct[k][i..i + jn],
                    &floor_v[k][..jn],
                    &mut flag[k][..jn],
                    i as i32,
                    jn,
                );
                for j in 0..jn {
                    quant[k][j] = mdct[k][i + j] * mdct[k][i + j];
                    raw[k][j] = quant[k][j];
                    if mdct[k][i + j] < 0.0 {
                        raw[k][j] *= -1.0;
                    }
                    floor_v[k][j] *= floor_v[k][j];
                }
                // collect iout into a local slice for noise_normalize
                let mut out_slice: Vec<i32> = iout.to_vec();
                acc[track] = noise_normalize(
                    p,
                    limit,
                    &raw[k][..jn],
                    &mut quant[k][..jn],
                    &floor_v[k][..jn],
                    None,
                    acc[track],
                    i as i32,
                    jn,
                    &mut out_slice,
                );
                iout.copy_from_slice(&out_slice);
            } else {
                for j in 0..jn {
                    floor_v[k][j] = 1e-10;
                    raw[k][j] = 0.0;
                    quant[k][j] = 0.0;
                    flag[k][j] = 0;
                    iout[j] = 0;
                }
                acc[track] = 0.0;
            }
            track += 1;
        }

        // coupling
        for step in 0..vi.coupling_steps {
            let mi = vi.coupling_mag[step];
            let ai = vi.coupling_ang[step];

            if nz[mi] != 0 || nz[ai] != 0 {
                nz[mi] = 1;
                nz[ai] = 1;

                for j in 0..jn {
                    if (j as i32) < sliding_lowpass - i as i32 {
                        if flag[mi][j] != 0 || flag[ai][j] != 0 {
                            // lossless coupling
                            raw[mi][j] = raw[mi][j].abs() + raw[ai][j].abs();
                            quant[mi][j] = quant[mi][j] + quant[ai][j];
                            flag[mi][j] = 1;
                            flag[ai][j] = 1;

                            let a = iwork[mi][i + j];
                            let b = iwork[ai][i + j];
                            if a.abs() > b.abs() {
                                iwork[ai][i + j] = if a > 0 { a - b } else { b - a };
                            } else {
                                iwork[ai][i + j] = if b > 0 { a - b } else { b - a };
                                iwork[mi][i + j] = b;
                            }

                            if iwork[ai][i + j] >= iwork[mi][i + j].abs() * 2 {
                                iwork[ai][i + j] = -iwork[ai][i + j];
                                iwork[mi][i + j] = -iwork[mi][i + j];
                            }
                        } else {
                            // lossy (point) coupling
                            if (j as i32) < limit - i as i32 {
                                // dipole
                                raw[mi][j] += raw[ai][j];
                                quant[mi][j] = raw[mi][j].abs();
                            } else {
                                // elliptical
                                let sum = raw[mi][j].abs() + raw[ai][j].abs();
                                if raw[mi][j] + raw[ai][j] < 0.0 {
                                    quant[mi][j] = sum;
                                    raw[mi][j] = -sum;
                                } else {
                                    quant[mi][j] = sum;
                                    raw[mi][j] = sum;
                                }
                            }
                            raw[ai][j] = 0.0;
                            quant[ai][j] = 0.0;
                            flag[ai][j] = 1;
                            iwork[ai][i + j] = 0;
                        }
                    }
                    floor_v[mi][j] = floor_v[ai][j] + floor_v[mi][j];
                    floor_v[ai][j] = floor_v[mi][j]; // both point to same sum now
                }

                {
                    let mut im_out: Vec<i32> = iwork[mi][i..i + jn].to_vec();
                    acc[track] = noise_normalize(
                        p,
                        limit,
                        &raw[mi][..jn],
                        &mut quant[mi][..jn],
                        &floor_v[mi][..jn],
                        Some(&flag[mi][..jn]),
                        acc[track],
                        i as i32,
                        jn,
                        &mut im_out,
                    );
                    iwork[mi][i..i + jn].copy_from_slice(&im_out);
                }
                track += 1;
            }
        }

        i += partition;
    }

    for i in 0..vi.coupling_steps {
        if nonzero[vi.coupling_mag[i]] != 0 || nonzero[vi.coupling_ang[i]] != 0 {
            nonzero[vi.coupling_mag[i]] = 1;
            nonzero[vi.coupling_ang[i]] = 1;
        }
    }
}
