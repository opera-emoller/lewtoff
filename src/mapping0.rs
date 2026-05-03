//! mapping0_forward: literal port of libvorbis 1.3.7 `lib/mapping0.c`
//! `mapping0_forward` function (lines 230-697).
//!
//! Drives the full encoding pipeline for one audio block:
//!   PCM windowed → MDCT → psy → floor1 → quantize/couple → residue → bits

#![allow(clippy::needless_range_loop)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::assign_op_pattern)]
#![allow(non_snake_case)]
#![allow(unused_variables)]
#![allow(unused_mut)]

use crate::bitpack::BitWriter;
use crate::drft::{drft_forward_long, drft_forward_short};
use crate::floor1::{floor1_encode, floor1_fit, Floor1State};
use crate::mdct::{mdct_forward_long, mdct_forward_short};
use crate::psy::{
    to_db, vp_ampmax_decay, vp_noisemask, vp_offset_and_mix, vp_tonemask, VorbisInfoPsyGlobal,
    VorbisLookPsy,
};
use crate::residue::{
    res1_class, res1_forward, res2_class, res2_forward, ResidueLook, ResidueSetup,
};
use crate::setup::Mapping;
use crate::window::{LONG_BLOCK, LONG_HALF, SHORT_BLOCK, SHORT_HALF};

// ---------------------------------------------------------------------------
// mapping0_forward
//
// Arguments:
//   pcm_blocks   - per-channel 2048-sample windowed PCM blocks
//   psy_look     - psy analysis state (single shared look used for all channels)
//   gi           - global psy info (for ampmax decay, coupling)
//   ampmax       - mutable current amplitude max (updated in place)
//   floor_states - per-channel mutable floor1 states (already mutable borrow)
//   residue_types - per-residue type
//   residue_setups - per-residue setup
//   residue_looks  - per-residue runtime look
//   mapping      - the mapping config for this mode
//   mode_number  - mode index to encode in the packet header
//   modebits     - number of bits for mode number
//   books        - full codebook slice
//   opb          - output bit writer
// ---------------------------------------------------------------------------

/// Parameters for mode/window flags in the audio packet header.
pub(crate) struct BlockMode {
    pub mode_number: usize,
    pub modebits: u32,
    /// Is this a long block? (false = short block, n=256)
    pub is_long: bool,
    /// Was the previous block long? (only written for long blocks)
    pub prev_window: bool,
    /// Is the next block long? (only written for long blocks)
    pub next_window: bool,
}

