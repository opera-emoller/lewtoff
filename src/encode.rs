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

    // Tone mask params from _psy_tone_masteratt_44[6] = {{20,6,-6},0,0}
    vi.tone_masteratt = [20.0, 6.0, -6.0];
    vi.tone_centerboost = 0.0;
    vi.tone_decay = 0.0;
    vi.tone_abs_limit = -30.0; // _psy_tone_suppress[6] = -30

    // Tone per-band attenuation from _vp_tonemask_adj_longblock[6]
    // {-16,-16,-16,-16,-16,-16,-16,-15,-14,-14,-14,-12,-8,-4,-2,-2,0}
    let toneatt = [
        -16.0f32, -16.0, -16.0, -16.0, -16.0, -16.0, -16.0, -15.0, -14.0, -14.0, -14.0, -12.0,
        -8.0, -4.0, -2.0, -2.0, 0.0,
    ];
    vi.toneatt = toneatt;

    vi.max_curve_dB = 105.0; // _psy_tone_0dB[6] = 105

    // Noise mask params
    vi.noisemaskp = 1;
    vi.noisemaxsupp = -30.0; // _psy_noise_suppress[6] = -30
    vi.noisewindowlo = 0.5;
    vi.noisewindowhi = 0.5;
    vi.noisewindowlomin = 10; // _psy_noiseguards_44[2].lo (long block)
    vi.noisewindowhimin = 10; // _psy_noiseguards_44[2].hi
    vi.noisewindowfixed = 100; // _psy_noiseguards_44[2].fixed

    // Noise offsets from _psy_noisebias_long[6] (C-index 6, label "5") after
    // vorbis_encode_noisebias_setup clamping: min = noiseoff[j][0]+6, userbias=0.
    // raw curve 0: {-15,-15,-15,-15,-15,-15,-15,-10,-4,1,1,1,2,3,3,4,7}, min=-9
    // raw curve 1: {-22,-22,-22,-22,-22,-22,-22,-16,-12,-6,-4,-4,-4,-4,-3,-1,0}, min=-16
    // raw curve 2: {-24,-24,-24,-24,-24,-24,-24,-18,-14,-12,-12,-12,-12,-10,-10,-9,-8}, min=-18
    let noiseoff_0: [f32; P_BANDS] = [
        -9.0, -9.0, -9.0, -9.0, -9.0, -9.0, -9.0, -9.0, -4.0, 1.0, 1.0, 1.0, 2.0, 3.0, 3.0, 4.0,
        7.0,
    ];
    let noiseoff_1: [f32; P_BANDS] = [
        -16.0, -16.0, -16.0, -16.0, -16.0, -16.0, -16.0, -16.0, -12.0, -6.0, -4.0, -4.0, -4.0,
        -4.0, -3.0, -1.0, 0.0,
    ];
    let noiseoff_2: [f32; P_BANDS] = [
        -18.0, -18.0, -18.0, -18.0, -18.0, -18.0, -18.0, -18.0, -14.0, -12.0, -12.0, -12.0, -12.0,
        -10.0, -10.0, -9.0, -8.0,
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

    vi.ath_adjatt = -105.0;
    vi.ath_maxatt = -140.0;

    vi.tone_masteratt = [20.0, 6.0, -6.0];
    vi.tone_centerboost = 0.0;
    vi.tone_decay = 0.0;
    vi.tone_abs_limit = -30.0;

    let toneatt = [
        -16.0f32, -16.0, -16.0, -16.0, -16.0, -16.0, -16.0, -15.0, -14.0, -14.0, -14.0, -12.0,
        -8.0, -4.0, -2.0, -2.0, 0.0,
    ];
    vi.toneatt = toneatt;

    vi.max_curve_dB = 105.0;

    vi.noisemaskp = 1;
    vi.noisemaxsupp = -30.0;
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
    let noiseoff_0: [f32; P_BANDS] = [
        -14.0, -14.0, -14.0, -14.0, -14.0, -14.0, -14.0, -10.0, -4.0, 0.0, 0.0, 0.0, 0.0, 4.0, 4.0,
        6.0, 11.0,
    ];
    let noiseoff_1: [f32; P_BANDS] = [
        -26.0, -26.0, -26.0, -26.0, -26.0, -24.0, -22.0, -16.0, -10.0, -6.0, -8.0, -8.0, -6.0,
        -6.0, -6.0, -4.0, -2.0,
    ];
    let noiseoff_2: [f32; P_BANDS] = [
        -28.0, -28.0, -28.0, -28.0, -28.0, -26.0, -24.0, -18.0, -14.0, -12.0, -12.0, -12.0, -12.0,
        -12.0, -10.0, -9.0, -5.0,
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

#[allow(dead_code)]
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
        -16.0f32, -16.0, -16.0, -16.0, -16.0, -16.0, -16.0, -15.0, -14.0, -14.0, -14.0, -12.0,
        -8.0, -4.0, -2.0, -2.0, 0.0,
    ];
    vi.toneatt = toneatt;

    vi.max_curve_dB = 105.0; // _psy_tone_0dB[6] = 105

    vi.noisemaskp = 1;
    vi.noisemaxsupp = -30.0; // _psy_noise_suppress[6] = -30
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
        -16.0f32, -16.0, -16.0, -16.0, -16.0, -16.0, -16.0, -15.0, -14.0, -14.0, -14.0, -12.0,
        -8.0, -4.0, -2.0, -2.0, 0.0,
    ];
    vi.toneatt = toneatt;

    vi.max_curve_dB = 105.0;

    vi.noisemaskp = 1;
    vi.noisemaxsupp = -30.0;
    vi.noisewindowlo = 0.5;
    vi.noisewindowhi = 0.5;
    vi.noisewindowlomin = 10;
    vi.noisewindowhimin = 10;
    vi.noisewindowfixed = 100;

    // Noise offsets from _psy_noisebias_trans[6] (C-index 6, label "5") after
    // vorbis_encode_noisebias_setup clamping: min = noiseoff[j][0]+6, userbias=0.
    // raw curve 0: {-24,-24,-24,-24,-20,-18,-14,-8,-1,1,1,1,2,3,3,4,7}, min=-18
    // raw curve 1: {-32,-32,-32,-32,-28,-24,-22,-16,-12,-6,-4,-4,-4,-4,-3,-1,0}, min=-26
    // raw curve 2: {-34,-34,-34,-34,-30,-24,-24,-18,-14,-12,-12,-12,-12,-10,-10,-9,-5}, min=-28
    let noiseoff_0: [f32; P_BANDS] = [
        -18.0, -18.0, -18.0, -18.0, -18.0, -18.0, -14.0, -8.0, -1.0, 1.0, 1.0, 1.0, 2.0, 3.0, 3.0,
        4.0, 7.0,
    ];
    let noiseoff_1: [f32; P_BANDS] = [
        -26.0, -26.0, -26.0, -26.0, -26.0, -24.0, -22.0, -16.0, -12.0, -6.0, -4.0, -4.0, -4.0,
        -4.0, -3.0, -1.0, 0.0,
    ];
    let noiseoff_2: [f32; P_BANDS] = [
        -28.0, -28.0, -28.0, -28.0, -28.0, -24.0, -24.0, -18.0, -14.0, -12.0, -12.0, -12.0, -12.0,
        -10.0, -10.0, -9.0, -5.0,
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

    // Block layout: libvorbis always opens with one short (padding) block,
    // then long blocks, then a flush long block at EOS.
    //   Block 0:      SHORT block (PADDING)
    //   Blocks 1..N:  LONG blocks + 1 flush block
    //   FIRST_LONG_START = SHORT_BLOCK/4 + LONG_BLOCK/4 = 64 + 512 = 576
    //   total = 1 + ceil((total_samples - 576) / 1024) + 1
    //
    // This holds for silence, tonal, and ramp inputs — libvorbis uses the same
    // padding path at stream start regardless of signal content.
    let n_short_blocks: usize = 1;
    let first_long_start: usize = SHORT_BLOCK / 4 + LONG_BLOCK / 4; // 576
    let remaining_after_transition = total_samples.saturating_sub(first_long_start);
    let long_data_blocks = remaining_after_transition.div_ceil(LONG_HALF);
    let total_blocks = n_short_blocks + long_data_blocks + 1; // +1 flush block

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
        let is_short = block_idx < n_short_blocks;
        let is_last = block_idx == total_blocks - 1;

        // nW = next block is long: true unless next block is also short
        let nw_is_long = block_idx >= n_short_blocks - 1;

        let windowed_blocks: Vec<Vec<f32>>;
        let block_mode: BlockMode;
        let decoded_samples: u64;

        if is_short {
            // Short block: 128 new samples per block
            let block_start = block_idx * SHORT_HALF;
            let current_blocks_short: Vec<[f32; SHORT_HALF]> = (0..ch)
                .map(|c| {
                    let mut blk = [0.0f32; SHORT_HALF];
                    for i in 0..SHORT_HALF {
                        let idx = block_start + i;
                        if idx < total_samples {
                            blk[i] = pcm_channels[c][idx];
                        }
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
            // Short block decodes 128 samples
            decoded_samples = SHORT_HALF as u64;
        } else {
            // Long block. block_start = first_long_start + long_block_index * LONG_HALF.
            let long_block_index = block_idx - n_short_blocks;
            let block_start = first_long_start + long_block_index * LONG_HALF;

            let prev_is_long = long_block_index > 0; // first long block's prev is short

            // For the short→long transition (long_block_index==0), the un-windowed middle
            // section [576..1024] of the analysis frame needs the 448 PCM samples that
            // precede `block_start` in the stream.
            let mid_len = LONG_HALF - (LONG_BLOCK / 4 - SHORT_BLOCK / 4) - SHORT_HALF; // 448
            let pre_current_data: Option<Vec<Vec<f32>>> = if !prev_is_long {
                Some(
                    (0..ch)
                        .map(|c| {
                            let pre_start = block_start.saturating_sub(mid_len);
                            let pre_end = block_start;
                            let mut pre = vec![0.0f32; mid_len];
                            for (i, idx) in (pre_start..pre_end).enumerate() {
                                if idx < total_samples {
                                    pre[i] = pcm_channels[c][idx];
                                }
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
                        let idx = block_start + i;
                        if idx < total_samples {
                            blk[i] = pcm_channels[c][idx];
                        }
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

        // Update granule position
        cumulative_granule += decoded_samples;
        let granule_pos = cumulative_granule.min(total_samples as u64);

        let mapping = if is_short {
            short_mapping
        } else {
            long_mapping
        };
        let psy_look = if is_short {
            &psy_look_short
        } else if block_mode.prev_window {
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
