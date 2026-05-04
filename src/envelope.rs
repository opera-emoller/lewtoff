//! Port of libvorbis lib/envelope.{h,c} for transient detection.
//!
//! libvorbis decides per-block whether to use short or long block size based
//! on a 7-band envelope analysis run on a small (n=128) MDCT of overlapping
//! 64-sample windows. Without this, ramp/transient inputs encode as long
//! blocks where libvorbis would emit short impulse blocks.
//!
//! This module mirrors `_ve_envelope_init`, `_ve_amp`, `_ve_envelope_search`
//! and `_ve_envelope_mark` enough to produce the same W-flag sequence that
//! libvorbis would emit for a given input. Numeric details (constants,
//! orderings, decay value, +-99999 sentinels) are kept literal.
//!
//! Used by `encode.rs` to pre-compute the block-size sequence so the linear
//! block loop matches libvorbis's streaming-model output.
#![allow(non_snake_case)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::excessive_precision)]
#![allow(clippy::items_after_test_module)]
#![allow(dead_code)]

use crate::mdct::mdct_forward_envelope;
use crate::psy::VorbisInfoPsyGlobal;
use crate::window::{LONG_BLOCK, SHORT_BLOCK};

pub const VE_PRE: usize = 16;
pub const VE_WIN: usize = 4;
pub const VE_POST: usize = 2;
pub const VE_AMP: usize = VE_PRE + VE_POST - 1;
pub const VE_BANDS: usize = 7;
pub const VE_NEARDC: usize = 15;
pub const VE_MINSTRETCH: i32 = 2;
pub const VE_MAXSTRETCH: i32 = 12;

const ENV_WINLENGTH: usize = 128;
const ENV_SEARCHSTEP: usize = 64;

#[derive(Clone)]
struct FilterState {
    ampbuf: [f32; VE_AMP],
    ampptr: usize,
    near_dc: [f32; VE_NEARDC],
    near_dc_acc: f32,
    near_dc_partialacc: f32,
    nearptr: usize,
}

impl FilterState {
    fn new() -> Self {
        Self {
            ampbuf: [0.0; VE_AMP],
            ampptr: 0,
            near_dc: [0.0; VE_NEARDC],
            near_dc_acc: 0.0,
            near_dc_partialacc: 0.0,
            nearptr: 0,
        }
    }
}

struct Band {
    begin: usize,
    end: usize,
    window: Vec<f32>,
    total: f32,
}

pub struct EnvelopeLookup {
    ch: usize,
    minenergy: f32,
    mdct_win: [f32; ENV_WINLENGTH],
    bands: [Band; VE_BANDS],
    /// `filters[ch * VE_BANDS + band]` = state for that (channel, band).
    filters: Vec<FilterState>,
    stretch: i32,
    /// Envelope marks per searchstep (= 64-sample window). True if a transient
    /// was detected at that position.
    pub marks: Vec<bool>,
}

impl EnvelopeLookup {
    pub fn new(channels: usize, minenergy: f32) -> Self {
        let mut mdct_win = [0.0f32; ENV_WINLENGTH];
        for i in 0..ENV_WINLENGTH {
            // libvorbis:
            //   e->mdct_win[i] = sin(i/(n-1.)*M_PI);  // sin() returns f64,
            //                                         // assigned to f32 truncates
            //   e->mdct_win[i] *= e->mdct_win[i];     // f32 * f32 with already-
            //                                         // truncated value
            // Mirror that exactly: sin in f64, truncate to f32, then square in f32.
            let s = (i as f64 / (ENV_WINLENGTH - 1) as f64 * std::f64::consts::PI).sin() as f32;
            mdct_win[i] = s * s;
        }
        if crate::debug_flag!("LW_DEBUG_MDCT_WIN") {
            for i in 0..8 {
                eprintln!("R_MDCT_WIN[{}] = 0x{:08x}", i, mdct_win[i].to_bits());
            }
        }

        let band_specs: [(usize, usize); VE_BANDS] =
            [(2, 4), (4, 5), (6, 6), (9, 8), (13, 8), (17, 8), (22, 8)];
        let bands: Vec<Band> = band_specs
            .iter()
            .map(|&(b, e)| {
                let n = e;
                let mut win = vec![0.0f32; n];
                let mut total = 0.0f32;
                for i in 0..n {
                    let w = ((i as f64 + 0.5) / n as f64 * std::f64::consts::PI).sin() as f32;
                    win[i] = w;
                    total += w;
                }
                Band {
                    begin: b,
                    end: e,
                    window: win,
                    // libvorbis: `e->band[j].total = 1./e->band[j].total;` —
                    // `1.` is double, so the division promotes to f64.
                    total: (1.0_f64 / total as f64) as f32,
                }
            })
            .collect();
        let bands: [Band; VE_BANDS] = bands.try_into().map_err(|_| ()).expect("VE_BANDS-sized");

        let filters = vec![FilterState::new(); VE_BANDS * channels];

        Self {
            ch: channels,
            minenergy,
            mdct_win,
            bands,
            filters,
            stretch: 0,
            marks: Vec::new(),
        }
    }

