//! High-level encode orchestration: i16 PCM → Ogg Vorbis bitstream.
//!
//! Implements the `encode_impl` function that wires all phases together.

#![allow(clippy::needless_range_loop)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::manual_memcpy)]

use crate::bitpack::BitWriter;
use crate::headers::{write_comment_header, write_id_header, write_setup_header};
use crate::mapping0::mapping0_forward;
use crate::ogg_pages::OggStreamWriter;
use crate::psy::{
    vp_psy_init, VorbisInfoPsy, VorbisInfoPsyGlobal, VorbisLookPsy, NOISE_COMPAND_LEVELS,
    PACKETBLOBS, P_BANDS,
};
use crate::setup::q5_setup_for;
use crate::window::{WindowingBuffer, BLOCK_SIZE, HALF_BLOCK};
use crate::{Channels, SampleRate};

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
// Values from libvorbis psych_44.h Q5 (index 5) for long blocks.
// ---------------------------------------------------------------------------

fn make_q5_psy(rate: i64) -> VorbisInfoPsy {
    let _ = rate; // rate affects ATH but not the other params we set here

    let mut vi = VorbisInfoPsy::default();

    vi.blockflag = 1; // long block

    // ATH params from _psy_ath_floater[5]=-105, _psy_ath_abs[5]=-140
    vi.ath_adjatt = -105.0;
    vi.ath_maxatt = -140.0;

    // Tone mask params from _psy_tone_masteratt_44[5]
    vi.tone_masteratt = [20.0, 6.0, -6.0];
    vi.tone_centerboost = 0.0;
    vi.tone_decay = 0.0;
    vi.tone_abs_limit = -24.0; // _psy_tone_suppress[5]

    // Tone per-band attenuation from _vp_tonemask_adj_longblock[5]
    // 17 values for P_BANDS=17
    let toneatt = [
        -16.0f32, -16.0, -16.0, -16.0, -16.0, -16.0, -16.0, -15.0, -14.0, -14.0, -13.0, -11.0,
        -7.0, -3.0, -1.0, -1.0, 0.0,
    ];
    vi.toneatt = toneatt;

    vi.max_curve_dB = 95.0; // _psy_tone_0dB[5]

    // Noise mask params
    vi.noisemaskp = 1;
    vi.noisemaxsupp = -24.0; // _psy_noise_suppress[5]
    vi.noisewindowlo = 0.5;
    vi.noisewindowhi = 0.5;
    vi.noisewindowlomin = 10; // _psy_noiseguards_44[2].lo (long block pair)
    vi.noisewindowhimin = 10; // _psy_noiseguards_44[2].hi
    vi.noisewindowfixed = 100; // _psy_noiseguards_44[2].fixed

    // Noise offsets from _psy_noisebias_long[5] (three curves × P_BANDS=17)
    // curve 0 (low): {-20,-20,-20,-20,-20,-18,-14,-10,-4,0,0,0,0,4,4,6,11}
    // curve 1 (mid): {-32,-32,-32,-32,-28,-24,-22,-16,-10,-6,-8,-8,-6,-6,-6,-4,-2}
    // curve 2 (hi):  {-34,-34,-34,-34,-30,-26,-24,-18,-14,-12,-12,-12,-12,-12,-10,-9,-5}
    let noiseoff_0: [f32; P_BANDS] = [
        -20.0, -20.0, -20.0, -20.0, -20.0, -18.0, -14.0, -10.0, -4.0, 0.0, 0.0, 0.0, 0.0, 4.0, 4.0,
        6.0, 11.0,
    ];
    let noiseoff_1: [f32; P_BANDS] = [
        -32.0, -32.0, -32.0, -32.0, -28.0, -24.0, -22.0, -16.0, -10.0, -6.0, -8.0, -8.0, -6.0,
        -6.0, -6.0, -4.0, -2.0,
    ];
    let noiseoff_2: [f32; P_BANDS] = [
        -34.0, -34.0, -34.0, -34.0, -30.0, -26.0, -24.0, -18.0, -14.0, -12.0, -12.0, -12.0, -12.0,
        -12.0, -10.0, -9.0, -5.0,
    ];
    vi.noiseoff[0] = noiseoff_0;
    vi.noiseoff[1] = noiseoff_1;
    vi.noiseoff[2] = noiseoff_2;

    // Noise compander from _psy_compand_44 at compand_long_mapping[5]=5.0 → index 5 = "mode_Z nominal long":
    // {{0,1,2,3,4,5,6,7, 8,9,10,11,12,13,14,15, 16,17,18,19,20,21,22,23, 24,25,26,27,28,29,30,31, 32,33,34,35,36,37,38,39}}
    // (linear passthrough)
    let mut noisecompand = [0.0f32; NOISE_COMPAND_LEVELS];
    for (i, v) in noisecompand.iter_mut().enumerate() {
        *v = i as f32;
    }
    vi.noisecompand = noisecompand;

    // Noise normalization
    // normal_p=1 but normal_start=9999 → effectively disabled
    vi.normal_p = 1;
    vi.normal_start = 9999;
    vi.normal_partition = 32;
    vi.normal_thresh = 9999.0;

    vi
}

