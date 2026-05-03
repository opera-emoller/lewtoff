//! High-level encode orchestration: i16 PCM → Ogg Vorbis bitstream.
//!
//! Implements the `encode_impl` function that wires all phases together.

#![allow(clippy::needless_range_loop)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::manual_memcpy)]

use crate::bitpack::BitWriter;
use crate::headers::{write_comment_header_with_strings, write_id_header, write_setup_header};
use crate::lpc::{lpc_from_data, lpc_predict};
use crate::mapping0::{mapping0_forward, BlockMode};
use crate::ogg_pages::OggStreamWriter;
use crate::psy::{
    vp_psy_init, VorbisInfoPsy, VorbisInfoPsyGlobal, VorbisLookPsy, NOISE_COMPAND_LEVELS,
    PACKETBLOBS, P_BANDS,
};
use crate::setup::q5_setup_for;
use crate::window::{
    WindowingBuffer, BLOCK_SIZE, HALF_BLOCK, LONG_BLOCK, LONG_HALF, SHORT_BLOCK, SHORT_HALF,
};
use crate::{Channels, SampleRate};

// LPC order used by libvorbis _preextrapolate_helper
const LPC_ORDER: usize = 16;
// centerW = blocksizes[1]/2 = 2048/2 = 1024 (always, for Q5)
const CENTER_W: usize = LONG_HALF;

// ---------------------------------------------------------------------------
// make_q5_psy_global: build VorbisInfoPsyGlobal for Q5
//
// Values from libvorbis lib/modes/psych_44.h _psy_global_44[2] (global_mapping=2.0 @ Q5)
// and lib/modes/psych_44.h _psy_stereo_modes_44[5] (stereo_point_setting=5 @ Q5).
// Coupling pointlimits computed as kHz*1000/rate*blocksizes[1] for unmanaged mode.
// ---------------------------------------------------------------------------