    /// Run envelope amplitude analysis on a 128-sample window.
    /// Updates `self.filters[band_offset..]` in place; returns the trigger
    /// flags (bit 0 = preecho hi, bit 1 = postecho lo, bit 2 = strong).
    fn amp(&mut self, gi: &VorbisInfoPsyGlobal, data: &[f32], band_offset: usize) -> i32 {
        let mut ret = 0i32;
        let stretch = VE_MINSTRETCH.max(self.stretch / 2);
        let mut penalty = gi.stretch_penalty - (self.stretch / 2 - VE_MINSTRETCH) as f32;
        if penalty < 0.0 {
            penalty = 0.0;
        }
        if penalty > gi.stretch_penalty {
            penalty = gi.stretch_penalty;
        }

        // window + MDCT in place
        let mut vec = [0.0f32; ENV_WINLENGTH];
        for i in 0..ENV_WINLENGTH {
            vec[i] = data[i] * self.mdct_win[i];
        }
        let mut vec_in: [f32; ENV_WINLENGTH] = vec;
        let mut vec_out = [0.0f32; ENV_WINLENGTH / 2];
        if crate::debug_flag!("LW_DEBUG_AMP_DBG") {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            if (32..=38).contains(&n) {
                let mut s = format!("R_AMP_DBG call={} windowed_pcm_first16:", n);
                for z in 0..16 {
                    s.push_str(&format!(" 0x{:08x}", vec_in[z].to_bits()));
                }
                eprintln!("{}", s);
            }
            mdct_forward_envelope(&vec_in, &mut vec_out);
            if (32..=38).contains(&n) {
                let mut s = format!("R_AMP_DBG call={} mdct_out_first16:", n);
                for z in 0..16 {
                    s.push_str(&format!(" 0x{:08x}", vec_out[z].to_bits()));
                }
                eprintln!("{}", s);
            }
        } else {
            mdct_forward_envelope(&vec_in, &mut vec_out);
        }
        // Reuse vec to hold output spread/limited values across band loop:
        // first half stores spreaded values, second half unused.
        let _ = &mut vec_in;

        // Near-DC accumulator decay term.
        let decay_init = {
            let band0 = self.filters.get_mut(band_offset).unwrap();
            // libvorbis: `temp = vec[0]*vec[0] + .7*vec[1]*vec[1] + .2*vec[2]*vec[2]`
            // The first term `vec[0]*vec[0]` is float*float (f32) since both
            // operands are float — `.7` only promotes its own multiplication
            // chain. The middle terms `.7*vec[1]*vec[1]` and `.2*vec[2]*vec[2]`
            // are evaluated in f64 (because `.7`/`.2` are double). The final
            // `f32 + f64 + f64` happens in f64; assignment to `float temp`
            // truncates back to f32.
            let term0 = vec_out[0] * vec_out[0]; // f32 * f32 = f32
            let term1 = 0.7_f64 * vec_out[1] as f64 * vec_out[1] as f64;
            let term2 = 0.2_f64 * vec_out[2] as f64 * vec_out[2] as f64;
            let temp = (term0 as f64 + term1 + term2) as f32;
            if crate::debug_flag!("LW_DEBUG_NEARDC") {
                use std::sync::atomic::{AtomicUsize, Ordering};
                static N: AtomicUsize = AtomicUsize::new(0);
                let n = N.fetch_add(1, Ordering::Relaxed);
                if n < 50 {
                    eprintln!(
                        "R_NEARDC call={} v0=0x{:08x} v1=0x{:08x} v2=0x{:08x} temp=0x{:08x}",
                        n,
                        vec_out[0].to_bits(),
                        vec_out[1].to_bits(),
                        vec_out[2].to_bits(),
                        temp.to_bits()
                    );
                }
            }
            let ptr = band0.nearptr;
            let decay;
            if ptr == 0 {
                band0.near_dc_acc = band0.near_dc_partialacc + temp;
                decay = band0.near_dc_acc;
                band0.near_dc_partialacc = temp;
            } else {
                band0.near_dc_acc += temp;
                decay = band0.near_dc_acc;
                band0.near_dc_partialacc += temp;
            }
            band0.near_dc_acc -= band0.near_dc[ptr];
            band0.near_dc[ptr] = temp;
            band0.nearptr += 1;
            if band0.nearptr >= VE_NEARDC {
                band0.nearptr = 0;
            }
            // libvorbis: `decay *= (1./(VE_NEARDC+1));` — `1./16` is double,
            // so decay promotes to f64, multiplies, truncates back to f32.
            let decay_scaled = ((decay as f64) * (1.0_f64 / (VE_NEARDC + 1) as f64)) as f32;
            // libvorbis: `decay = todB(&decay)*.5-15.f` where todB returns
            // FLOAT (the inline IEEE_FLOAT32 path). `.5` is double, so the
            // float result promotes to double for the multiply, then `15.f`
            // promotes to double for the subtract; assignment to float
            // truncates. Mirror that promotion chain explicitly.
            let decay_db = (todb(decay_scaled) as f64 * 0.5_f64 - 15.0_f32 as f64) as f32;
            if crate::debug_flag!("LW_DEBUG_DECAY") {
                use std::sync::atomic::{AtomicUsize, Ordering};
                static N: AtomicUsize = AtomicUsize::new(0);
                let n = N.fetch_add(1, Ordering::Relaxed);
                if n < 40 {
                    eprintln!(
                        "R_DECAY call={} decay_in=0x{:08x} decay_scaled=0x{:08x} decay_db=0x{:08x} acc=0x{:08x} pacc=0x{:08x}",
                        n,
                        decay.to_bits(),
                        decay_scaled.to_bits(),
                        decay_db.to_bits(),
                        band0.near_dc_acc.to_bits(),
                        band0.near_dc_partialacc.to_bits(),
                    );
                }
            }
            decay_db
        };

        // Spread + limit. Output stored in vec_out[i>>1] (half-size).
        let n2 = ENV_WINLENGTH / 2;
        let mut spread = [0.0f32; ENV_WINLENGTH / 4];
        let mut decay = decay_init;
        let minv = self.minenergy;
        let mut i = 0usize;
        while i < n2 {
            let val = vec_out[i] * vec_out[i] + vec_out[i + 1] * vec_out[i + 1];
            // libvorbis: `val = todB(&val)*.5f` — todB returns FLOAT, `.5f`
            // is FLOAT, so this is a pure f32 multiplication.
            let mut val_db = todb(val) * 0.5_f32;
            if val_db < decay {
                val_db = decay;
            }
            if val_db < minv {
                val_db = minv;
            }
            spread[i >> 1] = val_db;
            // libvorbis: `decay -= 8.;` — `8.` is double, so decay promotes
            // to f64, subtracts, truncates back. Mirror to keep precision
            // aligned. (Each j's decay starts as `decay_init` and decrements
            // by 8 per pair, so any drift here accumulates.)
            decay = ((decay as f64) - 8.0_f64) as f32;
            i += 2;
        }

        if crate::debug_flag!("LW_DEBUG_SPREAD") {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            if (32..=36).contains(&n) {
                let mut s = format!(
                    "R_SPREAD call={} decay_init=0x{:08x} spread:",
                    n,
                    decay_init.to_bits()
                );
                for k in 0..16 {
                    s.push_str(&format!(" 0x{:08x}", spread[k].to_bits()));
                }
                eprintln!("{}", s);
            }
        }
        // Per-band trigger detection.
        for j in 0..VE_BANDS {
            let band = &self.bands[j];
            let filter = &mut self.filters[band_offset + j];

            let mut acc = 0.0f32;
            for k in 0..band.end {
                acc += spread[k + band.begin] * band.window[k];
            }
            acc *= band.total;

            let this = filter.ampptr;
            let mut p = if this == 0 { VE_AMP - 1 } else { this - 1 };
            let postmax = acc.max(filter.ampbuf[p]);
            let postmin = acc.min(filter.ampbuf[p]);

            let mut premax = -99999.0f32;
            let mut premin = 99999.0f32;
            for _ in 0..stretch {
                if p == 0 {
                    p = VE_AMP - 1;
                } else {
                    p -= 1;
                }
                if filter.ampbuf[p] > premax {
                    premax = filter.ampbuf[p];
                }
                if filter.ampbuf[p] < premin {
                    premin = filter.ampbuf[p];
                }
            }

            let valmin = postmin - premin;
            let valmax = postmax - premax;

            filter.ampbuf[this] = acc;
            filter.ampptr += 1;
            if filter.ampptr >= VE_AMP {
                filter.ampptr = 0;
            }

            if valmax > gi.preecho_thresh[j] + penalty {
                ret |= 1;
                ret |= 4;
            }
            if valmin < gi.postecho_thresh[j] - penalty {
                ret |= 2;
            }
            if crate::debug_flag!("LW_DEBUG_AMP") {
                use std::sync::atomic::{AtomicUsize, Ordering};
                static N: AtomicUsize = AtomicUsize::new(0);
                let n = N.fetch_add(1, Ordering::Relaxed);
                if n < 2000 {
                    eprintln!(
                        "R_AMP band={} acc=0x{:08x} postmax=0x{:08x} premax=0x{:08x} valmax={:.6} thresh={:.6} stretch={}",
                        j,
                        acc.to_bits(),
                        postmax.to_bits(),
                        premax.to_bits(),
                        valmax,
                        gi.preecho_thresh[j],
                        stretch,
                    );
                }
            }
        }

        ret
    }
}