// ---------------------------------------------------------------------------
// encode_impl / encode_with_serial: main encode entry point
// ---------------------------------------------------------------------------

fn random_serial() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0x1234_5678);
    t ^ 0xDEAD_BEEF
}

pub(crate) fn encode_impl(samples: &[i16], rate: SampleRate, channels: Channels) -> Vec<u8> {
    encode_with_serial(samples, rate, channels, random_serial())
}

pub(crate) fn encode_with_serial(
    samples: &[i16],
    rate: SampleRate,
    channels: Channels,
    serial: u32,
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

    // Find the mode for long blocks (blockflag=true) — mode index for W=1
    let mode_number = setup.modes.iter().position(|m| m.blockflag).unwrap_or(0);
    let mode = &setup.modes[mode_number];
    let mapping = &setup.mappings[mode.mapping];

    // Build psy state
    let gi = make_q5_psy_global(rate_hz, ch);
    let vi = make_q5_psy(rate_hz);
    let psy_look: VorbisLookPsy = vp_psy_init(vi, &gi, HALF_BLOCK, rate_hz);

    // De-interleave input into per-channel buffers
    let total_samples = if ch > 1 {
        samples.len() / ch
    } else {
        samples.len()
    };
    let pcm_channels: Vec<Vec<f32>> = (0..ch)
        .map(|c| {
            if ch == 1 {
                samples.iter().map(|&s| s as f32).collect()
            } else {
                samples
                    .chunks_exact(ch)
                    .map(|frame| frame[c] as f32)
                    .collect()
            }
        })
        .collect();

    // Determine number of blocks.
    // Each block contributes HALF_BLOCK = 1024 new samples.
    // Pre-pad with 1024 zeros (first block's "previous" is silence).
    // Post-pad with 1024 zeros (flush last block).
    // Total blocks = ceil(total_samples / HALF_BLOCK) + 1 (for flush)
    let data_blocks = total_samples.div_ceil(HALF_BLOCK);
    let total_blocks = data_blocks + 1; // +1 for the post-flush block

    // Granule position for each block: block k output corresponds to
    // granule position (k+1) * HALF_BLOCK, capped at total_samples for last block.
    // In Vorbis, granule position is the number of samples decoded so far.

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
        write_comment_header(&mut w);
        ogg.write_packet(&w.into_bytes(), 0, false, true);
    }
    {
        let mut w = BitWriter::new();
        write_setup_header(rate, channels, &mut w);
        ogg.write_packet(&w.into_bytes(), 0, false, true);
    }

    // Windowing buffers per channel
    let mut win_bufs: Vec<WindowingBuffer> = (0..ch).map(|_| WindowingBuffer::new()).collect();

    // Mutable floor states (mapping0_forward needs &mut)
    // We clone setup's floor states for use during encode
    let mut floor_states: Vec<crate::floor1::Floor1State> = setup
        .floor_states
        .iter()
        .map(|s| crate::floor1::floor1_look(s.vi.clone()))
        .collect();

    let mut ampmax = -9999.0f32;

    for block_idx in 0..total_blocks {
        // Build per-channel 1024-sample blocks
        let block_start = block_idx * HALF_BLOCK;

        let current_blocks: Vec<[f32; HALF_BLOCK]> = (0..ch)
            .map(|c| {
                let mut blk = [0.0f32; HALF_BLOCK];
                for i in 0..HALF_BLOCK {
                    let idx = block_start + i;
                    if idx < total_samples {
                        blk[i] = pcm_channels[c][idx];
                    }
                    // else: zero padding
                }
                blk
            })
            .collect();

        // Apply window + overlap
        let windowed_blocks: Vec<[f32; BLOCK_SIZE]> = (0..ch)
            .map(|c| win_bufs[c].push_block(&current_blocks[c]))
            .collect();

        // Encode this block
        let granule_pos = ((block_idx + 1) as u64 * HALF_BLOCK as u64).min(total_samples as u64);

        let is_last = block_idx == total_blocks - 1;

        let mut w = BitWriter::new();
        mapping0_forward(
            &windowed_blocks,
            &psy_look,
            &gi,
            &mut ampmax,
            &mut floor_states,
            &setup.residue_types,
            &setup.residue_setups,
            &setup.residue_looks,
            mapping,
            mode_number,
            setup.modebits,
            &setup.books,
            &mut w,
        );
        let packet_bytes = w.into_bytes();
        ogg.write_packet(&packet_bytes, granule_pos, is_last, false);
    }

    ogg.into_bytes()
}