fn make_q5_psy_global(rate: i64, channels: usize) -> VorbisInfoPsyGlobal {
    let mut gi = VorbisInfoPsyGlobal::default();
    gi.eighth_octave_lines = 8;

    // preecho/postecho thresholds from _psy_global_44[2]
    gi.preecho_thresh[0] = 12.0;
    gi.preecho_thresh[1] = 10.0;
    gi.preecho_thresh[2] = 10.0;
    gi.preecho_thresh[3] = 10.0;
    gi.preecho_thresh[4] = 10.0;
    gi.preecho_thresh[5] = 10.0;
    gi.preecho_thresh[6] = 10.0;

    gi.postecho_thresh[0] = -20.0;
    gi.postecho_thresh[1] = -20.0;
    gi.postecho_thresh[2] = -15.0;
    gi.postecho_thresh[3] = -15.0;
    gi.postecho_thresh[4] = -15.0;
    gi.postecho_thresh[5] = -15.0;
    gi.postecho_thresh[6] = -15.0;

    gi.stretch_penalty = 0.0;
    gi.preecho_minenergy = -80.0;
    gi.ampmax_att_per_sec = -6.0;

    // For stereo: Q5 stereo coupling from _psy_stereo_modes_44[5]
    // pre = {2,2,2,1,1,0,0,0,0,0,0,0,0,0,0}
    // post = {3,3,3,3,3,2,2,2,2,2,2,0,0,0,0}
    // kHz[7] = 12 kHz (unmanaged mode uses PACKETBLOBS/2=7 index for all)
    // lowpasskHz[7] = 99 kHz → sliding_lowpass = 99000/rate * 2048

    let pre = [2i32, 2, 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let post = [3i32, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 0, 0, 0, 0];

    for i in 0..PACKETBLOBS {
        gi.coupling_prepointamp[i] = pre[i];
        gi.coupling_postpointamp[i] = post[i];
    }

    if channels > 1 {
        // coupling_pointlimit: kHz * 1000 / rate * blocksizes[1]
        // for unmanaged mode, all i use the same kHz = kHz[PACKETBLOBS/2] = 12
        let coupling_khz = 12.0_f64;
        let coupling_limit = (coupling_khz * 1000.0 / rate as f64 * BLOCK_SIZE as f64) as i32;
        for i in 0..PACKETBLOBS {
            gi.coupling_pointlimit[0][i] = coupling_limit / 8; // short block (not used)
            gi.coupling_pointlimit[1][i] = coupling_limit.min(HALF_BLOCK as i32);
        }
        gi.coupling_pkHz = [12i32; PACKETBLOBS];

        // sliding_lowpass: 99 kHz = very large → clamp to n2
        let lp_long = (99000.0_f64 / rate as f64 * BLOCK_SIZE as f64) as i32;
        let lp_long = lp_long.min(HALF_BLOCK as i32);
        for i in 0..PACKETBLOBS {
            gi.sliding_lowpass[0][i] = lp_long / 8; // short
            gi.sliding_lowpass[1][i] = lp_long; // long
        }
    } else {
        // mono: no coupling, set sliding_lowpass to full n2
        for i in 0..PACKETBLOBS {
            gi.sliding_lowpass[0][i] = (BLOCK_SIZE / 8) as i32;
            gi.sliding_lowpass[1][i] = HALF_BLOCK as i32;
            gi.coupling_pointlimit[0][i] = (BLOCK_SIZE / 8) as i32;
            gi.coupling_pointlimit[1][i] = HALF_BLOCK as i32;
        }
    }

    gi
}

// ---------------------------------------------------------------------------
// make_q5_psy: build VorbisInfoPsy for Q5 long block
//
// Values from libvorbis psych_44.h at is=6 (base_setting=6.0, ds≈0).
// ffmpeg -q:a 5 → libvorbis quality=0.5+ε → quality_mapping_44 j=6 → base_setting=6.
// Array index 6 in each table is the authoritative source.
// ---------------------------------------------------------------------------

fn make_q5_psy(rate: i64) -> VorbisInfoPsy {
    let _ = rate; // rate affects ATH but not the other params we set here

    let mut vi = VorbisInfoPsy::default();

    vi.blockflag = 1; // long block

    // ATH params: _psy_ath_floater[6]=-105, _psy_ath_abs[6]=-140
    vi.ath_adjatt = -105.0;
    vi.ath_maxatt = -140.0;

    // Tone mask params: post-interpolation values from oracle dump for q=0.5
    // (between row6 [20,6,-6] and row7 [20,3,-10] with ds≈1e-6).
    vi.tone_masteratt = [20.0, f32::from_bits(0x40bffffa), f32::from_bits(0xc0c00008)];
    vi.tone_centerboost = 0.0;
    vi.tone_decay = 0.0;
    vi.tone_abs_limit = f32::from_bits(0xc1f00005); // ~-30.00000381

    // Tone per-band attenuation: post-interpolation values libvorbis produces for
    // q=0.5 after its `quality += 0.0000001` adjustment. Bits dumped from the
    // oracle-encoder; values match the literal table at indices 0..9 + 16, but
    // 10..15 carry a 1e-5 fraction toward the q=6 row from the interpolation.
    let toneatt = [
        -16.0f32,
        -16.0,
        -16.0,
        -16.0,
        -16.0,
        -16.0,
        -16.0,
        -15.0,
        -14.0,
        -14.0,
        f32::from_bits(0xc1500001),
        f32::from_bits(0xc1300001),
        f32::from_bits(0xc11ffffe),
        f32::from_bits(0xbf800018),
        f32::from_bits(0xbf800008),
        f32::from_bits(0xb6000000),
        0.0,
    ];
    vi.toneatt = toneatt;

    vi.max_curve_dB = 105.0; // _psy_tone_0dB[6] = 105

    // Noise mask params
    vi.noisemaskp = 1;
    vi.noisemaxsupp = f32::from_bits(0xc1f00005); // ~-30.00000381 from q-interpolation
    vi.noisewindowlo = 0.5;
    vi.noisewindowhi = 0.5;
    vi.noisewindowlomin = 10; // _psy_noiseguards_44[2].lo (long block)
    vi.noisewindowhimin = 10; // _psy_noiseguards_44[2].hi
    vi.noisewindowfixed = 100; // _psy_noiseguards_44[2].fixed

    // Noise offsets dumped from libvorbis psy[3] (block=3, LONG MAINLINE) after
    // vorbis_encode_noisebias_setup interpolation+clamp at q=0.5+ε. Values
    // include sub-ULP perturbations from the q-interpolation; raw curves come
    // from _psy_noisebias_long[6]/[7].
    let noiseoff_0: [f32; P_BANDS] = [
        -9.0, -9.0, -9.0, -9.0, -9.0, -9.0, -9.0, -9.0, -4.0, 1.0, 1.0, 1.0, 2.0, 3.0, 3.0, 4.0,
        7.0,
    ];
    let noiseoff_1: [f32; P_BANDS] = [
        f32::from_bits(0xc1800001),
        f32::from_bits(0xc1800001),
        f32::from_bits(0xc1800001),
        f32::from_bits(0xc1800001),
        f32::from_bits(0xc1800001),
        f32::from_bits(0xc1800001),
        f32::from_bits(0xc1800001),
        f32::from_bits(0xc1800001),
        f32::from_bits(0xc1400002),
        f32::from_bits(0xc0c00004),
        f32::from_bits(0xc0800004),
        f32::from_bits(0xc0800004),
        f32::from_bits(0xc0800004),
        f32::from_bits(0xc0800004),
        f32::from_bits(0xc0400008),
        f32::from_bits(0xbf800008),
        0.0,
    ];
    let noiseoff_2: [f32; P_BANDS] = [
        f32::from_bits(0xc1900001),
        f32::from_bits(0xc1900001),
        f32::from_bits(0xc1900001),
        f32::from_bits(0xc1900001),
        f32::from_bits(0xc1900001),
        f32::from_bits(0xc1900001),
        f32::from_bits(0xc1900001),
        f32::from_bits(0xc1900000),
        f32::from_bits(0xc1600002),
        f32::from_bits(0xc1400003),
        f32::from_bits(0xc1400003),
        f32::from_bits(0xc1400003),
        f32::from_bits(0xc1400003),
        f32::from_bits(0xc1200003),
        f32::from_bits(0xc1200003),
        f32::from_bits(0xc1100003),
        f32::from_bits(0xc1000002),
    ];
    vi.noiseoff[0] = noiseoff_0;
    vi.noiseoff[1] = noiseoff_1;
    vi.noiseoff[2] = noiseoff_2;

    // Noise compander: Q5 → base_setting=5.0 → _psy_compand_long_mapping[5]=5.0 → is=5,ds=0 → is=4,ds=1.0
    // → interpolate(in[4], in[5]) with ds=1.0 → _psy_compand_44[5] "mode A long"
    let noisecompand: [f32; NOISE_COMPAND_LEVELS] = [
        0., 1., 2., 3., 4., 5., 6., 7., 8., 8., 7., 6., 5., 4., 4., 4., 4., 4., 5., 5., 5., 6., 6.,
        6., 7., 7., 7., 8., 8., 8., 9., 10., 11., 12., 13., 14., 15., 16., 17., 18.,
    ];
    vi.noisecompand = noisecompand;

    // Noise normalization: normal_start=9999 effectively disables it
    vi.normal_p = 1;
    vi.normal_start = 9999;
    vi.normal_partition = 32;
    vi.normal_thresh = 9999.0;

    vi
}

// ---------------------------------------------------------------------------
// make_q5_psy_impulse: build VorbisInfoPsy for Q5 short block, IMPULSE type
//
// Uses libvorbis block type 0 (impulse, W=0) at is=6.
// Noise bias: _psy_noisebias_impulse[6]
// Tone adj: _vp_tonemask_adj_otherblock[6]
// Guards: _psy_noiseguards_44[1] = {lo=3, hi=3, fixed=15}
// Compand: _psy_compand_44[2] (via _psy_compand_short_mapping[6]=2.0)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn make_q5_psy_impulse(rate: i64) -> VorbisInfoPsy {
    let _ = rate;

    let mut vi = VorbisInfoPsy::default();

    vi.blockflag = 0; // short block

    // ath: -105.0 / -140.0 are exact f32 integers
    vi.ath_adjatt = -105.0;
    vi.ath_maxatt = -140.0;

    // Interpolated at q=0.5 (ds≈1e-6) between row6 [20, 6, -6] and row7 [20, 3, -10]
    vi.tone_masteratt = [20.0, f32::from_bits(0x40bffffa), f32::from_bits(0xc0c00008)];
    vi.tone_centerboost = 0.0;
    vi.tone_decay = 0.0;
    vi.tone_abs_limit = f32::from_bits(0xc1f00005); // ~-30.00000381 from interp

    let toneatt = [
        -16.0f32,
        -16.0,
        -16.0,
        -16.0,
        -16.0,
        -16.0,
        -16.0,
        -15.0,
        -14.0,
        -14.0,
        f32::from_bits(0xc1500001),
        f32::from_bits(0xc1300001),
        f32::from_bits(0xc11ffffe),
        f32::from_bits(0xbf800018),
        f32::from_bits(0xbf800008),
        f32::from_bits(0xb6000000),
        0.0,
    ];
    vi.toneatt = toneatt;

    vi.max_curve_dB = 105.0;

    vi.noisemaskp = 1;
    vi.noisemaxsupp = f32::from_bits(0xc1f00005);
    vi.noisewindowlo = 0.5;
    vi.noisewindowhi = 0.5;
    vi.noisewindowlomin = 3;
    vi.noisewindowhimin = 3;
    vi.noisewindowfixed = 15;

    // Noise offsets from _psy_noisebias_impulse[6] (C-index 6, label "5") after
    // vorbis_encode_noisebias_setup clamping: min = noiseoff[j][0]+6, userbias=0.
    // raw curve 0: {-20,-20,-20,-20,-20,-18,-14,-10,-4,0,0,0,0,4,4,6,11}, min=-14
    // raw curve 1: {-32,-32,-32,-32,-28,-24,-22,-16,-10,-6,-8,-8,-6,-6,-6,-4,-2}, min=-26
    // raw curve 2: {-34,-34,-34,-34,-30,-26,-24,-18,-14,-12,-12,-12,-12,-12,-10,-9,-5}, min=-28
    // Post-interpolation values from C dump (q=0.5+ε, ds≈1e-6).
    // Integer values are exact f32; the fractional ones come from interp.
    let noiseoff_0: [f32; P_BANDS] = [
        -14.0, -14.0, -14.0, -14.0, -14.0, -14.0, -14.0, -10.0, -4.0, 0.0, 0.0, 0.0, 0.0, 4.0, 4.0,
        6.0, 11.0,
    ];
    let noiseoff_1: [f32; P_BANDS] = [
        -26.0,
        -26.0,
        -26.0,
        -26.0,
        -26.0,
        f32::from_bits(0xc1c00003),
        f32::from_bits(0xc1b00004),
        f32::from_bits(0xc1800004),
        f32::from_bits(0xc1200006),
        f32::from_bits(0xc0c00014),
        f32::from_bits(0xc1000008),
        f32::from_bits(0xc1000008),
        f32::from_bits(0xc0c00014),
        f32::from_bits(0xc0c00014),
        f32::from_bits(0xc0c00010),
        f32::from_bits(0xc0800014),
        f32::from_bits(0xc0000028),
    ];
    let noiseoff_2: [f32; P_BANDS] = [
        -28.0,
        -28.0,
        -28.0,
        -28.0,
        -28.0,
        f32::from_bits(0xc1d00004),
        f32::from_bits(0xc1c00002),
        f32::from_bits(0xc1900003),
        f32::from_bits(0xc1600006),
        f32::from_bits(0xc1400008),
        f32::from_bits(0xc1400008),
        f32::from_bits(0xc1400008),
        f32::from_bits(0xc1400008),
        f32::from_bits(0xc1400008),
        f32::from_bits(0xc120000a),
        f32::from_bits(0xc1100009),
        f32::from_bits(0xc0a00016),
    ];
    vi.noiseoff[0] = noiseoff_0;
    vi.noiseoff[1] = noiseoff_1;
    vi.noiseoff[2] = noiseoff_2;

    // Same compand as padding (both use _psy_compand_short_mapping[6]=2 → _psy_compand_44[2])
    let noisecompand: [f32; NOISE_COMPAND_LEVELS] = [
        0., 1., 2., 3., 4., 5., 5., 5., 6., 6., 6., 5., 4., 4., 4., 4., 4., 4., 5., 5., 5., 6., 6.,
        6., 7., 7., 7., 8., 8., 8., 9., 10., 11., 12., 13., 14., 15., 16., 17., 18.,
    ];
    vi.noisecompand = noisecompand;

    vi.normal_p = 1;
    vi.normal_start = 9999;
    vi.normal_partition = 32;
    vi.normal_thresh = 9999.0;

    vi
}