/// libvorbis `todB` (with VORBIS_IEEE_FLOAT32 defined, the actual path used):
///
/// ```c
/// static inline float todB(const float *x){
///   union { ogg_uint32_t i; float f; } ix;
///   ix.f = *x;
///   ix.i = ix.i & 0x7fffffff;          // absolute value
///   return (float)(ix.i * 7.17711438e-7f - 764.6161886f);
/// }
/// ```
///
/// Takes the absolute value, then uses **f32** constants (`7.17711438e-7f`,
/// `764.6161886f`) so the entire expression evaluates in f32. The cast to
/// float at the end is a no-op since everything is already f32. Earlier I
/// ported the macro form (returns f64), which is dead code in this build —
/// the inline function above is what's actually compiled.
#[inline]
fn todb(x: f32) -> f32 {
    let bits = x.to_bits() & 0x7fff_ffff;
    (bits as f32) * 7.17711438e-7_f32 - 764.6161886_f32
}

/// Compute the envelope-mark vector for an entire stream.
///
/// This runs `_ve_amp` over each 64-sample step of the input and returns a
/// boolean per step: true if libvorbis would have set `mark[j]` at that step.
/// The encoder uses these marks to decide block sizes per the libvorbis
/// `_ve_envelope_search` rule.
pub fn compute_marks(pcm_channels: &[Vec<f32>], gi: &VorbisInfoPsyGlobal) -> Vec<bool> {
    let ch = pcm_channels.len();
    if ch == 0 {
        return Vec::new();
    }
    let n_samples = pcm_channels[0].len();
    if n_samples < ENV_WINLENGTH {
        return Vec::new();
    }

    let mut e = EnvelopeLookup::new(ch, gi.preecho_minenergy);

    // libvorbis: `int last = v->pcm_current/searchstep - VE_WIN;`
    // The for loop processes j=0..last-1. Mirror that exactly so we don't
    // emit spurious marks for steps libvorbis skips at the buffer edge.
    let n_steps = (n_samples / ENV_SEARCHSTEP).saturating_sub(VE_WIN);
    let mut marks = vec![false; n_steps + VE_POST + 1];

    let dbg = crate::debug_flag!("LW_DEBUG_ENV");
    for j in 0..n_steps {
        let mut ret = 0i32;
        e.stretch += 1;
        if e.stretch > VE_MAXSTRETCH * 2 {
            e.stretch = VE_MAXSTRETCH * 2;
        }
        let off = j * ENV_SEARCHSTEP;
        for i in 0..ch {
            let pcm = &pcm_channels[i][off..off + ENV_WINLENGTH];
            ret |= e.amp(gi, pcm, i * VE_BANDS);
        }
        marks[j + VE_POST] = false;
        if (ret & 1) != 0 {
            marks[j] = true;
            if j + 1 < marks.len() {
                marks[j + 1] = true;
            }
        }
        if (ret & 2) != 0 {
            marks[j] = true;
            if j > 0 {
                marks[j - 1] = true;
            }
        }
        if (ret & 4) != 0 {
            e.stretch = -1;
        }
        if dbg && ret != 0 {
            eprintln!("R_ENV_MARK j={} ret={} stretch={}", j, ret, e.stretch);
        }
    }

    marks
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::psy::VorbisInfoPsyGlobal;

    fn make_q5_global() -> VorbisInfoPsyGlobal {
        let mut gi = VorbisInfoPsyGlobal::default();
        gi.eighth_octave_lines = 8;
        gi.preecho_thresh = [12.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0];
        gi.postecho_thresh = [-20.0, -20.0, -15.0, -15.0, -15.0, -15.0, -15.0];
        gi.stretch_penalty = 0.0;
        gi.preecho_minenergy = -80.0;
        gi.ampmax_att_per_sec = -6.0;
        gi
    }

    #[test]
    fn silence_no_marks() {
        let pcm = vec![vec![0.0f32; 44100]];
        let gi = make_q5_global();
        let marks = compute_marks(&pcm, &gi);
        let first_marks: Vec<usize> = marks
            .iter()
            .enumerate()
            .filter(|(_, &b)| b)
            .map(|(i, _)| i)
            .take(10)
            .collect();
        eprintln!("silence first marks at indices: {:?}", first_marks);
        assert_eq!(n_short_blocks_at_start(&marks), 1, "silence: 1 short");
    }

    #[test]
    fn ramp_full_pattern_has_midstream_shorts() {
        use crate::encode::pre_extrap_for_test;
        let n = 44100usize;
        let raw_l: Vec<f32> = (0..n)
            .map(|i| (((i * 2) % 65536) as i32 - 32768) as f32 / 32768.0)
            .collect();
        let raw_r: Vec<f32> = (0..n)
            .map(|i| (((i * 2 + 1) % 65536) as i32 - 32768) as f32 / 32768.0)
            .collect();
        let pcm: Vec<Vec<f32>> = [&raw_l, &raw_r]
            .iter()
            .map(|raw| {
                let mut buf = Vec::with_capacity(1024 + raw.len());
                let pre = pre_extrap_for_test(&raw[..2112.min(raw.len())]);
                let mut pre_rev = vec![0.0f32; 1024];
                for k in 0..1024 {
                    pre_rev[1024 - 1 - k] = pre[k];
                }
                buf.extend_from_slice(&pre_rev);
                buf.extend_from_slice(raw);
                buf
            })
            .collect();
        let gi = make_q5_global();
        let marks = compute_marks(&pcm, &gi);
        // n_samples here is the audio buffer length (including the LPC
        // pre-stream prefix that envelope detection saw).
        let pattern = full_w_pattern(&marks, pcm[0].len() as i64);
        let n_short = pattern.iter().filter(|&&w| w == 0).count();
        eprintln!(
            "ramp pattern len={} n_short={} pattern={:?}",
            pattern.len(),
            n_short,
            pattern
        );
        // Expect at least one mid-stream short cluster (= more than the 2
        // initial short blocks).
        assert!(n_short > 4, "ramp must have mid-stream short blocks");
    }

    #[test]
    fn ramp_two_short() {
        // libvorbis envelope detection runs on the pre-extrapolated PCM
        // buffer: 1024 LPC-predicted virtual samples (centerW) + audio. Mirror
        // that here so envelope state evolves the same way.
        use crate::encode::{post_extrap_for_test, pre_extrap_for_test};

        let n = 44100usize;
        let raw_l: Vec<f32> = (0..n)
            .map(|i| (((i * 2) % 65536) as i32 - 32768) as f32 / 32768.0)
            .collect();
        let raw_r: Vec<f32> = (0..n)
            .map(|i| (((i * 2 + 1) % 65536) as i32 - 32768) as f32 / 32768.0)
            .collect();
        let pcm: Vec<Vec<f32>> = [&raw_l, &raw_r]
            .iter()
            .map(|raw| {
                let mut buf = Vec::with_capacity(1024 + raw.len());
                let pre = pre_extrap_for_test(&raw[..2112.min(raw.len())]);
                // pre[k] corresponds to virtual sample at -(k+1); insert
                // reversed into [0..1024] so that pcm[1024] = audio[0].
                let mut pre_rev = vec![0.0f32; 1024];
                for k in 0..1024 {
                    pre_rev[1024 - 1 - k] = pre[k];
                }
                buf.extend_from_slice(&pre_rev);
                buf.extend_from_slice(raw);
                let _ = post_extrap_for_test;
                buf
            })
            .collect();
        let gi = make_q5_global();
        let marks = compute_marks(&pcm, &gi);
        let first_marks: Vec<usize> = marks
            .iter()
            .enumerate()
            .filter(|(_, &b)| b)
            .map(|(i, _)| i)
            .take(20)
            .collect();
        eprintln!("ramp first marks at indices: {:?}", first_marks);
        let n_short = n_short_blocks_at_start(&marks);
        assert_eq!(n_short, 2, "ramp: 2 short (got {n_short})");
    }
}