pub(crate) fn mapping0_forward(
    pcm_blocks: &[Vec<f32>], // per-channel windowed PCM, variable length (256 or 2048)
    psy_look: &VorbisLookPsy,
    gi: &VorbisInfoPsyGlobal,
    ampmax: &mut f32,
    floor_states: &mut [Floor1State],
    residue_types: &[u16],
    residue_setups: &[ResidueSetup],
    residue_looks: &[ResidueLook],
    mapping: &Mapping,
    block_mode: &BlockMode,
    books: &[crate::codebook::Codebook],
    opb: &mut BitWriter,
) {
    let n = if block_mode.is_long {
        LONG_BLOCK
    } else {
        SHORT_BLOCK
    };
    let n2 = n / 2;
    let channels = pcm_blocks.len();

    // Scale factor for dB computation (port of `scale = 4.f/n; scale_dB = todB(&scale) + .345`)
    let scale = 4.0f32 / n as f32;
    let scale_dB = to_db(scale) + 0.345_f32;

    // Per-channel MDCT output (gmdct)
    let mut gmdct: Vec<Vec<f32>> = vec![vec![0.0f32; n2]; channels];

    // Per-channel log-FFT approximation (logfft, overlaid on pcm scratch)
    let mut logfft: Vec<Vec<f32>> = vec![vec![0.0f32; n2]; channels];

    // Per-channel local amplitude max
    let mut local_ampmax = vec![0.0f32; channels];

    // Apply ampmax decay at the START of each block (port of block.c _vp_ampmax_decay call).
    // libvorbis decays g->ampmax before calling mapping0_forward, using the CURRENT block's n2.
    // This ensures that a long block (n2=1024) decays more than a short block (n2=128).
    *ampmax = vp_ampmax_decay(*ampmax, gi, n2, psy_look.rate);

    if std::env::var("LW_DEBUG_PCM").is_ok() {
        for i in 0..channels {
            let vals: Vec<String> = pcm_blocks[i].iter().map(|v| format!("{:.8}", v)).collect();
            eprintln!("LW_WINDOWED_PCM ch={} n={}: [{}]", i, n, vals.join(","));
        }
    }
    // Apply window (already done by caller), MDCT, and compute log-FFT approx
    // Port of: lines 254-360 in mapping0.c
    for i in 0..channels {
        // MDCT — dispatch on block size
        if block_mode.is_long {
            let pcm_arr: &[f32; LONG_BLOCK] = pcm_blocks[i]
                .as_slice()
                .try_into()
                .expect("long block size");
            let mut out = [0.0f32; LONG_HALF];
            mdct_forward_long(pcm_arr, &mut out);
            gmdct[i].copy_from_slice(&out);
        } else {
            let pcm_arr: &[f32; SHORT_BLOCK] = pcm_blocks[i]
                .as_slice()
                .try_into()
                .expect("short block size");
            let mut out = [0.0f32; SHORT_HALF];
            mdct_forward_short(pcm_arr, &mut out);
            gmdct[i].copy_from_slice(&out);
        }

        // Compute log-magnitude spectrum via real FFT on the windowed PCM.
        // Port of mapping0.c lines 308-337:
        //   drft_forward(&b->fft_look[vb->W], pcm);
        //   logfft[0] = scale_dB + todB(pcm) + .345;
        //   for j=1; j<n-1; j+=2:
        //     temp = pcm[j]^2 + pcm[j+1]^2
        //     logfft[(j+1)>>1] = scale_dB + 0.5*todB(&temp) + .345
        //     local_ampmax[i] = max(...)
        if block_mode.is_long {
            let mut fft_buf = [0.0f32; LONG_BLOCK];
            fft_buf.copy_from_slice(&pcm_blocks[i]);
            drft_forward_long(&mut fft_buf);
            // DC bin
            let lam = scale_dB + to_db(fft_buf[0]) + 0.345_f32;
            logfft[i][0] = lam;
            local_ampmax[i] = lam;
            // Bins 1..n/2
            let mut j = 1usize;
            while j < n - 1 {
                let temp = fft_buf[j] * fft_buf[j] + fft_buf[j + 1] * fft_buf[j + 1];
                let t = scale_dB + 0.5_f32 * to_db(temp) + 0.345_f32;
                logfft[i][(j + 1) >> 1] = t;
                if t > local_ampmax[i] {
                    local_ampmax[i] = t;
                }
                j += 2;
            }
        } else {
            let mut fft_buf = [0.0f32; SHORT_BLOCK];
            fft_buf.copy_from_slice(&pcm_blocks[i]);
            drft_forward_short(&mut fft_buf);
            let lam = scale_dB + to_db(fft_buf[0]) + 0.345_f32;
            logfft[i][0] = lam;
            local_ampmax[i] = lam;
            let mut j = 1usize;
            while j < n - 1 {
                let temp = fft_buf[j] * fft_buf[j] + fft_buf[j + 1] * fft_buf[j + 1];
                let t = scale_dB + 0.5_f32 * to_db(temp) + 0.345_f32;
                logfft[i][(j + 1) >> 1] = t;
                if t > local_ampmax[i] {
                    local_ampmax[i] = t;
                }
                j += 2;
            }
        }
        if local_ampmax[i] > 0.0 {
            local_ampmax[i] = 0.0;
        }
        if local_ampmax[i] > *ampmax {
            *ampmax = local_ampmax[i];
        }
        // DEBUG
        if std::env::var("LW_DEBUG_FFT").is_ok() {
            eprintln!(
                "ch={} n={} local_ampmax={:.2} logfft[0..5]={:?}",
                i,
                n,
                local_ampmax[i],
                &logfft[i][..5.min(logfft[i].len())]
            );
        }
    }

    // Noise + tone masking per channel, then floor1 fit
    // Port of: lines 362-576 in mapping0.c (minus bitrate management, we use offset_select=1 only)
    let mut floor_posts: Vec<Option<Vec<i32>>> = vec![None; channels];
    let mut noise_bufs: Vec<Vec<f32>> = vec![vec![0.0f32; n2]; channels];
    let mut tone_bufs: Vec<Vec<f32>> = vec![vec![0.0f32; n2]; channels];

    // We need gmdct_copy for use after the loop (psy modifies it in vp_offset_and_mix)
    let mut gmdct_work: Vec<Vec<f32>> = gmdct.clone();

    for i in 0..channels {
        let submap = mapping.chmuxlist[i];

        let logmdct: Vec<f32> = gmdct_work[i]
            .iter()
            .map(|&v| to_db(v.abs()) + 0.345_f32)
            .collect();
        if std::env::var("LW_DEBUG_MDCT_RAW").is_ok() {
            for &b in &[14usize, 224, 265, 269, 273, 347, 490] {
                if b < n2 {
                    eprintln!(
                        "LW_MDCT_RAW ch={} n={} bin={}: gmdct={:.10e} logmdct={:.4}",
                        i, n, b, gmdct_work[i][b], logmdct[b]
                    );
                }
            }
        }

        let mut logmask = vec![0.0f32; n2];

        // Noise mask
        vp_noisemask(psy_look, &logmdct, &mut noise_bufs[i]);

        // Tone mask
        vp_tonemask(
            psy_look,
            &logfft[i],
            &mut tone_bufs[i],
            *ampmax,
            local_ampmax[i],
        );

        if std::env::var("LW_DEBUG_TONE").is_ok() {
            let vals: Vec<String> = tone_bufs[i].iter().map(|v| format!("{:.6}", v)).collect();
            eprintln!("LW_TONE: [{}]", vals.join(","));
            let nvals: Vec<String> = noise_bufs[i].iter().map(|v| format!("{:.6}", v)).collect();
            eprintln!("LW_NOISE: [{}]", nvals.join(","));
        }
        if std::env::var("LW_DEBUG_FULLNOISE").is_ok() {
            let nvals: Vec<String> = noise_bufs[i].iter().map(|v| format!("{:.6}", v)).collect();
            eprintln!("LW_BLOCK0_NOISE: [{}]", nvals.join(","));
        }
        // Offset + mix (offset_select=1 = nominal, not bitrate managed)
        vp_offset_and_mix(
            psy_look,
            &noise_bufs[i].clone(),
            &tone_bufs[i].clone(),
            1,
            &mut logmask,
            &mut gmdct_work[i],
            &logmdct,
        );

        // floor1_fit
        let floor_state = &floor_states[mapping.floorsubmap[submap]];
        if std::env::var("LW_DEBUG_LOGMDCT").is_ok() {
            let vals: Vec<String> = logmdct.iter().map(|v| format!("{:.6}", v)).collect();
            eprintln!("LW_BLOCK0_LOGMDCT: [{}]", vals.join(","));
            let mvals: Vec<String> = logmask.iter().map(|v| format!("{:.6}", v)).collect();
            eprintln!("LW_BLOCK0_LOGMASK: [{}]", mvals.join(","));
        }
        if std::env::var("LW_DEBUG_PSY").is_ok() {
            eprintln!(
                "LW_LOGMASK ch={} n={} ampmax={:.2} [0..5]: {:.2} {:.2} {:.2} {:.2} {:.2}",
                i, n, *ampmax, logmask[0], logmask[1], logmask[2], logmask[3], logmask[4]
            );
        }
        floor_posts[i] = floor1_fit(floor_state, &logmdct, &logmask);
        if std::env::var("LW_DEBUG_FLOOR").is_ok() {
            if let Some(ref posts) = floor_posts[i] {
                eprintln!(
                    "LW_FLOOR ch={} n={} ampmax={:.2} floor_posts={:?}",
                    i, n, *ampmax, posts
                );
            } else {
                eprintln!(
                    "LW_FLOOR ch={} n={} ampmax={:.2} floor_posts=None",
                    i, n, *ampmax
                );
            }
        }
    }

    // --- Encode the packet ---
    // Port of: lines 592-688 in mapping0.c
    // (we don't do bitrate management: only k = PACKETBLOBS/2)

    // Encode packet type (audio = 0)
    opb.write(0, 1);
    // Encode mode number
    opb.write(block_mode.mode_number as u32, block_mode.modebits);
    // For long blocks (W=1): encode lW and nW window flags.
    // For short blocks (W=0): no window flags (the C code: if(vb->W) {...}).
    if block_mode.is_long {
        opb.write(if block_mode.prev_window { 1 } else { 0 }, 1);
        opb.write(if block_mode.next_window { 1 } else { 0 }, 1);
    }

    // Per-channel ilogmask (integer quantized floor)
    let mut iwork: Vec<Vec<i32>> = vec![vec![0i32; n2]; channels];
    let mut nonzero = vec![0i32; channels];

    for i in 0..channels {
        let submap = mapping.chmuxlist[i];
        let floor_state_idx = mapping.floorsubmap[submap];
        let floor_state = &mut floor_states[floor_state_idx];

        nonzero[i] = floor1_encode(
            opb,
            floor_state,
            floor_posts[i].as_mut(),
            &mut iwork[i],
            books,
            n,
        );
    }

    // Couple + quantize + normalize
    // Use blobno = PACKETBLOBS/2 = 7 (nominal quality blob)
    let blobno = crate::psy::PACKETBLOBS / 2;
    let w_flag = if block_mode.is_long { 1 } else { 0 };
    let sliding_lowpass = gi.sliding_lowpass[w_flag][blobno];

    crate::psy::vp_couple_quantize_normalize(
        blobno,
        gi,
        psy_look,
        &mapping.vp_mapping,
        &mut gmdct_work,
        &mut iwork,
        &mut nonzero,
        sliding_lowpass,
        channels,
    );

    // Encode residue by submap
    for submap_idx in 0..mapping.submaps {
        let res_idx = mapping.residuesubmap[submap_idx];
        let res_type = residue_types[res_idx];
        let res_setup = &residue_setups[res_idx];
        let res_look = &residue_looks[res_idx];

        // Gather channels in this submap
        let mut couple_bundle_indices: Vec<usize> = Vec::new();
        for j in 0..channels {
            if mapping.chmuxlist[j] == submap_idx {
                couple_bundle_indices.push(j);
            }
        }
        let ch_in_bundle = couple_bundle_indices.len();

        let zerobundle: Vec<bool> = couple_bundle_indices
            .iter()
            .map(|&ci| nonzero[ci] != 0)
            .collect();

        match res_type {
            0 | 1 => {
                // res1 (and res0, treated same for encode purposes)
                // Classify (immutable)
                let in_slices: Vec<&[i32]> = couple_bundle_indices
                    .iter()
                    .map(|&ci| iwork[ci].as_slice())
                    .collect();
                let mut in_slices_for_class: Vec<&[i32]> = in_slices;
                let partword = res1_class(
                    res_setup,
                    res_look,
                    &mut in_slices_for_class,
                    &zerobundle,
                    ch_in_bundle,
                );
                if let Some(pw) = partword {
                    // Clone the relevant channel data, encode with mutable access, write back
                    let mut local: Vec<Vec<i32>> = couple_bundle_indices
                        .iter()
                        .map(|&ci| iwork[ci].clone())
                        .collect();
                    {
                        let mut in_mut: Vec<&mut [i32]> =
                            local.iter_mut().map(|v| v.as_mut_slice()).collect();
                        res1_forward(
                            opb,
                            res_setup,
                            res_look,
                            &mut in_mut,
                            &zerobundle,
                            ch_in_bundle,
                            &pw,
                            books,
                        );
                    }
                    for (k, &ci) in couple_bundle_indices.iter().enumerate() {
                        iwork[ci].copy_from_slice(&local[k]);
                    }
                }
            }
            2 => {
                // res2: interleaved stereo
                let in_slices: Vec<&[i32]> = couple_bundle_indices
                    .iter()
                    .map(|&ci| iwork[ci].as_slice())
                    .collect();
                let partword =
                    res2_class(res_setup, res_look, &in_slices, &zerobundle, ch_in_bundle);
                if let Some(pw) = partword {
                    res2_forward(
                        opb,
                        res_setup,
                        res_look,
                        &in_slices,
                        &zerobundle,
                        ch_in_bundle,
                        n2,
                        &pw,
                        books,
                    );
                }
            }
            _ => {}
        }
    }
}