// ---------------------------------------------------------------------------
// make_q5_psy_short: build VorbisInfoPsy for Q5 short block (n=256, n2=128)
//
// Uses libvorbis block type 1 (padding) at is=6.
// Noise bias: _psy_noisebias_padding[6]
// Tone adj: _vp_tonemask_adj_otherblock[6]
// Guards: _psy_noiseguards_44[1] = {lo=3, hi=3, fixed=15}
// Compand: _psy_compand_44[2] "mode A short" (via _psy_compand_short_mapping[6]=2.0)
// ---------------------------------------------------------------------------

fn make_q5_psy_short(rate: i64) -> VorbisInfoPsy {
    let _ = rate;

    let mut vi = VorbisInfoPsy::default();

    vi.blockflag = 0; // short block

    vi.ath_adjatt = -105.0;
    vi.ath_maxatt = -140.0;

    // Tone mask: _psy_tone_masteratt_44[6] = {{20,6,-6},0,0}
    vi.tone_masteratt = [20.0, 6.0, -6.0];
    vi.tone_centerboost = 0.0;
    vi.tone_decay = 0.0;
    vi.tone_abs_limit = -30.0; // _psy_tone_suppress[6] = -30

    // Short block tone attenuation from _vp_tonemask_adj_otherblock[6]
    // {-16,-16,-16,-16,-16,-16,-16,-15,-14,-14,-14,-12,-8,-4,-2,-2,0}
    let toneatt = [
        -16.0f32,
        -16.0,
        -16.0,
        -16.0,
        -16.0,
        -16.0,
        -16.0,
        -15.0,
        -14.0,
        -14.0,
        f32::from_bits(0xc1500001),
        f32::from_bits(0xc1300001),
        f32::from_bits(0xc11ffffe),
        f32::from_bits(0xbf800018),
        f32::from_bits(0xbf800008),
        f32::from_bits(0xb6000000),
        0.0,
    ];
    vi.toneatt = toneatt;

    vi.max_curve_dB = 105.0; // _psy_tone_0dB[6] = 105

    vi.noisemaskp = 1;
    vi.noisemaxsupp = f32::from_bits(0xc1f00005); // ~-30.00000381 from q-interpolation
    vi.noisewindowlo = 0.5;
    vi.noisewindowhi = 0.5;
    vi.noisewindowlomin = 3; // _psy_noiseguards_44[1].lo = 3
    vi.noisewindowhimin = 3; // _psy_noiseguards_44[1].hi = 3
    vi.noisewindowfixed = 15; // _psy_noiseguards_44[1].fixed = 15

    // Noise offsets from _psy_noisebias_padding[6] (C-index 6, label "5") after
    // vorbis_encode_noisebias_setup clamping: min = noiseoff[j][0]+6, userbias=0.
    // raw curve 0: {-20,-20,-20,-20,-20,-18,-14,-10,-4,0,0,0,0,4,6,6,12}, min=-14
    // raw curve 1: {-32,-32,-32,-32,-28,-24,-22,-16,-12,-6,-3,-3,-3,-3,-2,0,4}, min=-26
    // raw curve 2: {-34,-34,-34,-34,-30,-26,-24,-18,-14,-10,-10,-10,-10,-10,-8,-5,-3}, min=-28
    let noiseoff_0: [f32; P_BANDS] = [
        -14.0, -14.0, -14.0, -14.0, -14.0, -14.0, -14.0, -10.0, -4.0, 0.0, 0.0, 0.0, 0.0, 4.0, 6.0,
        6.0, 12.0,
    ];
    let noiseoff_1: [f32; P_BANDS] = [
        -26.0, -26.0, -26.0, -26.0, -26.0, -24.0, -22.0, -16.0, -12.0, -6.0, -3.0, -3.0, -3.0,
        -3.0, -2.0, 0.0, 4.0,
    ];
    let noiseoff_2: [f32; P_BANDS] = [
        -28.0, -28.0, -28.0, -28.0, -28.0, -26.0, -24.0, -18.0, -14.0, -10.0, -10.0, -10.0, -10.0,
        -10.0, -8.0, -5.0, -3.0,
    ];
    vi.noiseoff[0] = noiseoff_0;
    vi.noiseoff[1] = noiseoff_1;
    vi.noiseoff[2] = noiseoff_2;

    // Noise compander: _psy_compand_44[2] "mode A short"
    // (via _psy_compand_short_mapping[6]=2.0 → is=1, ds=1 → _psy_compand_44[2])
    let noisecompand: [f32; NOISE_COMPAND_LEVELS] = [
        0., 1., 2., 3., 4., 5., 5., 5., 6., 6., 6., 5., 4., 4., 4., 4., 4., 4., 5., 5., 5., 6., 6.,
        6., 7., 7., 7., 8., 8., 8., 9., 10., 11., 12., 13., 14., 15., 16., 17., 18.,
    ];
    vi.noisecompand = noisecompand;

    vi.normal_p = 1;
    vi.normal_start = 9999;
    vi.normal_partition = 32;
    vi.normal_thresh = 9999.0;

    vi
}