/// Reproduce libvorbis's `_ve_envelope_mark` for a short block.
///
/// For a short block at `center_w`, scan the marks vector in
/// `[centerW - bs[0]/2, centerW + bs[0]/2)` = `[centerW-128, centerW+128)`.
/// Any mark in that window → block is IMPULSE (returns true). No mark →
/// PADDING (returns false). Long blocks don't use this — their type is
/// determined by lW/nW.
pub fn short_is_impulse(marks: &[bool], curmark: i64, center_w: i64) -> bool {
    let bs0 = SHORT_BLOCK as i64;
    let step = ENV_SEARCHSTEP as i64;
    let begin_w = center_w - bs0 / 4 - bs0 / 4;
    let end_w = center_w + bs0 / 4 + bs0 / 4;
    // libvorbis fast path: if a recently-set curmark falls in the range,
    // immediately classify as IMPULSE.
    if curmark >= begin_w && curmark < end_w {
        return true;
    }
    let first = (begin_w / step).max(0) as usize;
    let last = ((end_w / step).max(0) as usize).min(marks.len());
    for i in first..last {
        if marks[i] {
            return true;
        }
    }
    false
}

/// Count how many short blocks libvorbis would emit at the start of the
/// stream. For our constrained input space (silence / sine / ramp), this is
/// either 1 (no transient detected past the leading-edge ampbuf-init noise)
/// or 2 (transient detected before testW=2176).
pub fn n_short_blocks_at_start(marks: &[bool]) -> usize {
    // libvorbis _ve_envelope_search: cursor starts at bs1/2 = 1024 (mark
    // index 16). testW for the first decision = centerW + bs[0]/4 + bs[1]/2
    // + bs[0]/4 = 1024 + 64 + 1024 + 64 = 2176 → mark index 34. So only
    // marks at index 16 ≤ i < 34 inform the first-block decision.
    //
    // The ampbuf-init noise only marks indices 0 and 1, which sit below the
    // cursor floor and thus never contribute to a decision.
    let cursor_idx = 1024 / ENV_SEARCHSTEP; // = 16
    let test_idx = 2176 / ENV_SEARCHSTEP; // = 34
    let scan_end = test_idx.min(marks.len());
    for i in cursor_idx..scan_end {
        if marks[i] {
            return 2;
        }
    }
    1
}

