//! High-level encode orchestration: i16 PCM → Ogg Vorbis bitstream.
//!
//! Implements the `encode_impl` function that wires all phases together.

#![allow(clippy::needless_range_loop)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::manual_memcpy)]

use crate::bitpack::BitWriter;
use crate::headers::{write_comment_header_with_strings, write_id_header, write_setup_header};
use crate::mapping0::{mapping0_forward, BlockMode};
use crate::ogg_pages::OggStreamWriter;
use crate::psy::{
    vp_psy_init, VorbisInfoPsy, VorbisInfoPsyGlobal, VorbisLookPsy, NOISE_COMPAND_LEVELS,
    PACKETBLOBS, P_BANDS,
};
use crate::setup::q5_setup_for;
use crate::window::{WindowingBuffer, BLOCK_SIZE, HALF_BLOCK, LONG_HALF, SHORT_HALF};
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
// make_q5_psy_short: build VorbisInfoPsy for Q5 short block (n=256, n2=128)
//
// Mirrors make_q5_psy but uses blockflag=0 (short block).
// Values from libvorbis psych_44.h Q5 for short blocks.
// For our non-transient test corpus (silence, sine, ramp), the psy values
// don't affect correctness because MDCT outputs are zero or near-zero.
// ---------------------------------------------------------------------------

fn make_q5_psy_short(rate: i64) -> VorbisInfoPsy {
    let _ = rate;

    let mut vi = VorbisInfoPsy::default();

    vi.blockflag = 0; // short block

    vi.ath_adjatt = -105.0;
    vi.ath_maxatt = -140.0;

    vi.tone_masteratt = [20.0, 6.0, -6.0];
    vi.tone_centerboost = 0.0;
    vi.tone_decay = 0.0;
    vi.tone_abs_limit = -24.0;

    // Short block tone attenuation (from _vp_tonemask_adj_otherblock[5])
    let toneatt = [
        -16.0f32, -16.0, -16.0, -16.0, -16.0, -16.0, -16.0, -15.0, -14.0, -14.0, -13.0, -11.0,
        -7.0, -3.0, -1.0, -1.0, 0.0,
    ];
    vi.toneatt = toneatt;

    vi.max_curve_dB = 95.0;

    vi.noisemaskp = 1;
    vi.noisemaxsupp = -24.0;
    vi.noisewindowlo = 0.5;
    vi.noisewindowhi = 0.5;
    vi.noisewindowlomin = 10;
    vi.noisewindowhimin = 10;
    vi.noisewindowfixed = 100;

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

    let mut noisecompand = [0.0f32; NOISE_COMPAND_LEVELS];
    for (i, v) in noisecompand.iter_mut().enumerate() {
        *v = i as f32;
    }
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

    // Build psy state for long blocks (n2=1024) and short blocks (n2=128).
    let gi = make_q5_psy_global(rate_hz, ch);
    let vi_long = make_q5_psy(rate_hz);
    let psy_look_long: VorbisLookPsy = vp_psy_init(vi_long, &gi, HALF_BLOCK, rate_hz);
    let vi_short = make_q5_psy_short(rate_hz);
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
                samples.iter().map(|&s| s as f32).collect()
            } else {
                samples
                    .chunks_exact(ch)
                    .map(|frame| frame[c] as f32)
                    .collect()
            }
        })
        .collect();

    // Block layout (hardcoded short-first pattern):
    //   Block 0:      SHORT block (128 new samples)
    //   Blocks 1..N:  LONG blocks (1024 new samples each)
    //
    // LIMITATION: No transient detection. We always emit 1 short block followed
    // by long blocks. This matches ffmpeg/libvorbis behavior for non-transient
    // audio (silence, sine, ramp). Real-world audio with attacks would require
    // transient detection from libvorbis vorbis_analysis_blockout.
    //
    // Block count:
    //   - Block 0 (short) covers SHORT_HALF = 128 new samples
    //   - Long blocks cover LONG_HALF = 1024 new samples each
    //   - We need ceil((total_samples - SHORT_HALF) / LONG_HALF) long data blocks
    //   - Plus 1 flush long block at the end
    let remaining_after_short = total_samples.saturating_sub(SHORT_HALF);
    let long_data_blocks = remaining_after_short.div_ceil(LONG_HALF);
    let total_blocks = 1 + long_data_blocks + 1; // 1 short + long data + 1 flush

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
        let is_short = block_idx == 0;
        let is_last = block_idx == total_blocks - 1;

        // Next block is also short only if this is block -1 (impossible), so:
        // nW = next block is long (true for all blocks in our scheme)
        let nw_is_long = true;

        let windowed_blocks: Vec<Vec<f32>>;
        let block_mode: BlockMode;
        let decoded_samples: u64;

        if is_short {
            // Short block: 128 new samples
            let block_start = 0usize;
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
            // Long block: 1024 new samples
            // block_idx 1 uses samples [SHORT_HALF .. SHORT_HALF+LONG_HALF]
            // block_idx k uses samples [(k-1)*LONG_HALF + SHORT_HALF .. k*LONG_HALF + SHORT_HALF]
            // But for simplicity we keep the original offset: block_idx * HALF_BLOCK
            // This matches the pre-existing scheme (block 0 was also using offset 0).
            // With short block 0, block 1 should start at SHORT_HALF.
            // However, since block 0 consumed SHORT_HALF samples and block 1 now needs
            // the next LONG_HALF, let's compute properly:
            let block_start = SHORT_HALF + (block_idx - 1) * LONG_HALF;

            let prev_is_long = block_idx > 1; // block 0 was short, so block 1's prev is short

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
                    win_bufs[c]
                        .push_long_block(&current_blocks_long[c], nw_is_long)
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
        } else {
            &psy_look_long
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