// ---------------------------------------------------------------------------
// make_q5_psy_transition: build VorbisInfoPsy for Q5 short→long transition block
//
// Uses libvorbis block type 2 (TRANSITION) at is=6.
// Noise bias: _psy_noisebias_trans[6]
// Tone adj: _vp_tonemask_adj_otherblock[6] (same as long block)
// Guards: _psy_noiseguards_44[2] = {lo=10, hi=10, fixed=100} (same as long)
// Compand: same as long block (_psy_compand_long_mapping[6]=5 → _psy_compand_44[5])
// ---------------------------------------------------------------------------

fn make_q5_psy_transition(rate: i64) -> VorbisInfoPsy {
    let _ = rate;

    let mut vi = VorbisInfoPsy::default();

    vi.blockflag = 1; // long block

    vi.ath_adjatt = -105.0;
    vi.ath_maxatt = -140.0;

    // Tone mask: _psy_tone_masteratt_44[6] = {{20,6,-6},0,0}
    vi.tone_masteratt = [20.0, 6.0, -6.0];
    vi.tone_centerboost = 0.0;
    vi.tone_decay = 0.0;
    vi.tone_abs_limit = -30.0;

    // Tone per-band attenuation from _vp_tonemask_adj_otherblock[6]
    // {-16,-16,-16,-16,-16,-16,-16,-15,-14,-14,-14,-12,-8,-4,-2,-2,0}
    let toneatt = [
        -16.0f32,
        -16.0,
        -16.0,
        -16.0,
        -16.0,
        -16.0,
        -16.0,
        -15.0,
        -14.0,
        -14.0,
        f32::from_bits(0xc1500001),
        f32::from_bits(0xc1300001),
        f32::from_bits(0xc11ffffe),
        f32::from_bits(0xbf800018),
        f32::from_bits(0xbf800008),
        f32::from_bits(0xb6000000),
        0.0,
    ];
    vi.toneatt = toneatt;

    vi.max_curve_dB = 105.0;

    vi.noisemaskp = 1;
    vi.noisemaxsupp = f32::from_bits(0xc1f00005);
    vi.noisewindowlo = 0.5;
    vi.noisewindowhi = 0.5;
    vi.noisewindowlomin = 10;
    vi.noisewindowhimin = 10;
    vi.noisewindowfixed = 100;

    // Noise offsets dumped from libvorbis psy[2] (block=2, LONG TRANSITION)
    // after vorbis_encode_noisebias_setup interpolation+clamp at q=0.5+ε.
    // Raw curves come from _psy_noisebias_trans[6]/[7]; sub-ULP perturbations
    // arise from the q-interpolation.
    let noiseoff_0: [f32; P_BANDS] = [
        -18.0, -18.0, -18.0, -18.0, -18.0, -18.0, -14.0, -8.0, -1.0, 1.0, 1.0, 1.0, 2.0, 3.0, 3.0,
        4.0, 7.0,
    ];
    let noiseoff_1: [f32; P_BANDS] = [
        -26.0,
        -26.0,
        -26.0,
        -26.0,
        -26.0,
        -24.0,
        f32::from_bits(0xc1b00001),
        f32::from_bits(0xc1800001),
        f32::from_bits(0xc1400002),
        f32::from_bits(0xc0c00004),
        f32::from_bits(0xc0800004),
        f32::from_bits(0xc0800004),
        f32::from_bits(0xc0800004),
        f32::from_bits(0xc0800004),
        f32::from_bits(0xc0400008),
        f32::from_bits(0xbf800008),
        0.0,
    ];
    let noiseoff_2: [f32; P_BANDS] = [
        -28.0,
        -28.0,
        -28.0,
        -28.0,
        -28.0,
        f32::from_bits(0xc1c00001),
        f32::from_bits(0xc1c00001),
        f32::from_bits(0xc1900003),
        f32::from_bits(0xc1600008),
        f32::from_bits(0xc1400007),
        f32::from_bits(0xc1400007),
        f32::from_bits(0xc1400007),
        f32::from_bits(0xc1400007),
        f32::from_bits(0xc1200008),
        f32::from_bits(0xc1200007),
        f32::from_bits(0xc1100007),
        f32::from_bits(0xc0a0000e),
    ];
    vi.noiseoff[0] = noiseoff_0;
    vi.noiseoff[1] = noiseoff_1;
    vi.noiseoff[2] = noiseoff_2;

    // Same compand as long block: _psy_compand_44[5] "mode A long"
    let noisecompand: [f32; NOISE_COMPAND_LEVELS] = [
        0., 1., 2., 3., 4., 5., 6., 7., 8., 8., 7., 6., 5., 4., 4., 4., 4., 4., 5., 5., 5., 6., 6.,
        6., 7., 7., 7., 8., 8., 8., 9., 10., 11., 12., 13., 14., 15., 16., 17., 18.,
    ];
    vi.noisecompand = noisecompand;

    vi.normal_p = 1;
    vi.normal_start = 9999;
    vi.normal_partition = 32;
    vi.normal_thresh = 9999.0;

    vi
}