/// Reproduce libvorbis's per-block nW decision from envelope marks.
///
/// Mirrors `_ve_envelope_search`: given the current `centerW`, current `W`
/// and the cursor position, walk forward through marks to find the next
/// mark in (centerW, testW). If found → nW=0 (short next). If testW reached
/// first → nW=1 (long next). If neither (cursor runs out of marks) → nW=-1
/// (insufficient data, defer).
///
/// Returns (nW, new_cursor). nW: 0=short, 1=long, -1=insufficient.
pub fn next_w(
    marks: &[bool],
    cursor: i64,
    curmark_in: i64,
    center_w: i64,
    w: i32,
) -> (i32, i64, i64) {
    let bs0 = SHORT_BLOCK as i64;
    let bs1 = LONG_BLOCK as i64;
    let step = ENV_SEARCHSTEP as i64;
    let test_w = center_w + (if w == 1 { bs1 } else { bs0 }) / 4 + bs1 / 2 + bs0 / 4;

    let mut j = cursor;
    // libvorbis: `while(j < ve->current - searchstep)` where
    // `ve->current = last * searchstep` and `last = pcm_current/searchstep - VE_WIN`.
    // marks vec is sized to n_steps + VE_POST + 1, so n_steps = len - VE_POST - 1.
    // Cursor walks up to (n_steps - 1) * step inclusive.
    let n_steps = marks.len() as i64 - VE_POST as i64 - 1;
    let limit = n_steps * step;

    while j < limit - step {
        if j >= test_w {
            return (1, j, curmark_in);
        }
        let idx = (j / step) as usize;
        if idx < marks.len() && marks[idx] && j > center_w {
            return (0, j, j);
        }
        j += step;
    }
    (-1, j, curmark_in)
}