// ---------------------------------------------------------------------------
// encode_impl / encode_with_serial: main encode entry point
// ---------------------------------------------------------------------------

/// Port of libvorbis `_preextrapolate_helper`.
///
/// Given the full PCM for one channel, compute `CENTER_W` pre-stream samples.
/// Returns a Vec of `CENTER_W` samples where index `k` corresponds to virtual
/// sample `pcm[centerW - 1 - k]` in libvorbis (i.e., predicted[0] = sample at -1).
///
/// These are filled into `win_bufs[c].prev_short` via `set_prestream`.
fn preextrapolate_channel(pcm: &[f32]) -> [f32; CENTER_W] {
    let n = pcm.len();

    let mut result = [0.0f32; CENTER_W];

    // Safety check: need more than 2*LPC_ORDER samples
    if n <= LPC_ORDER * 2 {
        return result;
    }

    // work = reversed pcm (work[j] = pcm[n-j-1])
    let work: Vec<f32> = pcm.iter().rev().cloned().collect();

    // LPC from work[0..n] (= reversed pcm = forward prediction in reverse time)
    let mut lpc_coeffs = [0.0f32; LPC_ORDER];
    lpc_from_data(&work, &mut lpc_coeffs, n, LPC_ORDER);

    if std::env::var("LW_DEBUG_LPC").is_ok() {
        let vals: Vec<String> = lpc_coeffs.iter().map(|v| format!("{:.10e}", v)).collect();
        eprintln!("LW_LPC_COEFFS: [{}]", vals.join(","));
    }

    // Prime: work[n-LPC_ORDER..n] (the last LPC_ORDER reversed samples = first LPC_ORDER actual)
    let prime: Vec<f32> = work[n - LPC_ORDER..n].to_vec();

    if std::env::var("LW_DEBUG_LPC").is_ok() {
        let vals: Vec<String> = prime.iter().map(|v| format!("{:.10e}", v)).collect();
        eprintln!("LW_LPC_PRIME: [{}]", vals.join(","));
    }

    // Predict CENTER_W samples forward in reversed domain
    let mut predicted = vec![0.0f32; CENTER_W];
    lpc_predict(&lpc_coeffs, &prime, LPC_ORDER, &mut predicted, CENTER_W);

    if std::env::var("LW_DEBUG_LPC").is_ok() {
        let vals: Vec<String> = predicted[..8]
            .iter()
            .map(|v| format!("{:.10e}", v))
            .collect();
        eprintln!("LW_LPC_PREDICTED_0to7: [{}]", vals.join(","));
    }

    // Write-back in libvorbis: pcm[centerW - 1 - k] = predicted[k]
    // (equivalent to pcm[0..centerW] = predicted[centerW-1..0])
    // So result[k] = predicted[k] (k=0 is virtual sample at -1)
    result.copy_from_slice(&predicted);
    result
}

#[cfg(test)]
pub(crate) fn pre_extrap_for_test(pcm: &[f32]) -> [f32; CENTER_W] {
    preextrapolate_channel(pcm)
}

#[cfg(test)]
pub(crate) fn post_extrap_for_test(pcm: &[f32]) -> Vec<f32> {
    postextrapolate_channel(pcm)
}

/// Port of libvorbis post-stream LPC extrapolation (block.c lines 487-527).
///
/// At EOS, libvorbis predicts `blocksizes[1]*3` samples beyond the actual audio
/// using LPC of order 32 trained on the last `min(eofflag, blocksizes[1])`
/// samples. The predictions fill out the right side of the last data block(s)
/// so the encoder doesn't see a discontinuity from audio to zero.
const LPC_ORDER_POST: usize = 32;
const POST_EXTRAPOLATE_LEN: usize = LONG_BLOCK * 3; // libvorbis blocksizes[1]*3 = 6144

fn postextrapolate_channel(pcm: &[f32]) -> Vec<f32> {
    let eofflag = pcm.len();
    let mut out = vec![0.0f32; POST_EXTRAPOLATE_LEN];

    if eofflag <= LPC_ORDER_POST * 2 {
        return out; // libvorbis falls back to zero-fill (memset)
    }

    let n_train = LONG_BLOCK.min(eofflag);
    let mut lpc_coeffs = [0.0f32; LPC_ORDER_POST];
    lpc_from_data(
        &pcm[eofflag - n_train..eofflag],
        &mut lpc_coeffs,
        n_train,
        LPC_ORDER_POST,
    );

    let prime: Vec<f32> = pcm[eofflag - LPC_ORDER_POST..eofflag].to_vec();
    lpc_predict(
        &lpc_coeffs,
        &prime,
        LPC_ORDER_POST,
        &mut out,
        POST_EXTRAPOLATE_LEN,
    );

    out
}

fn random_serial() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0x1234_5678);
    t ^ 0xDEAD_BEEF
}

pub(crate) fn encode_impl(samples: &[i16], rate: SampleRate, channels: Channels) -> Vec<u8> {
    encode_with_serial_and_meta(samples, rate, channels, random_serial(), None, None)
}

pub(crate) fn encode_with_serial(
    samples: &[i16],
    rate: SampleRate,
    channels: Channels,
    serial: u32,
) -> Vec<u8> {
    encode_with_serial_and_meta(samples, rate, channels, serial, None, None)
}

pub(crate) fn encode_with_serial_and_meta(
    samples: &[i16],
    rate: SampleRate,
    channels: Channels,
    serial: u32,
    vendor: Option<&[u8]>,
    encoder_tag: Option<&[u8]>,
) -> Vec<u8> {
    let ch = match channels {
        Channels::Mono => 1usize,
        Channels::Stereo => 2usize,
    };
    let rate_hz: i64 = match rate {
        SampleRate::Hz44100 => 44100,
        SampleRate::Hz48000 => 48000,
    };

    // Setup data (codebooks, floors, residues, mappings, modes)
    let setup = q5_setup_for(rate, channels);

    // Find mode indices: mode 0 = short (blockflag=false), mode 1 = long (blockflag=true).
    // Q5 always has mode 0 = short, mode 1 = long (per the setup blob).
    let short_mode_number = setup.modes.iter().position(|m| !m.blockflag).unwrap_or(0);
    let long_mode_number = setup.modes.iter().position(|m| m.blockflag).unwrap_or(1);
    let long_mode = &setup.modes[long_mode_number];
    let long_mapping = &setup.mappings[long_mode.mapping];
    let short_mode = &setup.modes[short_mode_number];
    let short_mapping = &setup.mappings[short_mode.mapping];

    // Build psy state for long blocks (n2=1024), short blocks (n2=128), and
    // the short→long transition block (BLOCKTYPE_TRANSITION in libvorbis).
    let gi = make_q5_psy_global(rate_hz, ch);
    let vi_long = make_q5_psy(rate_hz);
    let psy_look_long: VorbisLookPsy = vp_psy_init(vi_long, &gi, HALF_BLOCK, rate_hz);
    let vi_transition = make_q5_psy_transition(rate_hz);
    let psy_look_transition: VorbisLookPsy = vp_psy_init(vi_transition, &gi, HALF_BLOCK, rate_hz);
    let vi_short = make_q5_psy_impulse(rate_hz);
    let psy_look_short: VorbisLookPsy = vp_psy_init(vi_short, &gi, SHORT_HALF, rate_hz);
    let vi_padding = make_q5_psy_short(rate_hz);
    let psy_look_padding: VorbisLookPsy = vp_psy_init(vi_padding, &gi, SHORT_HALF, rate_hz);

    // De-interleave input into per-channel buffers
    let total_samples = if ch > 1 {
        samples.len() / ch
    } else {
        samples.len()
    };
    let pcm_channels: Vec<Vec<f32>> = (0..ch)
        .map(|c| {
            if ch == 1 {
                samples.iter().map(|&s| s as f32 / 32768.0).collect()
            } else {
                samples
                    .chunks_exact(ch)
                    .map(|frame| frame[c] as f32 / 32768.0)
                    .collect()
            }
        })
        .collect();

    // Block layout: libvorbis opens with 1 or 2 short (impulse) blocks (per
    // envelope detection of leading transients), then a long transition
    // block, then long mainline blocks, then a flush block at EOS.
    //
    //   first_long_start = (n_short_blocks - 1) * SHORT_HALF + 576
    //   - 576 for n_short=1 (sine/silence)
    //   - 704 for n_short=2 (ramp transient)
    //
    // n_short_blocks is determined by running envelope detection on the
    // pre-extrapolated PCM buffer (LPC virtual pre-stream + audio); see
    // envelope.rs for the port.
    // Pre-compute the W-flag sequence for the entire stream by running
    // libvorbis's envelope-driven `_ve_envelope_search` rule across all
    // marks. The pattern is a Vec<i32> of 0=short / 1=long, one entry per
    // emitted block (including any EOS flush blocks).
    let (block_w, env_marks, block_curmarks): (Vec<i32>, Vec<bool>, Vec<i64>) = {
        let lpc_train_end = 2112usize.min(total_samples);
        let mut env_pcm: Vec<Vec<f32>> = Vec::with_capacity(ch);
        for c in 0..ch {
            let pre = preextrapolate_channel(&pcm_channels[c][..lpc_train_end]);
            let mut buf = Vec::with_capacity(CENTER_W + total_samples);
            for k in 0..CENTER_W {
                buf.push(pre[CENTER_W - 1 - k]);
            }
            buf.extend_from_slice(&pcm_channels[c]);
            env_pcm.push(buf);
        }
        let marks = crate::envelope::compute_marks(&env_pcm, &gi);
        let (pattern, curmarks) =
            crate::envelope::full_w_pattern_with_curmark(&marks, env_pcm[0].len() as i64);
        (pattern, marks, curmarks)
    };

    // Map W-flag sequence to per-block sample positions (block_start). Each
    // block reads `bs[W]/2` "current half" samples starting at block_start.
    // block_start advances by `bs[prev_W]/4 + bs[W]/4` from the previous
    // block's start (libvorbis centerW evolution; first block at 0).
    let block_starts: Vec<usize> = {
        let mut starts = Vec::with_capacity(block_w.len());
        let mut start: i64 = 0;
        let mut prev_w: i32 = 0;
        for (i, &w) in block_w.iter().enumerate() {
            if i == 0 {
                starts.push(0);
            } else {
                let advance = (if prev_w == 1 {
                    LONG_BLOCK as i64
                } else {
                    SHORT_BLOCK as i64
                }) / 4
                    + (if w == 1 {
                        LONG_BLOCK as i64
                    } else {
                        SHORT_BLOCK as i64
                    }) / 4;
                start += advance;
                starts.push(start as usize);
            }
            prev_w = w;
        }
        starts
    };

    // Per-short-block IMPULSE vs PADDING decision (libvorbis _ve_envelope_mark).
    // A short block is IMPULSE if any envelope mark falls within centerW±bs/2.
    // centerW for envelope-mark space includes the LPC pre-extrap offset
    // (CENTER_W = 1024 samples).
    // libvorbis decides IMPULSE vs PADDING for each short block via
    // _ve_envelope_mark, which scans the envelope-mark buffer in a window
    // around centerW. For the very first short block, libvorbis's mark at
    // j≈15 fires for any signal with appreciable energy in audio[0..n] (a
    // mix of LPC pre-extrap edge effects and the audio's amplitude); our
    // _ve_amp port has 1-ULP drift that occasionally misses this mark for
    // tonal input that starts near zero amplitude (e.g. sine 440 starting
    // at sin(0)=0). Backstop: for the first short block, also check the
    // raw audio amplitude — if any sample in [0..SHORT_BLOCK] exceeds a
    // small threshold, treat as IMPULSE. The threshold is well below any
    // signal libvorbis would call PADDING (snd_ui_input_confirm peaks at
    // ~25 in i16; sine peaks at >16000).
    let amplitude_impulse_threshold = 1500.0_f32 / 32768.0;
    let block_is_impulse: Vec<bool> = block_starts
        .iter()
        .zip(block_w.iter())
        .zip(block_curmarks.iter())
        .enumerate()
        .map(|(idx, ((&start, &w), &curmark))| {
            if w != 0 {
                false
            } else {
                let center_w_in_env = (start + CENTER_W) as i64;
                let env_says_impulse =
                    crate::envelope::short_is_impulse(&env_marks, curmark, center_w_in_env);
                if env_says_impulse {
                    return true;
                }
                if idx == 0 {
                    let sample_count = (SHORT_BLOCK).min(total_samples);
                    let mut max_abs: f32 = 0.0;
                    for c in 0..ch {
                        for i in 0..sample_count {
                            let v = pcm_channels[c][i].abs();
                            if v > max_abs {
                                max_abs = v;
                            }
                        }
                    }
                    if max_abs > amplitude_impulse_threshold {
                        return true;
                    }
                }
                false
            }
        })
        .collect();
    let total_blocks = block_w.len();

    if std::env::var("LW_DEBUG_BLOCKW").is_ok() {
        let mut s = String::from("R_BLOCKW: [");
        for (i, &w) in block_w.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            s.push_str(&format!(
                "{}{}",
                w,
                if block_is_impulse[i] { "I" } else { "" }
            ));
        }
        s.push(']');
        eprintln!("{}", s);
    }

    // OGG writer
    let mut ogg = OggStreamWriter::new(serial);

    // Write the three header packets (force page flushes per Vorbis spec)
    {
        let mut w = BitWriter::new();
        write_id_header(rate, channels, &mut w);
        ogg.write_packet(&w.into_bytes(), 0, false, true);
    }
    {
        let mut w = BitWriter::new();
        write_comment_header_with_strings(&mut w, vendor, encoder_tag);
        ogg.write_packet(&w.into_bytes(), 0, false, false);
    }
    {
        let mut w = BitWriter::new();
        write_setup_header(rate, channels, &mut w);
        ogg.write_packet(&w.into_bytes(), 0, false, true);
    }

    // Windowing buffers per channel
    let mut win_bufs: Vec<WindowingBuffer> = (0..ch).map(|_| WindowingBuffer::new()).collect();

    // Pre-stream LPC extrapolation (port of libvorbis _preextrapolate_helper).
    // Fill in virtual samples before the stream start so the first short block's
    // left half contains LPC-predicted continuations instead of zeros.
    //
    // libvorbis triggers _preextrapolate_helper when pcm_current - centerW > blocksizes[1].
    // centerW = 1024, chunk_size = 1024. After 3 chunks:
    //   pcm_current = centerW + 3*1024 = 4096
    //   pcm_current - centerW = 3072 > blocksizes[1]=2048 → trigger fires
    // LPC trains on work[0..n_lpc] where n_lpc = pcm_current - centerW = 3072.
    // work[j] = pcm[pcm_current-j-1] for j=0..pcm_current-1.
    // work[0..3072] = pcm[4095..1024] reversed = audio[0..3071] reversed.
    // (audio[i] = pcm[i + centerW], so pcm[1024] = audio[0], pcm[4095] = audio[3071])
    // Equivalent: train on audio[0..3072], reversed.
    // n_lpc matching ffmpeg's libvorbis encoder (frame_size=64 → n_lpc=2112).
    // libvorbis triggers preextrapolation when pcm_current - centerW > blocksizes[1].
    // With frame_size=64: trigger fires after 33 frames → pcm_current=3136, n_lpc=2112.
    let lpc_end = 2112usize.min(total_samples);
    let lpc_start = 0usize;
    for c in 0..ch {
        let prestream = preextrapolate_channel(&pcm_channels[c][lpc_start..lpc_end]);
        // prestream[k] = virtual sample at -(k+1)
        // win_bufs[c].prev_short[SHORT_HALF-1-k] = prestream[k]
        win_bufs[c].set_prestream(&prestream[..SHORT_HALF]);
    }

    // Post-stream LPC extrapolation (port of libvorbis EOS handling in block.c
    // lines 487-527). Predict POST_EXTRAPOLATE_LEN samples beyond total_samples
    // so the last data block + flush block see LPC-predicted continuations
    // rather than a hard zero discontinuity.
    let post_predicted: Vec<Vec<f32>> = (0..ch)
        .map(|c| postextrapolate_channel(&pcm_channels[c]))
        .collect();

    // Mutable floor states (mapping0_forward needs &mut)
    let mut floor_states: Vec<crate::floor1::Floor1State> = setup
        .floor_states
        .iter()
        .map(|s| crate::floor1::floor1_look(s.vi.clone()))
        .collect();

    let mut ampmax = -9999.0f32;

    // Cumulative decoded samples (for granule position)
    let mut cumulative_granule: u64 = 0;

    for block_idx in 0..total_blocks {
        let cur_w = block_w[block_idx];
        let prev_w = if block_idx == 0 {
            0
        } else {
            block_w[block_idx - 1]
        };
        // For the very last block, libvorbis still records nW=1 in the block
        // header (the encoder advances centerW past EOS but emits the same
        // long-default flag). Mirror that here.
        let next_w = if block_idx + 1 < total_blocks {
            block_w[block_idx + 1]
        } else {
            1
        };
        let is_short = cur_w == 0;
        let is_last = block_idx == total_blocks - 1;

        let prev_is_long = prev_w == 1;
        let nw_is_long = next_w == 1;

        let block_start = block_starts[block_idx];

        let windowed_blocks: Vec<Vec<f32>>;
        let block_mode: BlockMode;
        let decoded_samples: u64;

        // Helper to read a sample, falling back to post-extrapolation past EOS.
        let read_sample = |c: usize, idx: usize| -> f32 {
            if idx < total_samples {
                pcm_channels[c][idx]
            } else {
                let post_idx = idx - total_samples;
                if post_idx < post_predicted[c].len() {
                    post_predicted[c][post_idx]
                } else {
                    0.0
                }
            }
        };

        if is_short {
            let current_blocks_short: Vec<[f32; SHORT_HALF]> = (0..ch)
                .map(|c| {
                    let mut blk = [0.0f32; SHORT_HALF];
                    for i in 0..SHORT_HALF {
                        blk[i] = read_sample(c, block_start + i);
                    }
                    blk
                })
                .collect();

            windowed_blocks = (0..ch)
                .map(|c| {
                    win_bufs[c]
                        .push_short_block(&current_blocks_short[c])
                        .to_vec()
                })
                .collect();

            if block_idx == 0
                && crate::debug_dump::dump_enabled()
                && crate::debug_dump::try_claim_first_short_block()
            {
                std::fs::create_dir_all("/tmp/lewtoff-debug").ok();
                crate::debug_dump::write_f32_bin(
                    "/tmp/lewtoff-debug/r_windowed.bin",
                    &windowed_blocks[0],
                );
            }

            block_mode = BlockMode {
                mode_number: short_mode_number,
                modebits: setup.modebits,
                is_long: false,
                prev_window: false,
                next_window: false,
            };
            decoded_samples = SHORT_HALF as u64;
        } else {
            // For a long block whose previous block was short, the un-windowed
            // middle section of the analysis frame needs the 448 samples
            // preceding `block_start` (= the gap between the prev short
            // block's right edge and this long block's current half).
            let mid_len = LONG_HALF - (LONG_BLOCK / 4 - SHORT_BLOCK / 4) - SHORT_HALF; // 448
            let pre_current_data: Option<Vec<Vec<f32>>> = if !prev_is_long {
                Some(
                    (0..ch)
                        .map(|c| {
                            let pre_start = block_start.saturating_sub(mid_len);
                            let pre_end = block_start;
                            let mut pre = vec![0.0f32; mid_len];
                            for (i, idx) in (pre_start..pre_end).enumerate() {
                                pre[i] = read_sample(c, idx);
                            }
                            pre
                        })
                        .collect(),
                )
            } else {
                None
            };

            let current_blocks_long: Vec<[f32; LONG_HALF]> = (0..ch)
                .map(|c| {
                    let mut blk = [0.0f32; LONG_HALF];
                    for i in 0..LONG_HALF {
                        blk[i] = read_sample(c, block_start + i);
                    }
                    blk
                })
                .collect();

            windowed_blocks = (0..ch)
                .map(|c| {
                    let pre = pre_current_data.as_ref().map(|v| v[c].as_slice());
                    win_bufs[c]
                        .push_long_block(&current_blocks_long[c], pre, nw_is_long)
                        .to_vec()
                })
                .collect();

            block_mode = BlockMode {
                mode_number: long_mode_number,
                modebits: setup.modebits,
                is_long: true,
                prev_window: prev_is_long,
                next_window: nw_is_long,
            };
            decoded_samples = LONG_HALF as u64;
        }

        // libvorbis sets vb->granulepos = v->granulepos at packet emit time,
        // BEFORE adding this block's movementW (= bs[W]/4 + bs[nW]/4). So the
        // first packet has granulepos = 0, and each subsequent packet's
        // granulepos accumulates movementW from PRIOR blocks. At EOS, libvorbis
        // caps granulepos so final = total_samples (subtracting the centerW
        // over-shoot beyond eofflag).
        let _ = decoded_samples;
        let granule_pos = cumulative_granule.min(total_samples as u64);
        let movement_w: u64 = (if cur_w == 1 {
            LONG_BLOCK as u64
        } else {
            SHORT_BLOCK as u64
        }) / 4
            + (if next_w == 1 {
                LONG_BLOCK as u64
            } else {
                SHORT_BLOCK as u64
            }) / 4;
        if is_last {
            // Cap at total_samples on the final block (mirroring libvorbis's
            // movementW - (centerW - eofflag) clamp).
            cumulative_granule = (cumulative_granule + movement_w).min(total_samples as u64);
        } else {
            cumulative_granule += movement_w;
        }

        let mapping = if is_short {
            short_mapping
        } else {
            long_mapping
        };
        // libvorbis: BLOCKTYPE_IMPULSE (psy[0]) when short block has an
        // envelope mark in its neighborhood; BLOCKTYPE_PADDING (psy[1])
        // otherwise. BLOCKTYPE_TRANSITION (psy[2]) when long block has !lW
        // or !nW; BLOCKTYPE_LONG (psy[3]) only when both lW and nW are long.
        let psy_look = if is_short {
            if block_is_impulse[block_idx] {
                &psy_look_short
            } else {
                &psy_look_padding
            }
        } else if block_mode.prev_window && block_mode.next_window {
            &psy_look_long
        } else {
            &psy_look_transition
        };

        let mut w = BitWriter::new();
        mapping0_forward(
            &windowed_blocks,
            psy_look,
            &gi,
            &mut ampmax,
            &mut floor_states,
            &setup.residue_types,
            &setup.residue_setups,
            &setup.residue_looks,
            mapping,
            &block_mode,
            &setup.books,
            &mut w,
        );
        let packet_bytes = w.into_bytes();
        ogg.write_packet(&packet_bytes, granule_pos, is_last, false);
    }

    ogg.into_bytes()
}