/// Pre-compute the W-flag pattern for the entire stream.
///
/// Returns a Vec<i32> where each entry is the W flag (0=short, 1=long) for
/// that block, advancing centerW per the libvorbis state machine.
///
/// `n_samples` includes any post-extrapolated tail (so blocks past EOS are
/// also decided).
pub fn full_w_pattern(marks: &[bool], n_samples: i64) -> Vec<i32> {
    let (pattern, _) = full_w_pattern_with_curmark(marks, n_samples);
    pattern
}

/// Same as [`full_w_pattern`] but also returns the per-block `curmark`
/// position (in env-mark space) — needed for `_ve_envelope_mark` IMPULSE
/// vs PADDING decisions.
pub fn full_w_pattern_with_curmark(marks: &[bool], n_samples: i64) -> (Vec<i32>, Vec<i64>) {
    let bs0 = SHORT_BLOCK as i64;
    let bs1 = LONG_BLOCK as i64;

    // Initial state mirrors libvorbis init:
    //   v->W = 0, v->lW = 0, v->nW = 0, v->centerW = bs1/2 = 1024
    //   ve->cursor = bs1/2 = 1024, ve->curmark = 0 (calloc'd init)
    let mut center_w: i64 = bs1 / 2;
    let mut cursor: i64 = bs1 / 2;
    let mut curmark: i64 = 0;
    let mut w: i32 = 0;

    let mut pattern: Vec<i32> = Vec::new();
    let mut curmarks: Vec<i64> = Vec::new();
    pattern.push(0); // first block is always short (W=0 in init)
    curmarks.push(curmark);

    loop {
        // Decide next block's W. If next_w returns -1 (insufficient data /
        // no upcoming mark), default to long (= what libvorbis settles on
        // once data accumulates). At true EOS the last block becomes short
        // via the same default — see how libvorbis sets nW=0 at eofflag.
        let (next, new_cursor, new_curmark) = next_w(marks, cursor, curmark, center_w, w);
        cursor = new_cursor;
        curmark = new_curmark;
        let nw = if next == -1 { 1 } else { next };

        // Advance centerW by bs[W]/4 + bs[nW]/4.
        let advance = (if w == 1 { bs1 } else { bs0 }) / 4 + (if nw == 1 { bs1 } else { bs0 }) / 4;
        let new_center_w = center_w + advance;

        // Emit the next block (with the new W flag).
        pattern.push(nw);
        curmarks.push(curmark);

        // Stop when centerW has advanced past the audio.
        if new_center_w >= n_samples {
            break;
        }

        center_w = new_center_w;
        w = nw;
    }

    (pattern, curmarks)
}

/// Faithful streaming-blockout simulation.
///
/// Mirrors libvorbis's `vorbis_analysis_blockout` chunk-by-chunk drain so we
/// can derive the exact `pcm_current` libvorbis observes at EOS (i.e. the
/// `eofflag` value captured inside `vorbis_analysis_wrote(v, 0)`). This is
/// what post-extrap LPC training keys on.
///
/// Inputs:
///   - `marks`: precomputed envelope marks over the full env_pcm buffer
///     (pre-extrap + audio + post-extrap), in absolute coordinates.
///   - `total_audio`: number of real audio samples (no pre/post-extrap).
///   - `chunk_size`: PCM feed granularity. ffmpeg+libvorbis use 64.
///
/// Returns `pcm_current` at the moment EOS is signaled — i.e. after all audio
/// has been fed and the blockout drain has run as far as it can without
/// `eofflag` being set. This is the value libvorbis stores in `v->eofflag`.
pub fn simulate_eofflag(marks: &[bool], total_audio: i64, chunk_size: i64) -> i64 {
    let bs0 = SHORT_BLOCK as i64;
    let bs1 = LONG_BLOCK as i64;
    let step = ENV_SEARCHSTEP as i64;
    let ve_win = VE_WIN as i64;

    let mut pcm_current: i64 = bs1 / 2; // libvorbis init
    let mut samples_fed: i64 = 0;
    let mut preextrapolated = false;
    let preextrap_threshold: i64 = bs1 / 2 + bs1; // 3072

    let center_w: i64 = bs1 / 2; // always reset to bs1/2 after each emit
    let mut cursor: i64 = bs1 / 2;
    let mut w: i32 = 0;

    // Total accumulated movementW so we can index `marks` in absolute coords.
    let mut total_shift: i64 = 0;

    loop {
        if samples_fed >= total_audio {
            // EOS would be next. Return pcm_current as eofflag.
            return pcm_current;
        }
        let chunk = chunk_size.min(total_audio - samples_fed);
        pcm_current += chunk;
        samples_fed += chunk;

        if !preextrapolated {
            if pcm_current > preextrap_threshold {
                preextrapolated = true;
            } else {
                continue;
            }
        }

        // Drain blocks (mirror vorbis_analysis_blockout loop)
        loop {
            // ve_envelope_search emulation:
            //   first = ve->current/step (we don't track ve->current explicitly
            //   since marks are precomputed; we only enforce the visibility
            //   limit `last = pcm_current/step - VE_WIN` on the cursor walk).
            let test_w = center_w + (if w == 1 { bs1 } else { bs0 }) / 4 + bs1 / 2 + bs0 / 4;
            let last = pcm_current / step - ve_win;
            let ve_current = last * step;

            let mut j = cursor;
            let mut search_result: i32 = -1;
            let mut new_cursor = cursor;
            while j < ve_current - step {
                if j >= test_w {
                    search_result = 1;
                    break;
                }
                new_cursor = j;
                let abs_idx = ((j + total_shift) / step) as usize;
                if abs_idx < marks.len() && marks[abs_idx] && j > center_w {
                    new_cursor = j;
                    if j >= test_w {
                        search_result = 1;
                    } else {
                        search_result = 0;
                    }
                    break;
                }
                j += step;
            }
            cursor = new_cursor;

            // eofflag is 0 in this simulation (we return before signaling EOS).
            // So bp == -1 means defer.
            if search_result == -1 {
                break;
            }
            let nw = search_result;

            let bs_w = if w == 1 { bs1 } else { bs0 };
            let bs_nw = if nw == 1 { bs1 } else { bs0 };
            let center_next = center_w + bs_w / 4 + bs_nw / 4;
            let blockbound = center_next + bs_nw / 2;
            if pcm_current < blockbound {
                break;
            }

            // Emit. Apply movementW shift (libvorbis "advance storage vectors").
            let movement = center_next - bs1 / 2;
            pcm_current -= movement;
            cursor -= movement;
            total_shift += movement;
            w = nw;
        }
    }
}
