//! LPC (Linear Predictive Coding) for pre-stream extrapolation.
//!
//! Port of libvorbis lib/lpc.c: `vorbis_lpc_from_data` and `vorbis_lpc_predict`.
//! Used by `_preextrapolate_helper` in block.c to fill in pre-stream samples
//! using backward prediction from actual audio data.

/// Compute LPC coefficients from data using Levinson-Durbin algorithm.
/// Port of libvorbis `vorbis_lpc_from_data`.
///
/// `data`: n samples
/// `lpci`: output LPC coefficients (length m)
/// `n`: number of input samples
/// `m`: LPC order
/// Returns the prediction error energy.
pub fn lpc_from_data(data: &[f32], lpci: &mut [f32], n: usize, m: usize) -> f32 {
    let mut aut = vec![0.0f64; m + 1];
    let mut lpc = vec![0.0f64; m];

    // Autocorrelation, m+1 lag coefficients.
    // Use mul_add to match the FMA (fused multiply-add) behavior of clang -O2 on arm64,
    // which is used by ffmpeg/libvorbis. Without FMA, the accumulated sums differ by
    // ~1e-7 leading to slightly different LPC coefficients and predicted samples.
    for j in 0..=m {
        let mut d = 0.0f64;
        for i in j..n {
            d = (data[i] as f64).mul_add(data[i - j] as f64, d);
        }
        aut[j] = d;
    }
    if std::env::var("LW_DEBUG_LPC").is_ok() {
        eprint!("LW_AUT[0..5]:");
        for val in aut.iter().take(5) {
            eprint!(" {:.15e}", val);
        }
        eprintln!();
    }

    // Generate LPC coefficients from autocorr values (Levinson-Durbin)
    let mut error = aut[0] * (1.0 + 1e-10);
    let epsilon = 1e-9 * aut[0] + 1e-10;

    for i in 0..m {
        if error < epsilon {
            for val in lpc[i..m].iter_mut() {
                *val = 0.0;
            }
            break;
        }

        let mut r = -aut[i + 1];
        for j in 0..i {
            r -= lpc[j] * aut[i - j];
        }
        r /= error;

        lpc[i] = r;
        for j in 0..i / 2 {
            let tmp = lpc[j];
            lpc[j] += r * lpc[i - 1 - j];
            lpc[i - 1 - j] += r * tmp;
        }
        if i & 1 != 0 {
            lpc[i / 2] += lpc[i / 2] * r;
        }

        error *= 1.0 - r * r;
    }

    // Slightly damp the filter
    {
        let g = 0.99f64;
        let mut damp = g;
        for val in lpc[..m].iter_mut() {
            *val *= damp;
            damp *= g;
        }
    }

    for (out, &val) in lpci[..m].iter_mut().zip(lpc[..m].iter()) {
        *out = val as f32;
    }

    error as f32
}

/// Predict n samples from LPC coefficients and prime values.
/// Port of libvorbis `vorbis_lpc_predict`.
///
/// `coeff`: LPC coefficients (length m)
/// `prime`: initial values (length m)
/// `m`: LPC order
/// `data`: output predicted samples (length n)
/// `n`: number of samples to predict
pub fn lpc_predict(coeff: &[f32], prime: &[f32], m: usize, data: &mut [f32], n: usize) {
    let mut work = vec![0.0f32; m + n];

    if !prime.is_empty() {
        work[..m].copy_from_slice(&prime[..m]);
    }

    for i in 0..n {
        let mut y = 0.0f32;
        for (o, p) in (i..i + m).zip((0..m).rev()) {
            y -= work[o] * coeff[p];
        }
        data[i] = y;
        work[m + i] = y;
    }
}

/// Compute pre-stream extrapolated samples using the same algorithm as
/// libvorbis `_preextrapolate_helper`.
///
/// `pcm`: the actual PCM samples (at least `order*2` samples needed)
/// `order`: LPC order (libvorbis uses 16)
/// `n_predict`: number of pre-stream samples to generate (typically `centerW = 1024`)
///
/// Returns a Vec of `n_predict` pre-stream samples, where index 0 corresponds
/// to the sample just before the stream start (index -1), and index n_predict-1
/// corresponds to sample at -(n_predict).
///
/// In libvorbis, the pre-stream buffer is the REVERSED time domain:
/// `work[j] = pcm[pcm_current - j - 1]` (reversed), then LPC, then predict,
/// then reverse back. This predicts BACKWARD from the stream start.
#[allow(dead_code)]
pub fn preextrapolate(pcm: &[f32], order: usize, n_predict: usize) -> Vec<f32> {
    let n_data = pcm.len();

    if n_data <= order * 2 {
        // Not enough data for LPC
        return vec![0.0; n_predict];
    }

    // Reverse the PCM data (libvorbis reverses the entire pcm_current worth)
    let mut work: Vec<f32> = pcm.iter().rev().cloned().collect();

    // Compute LPC from the reversed data
    let mut lpc_coeffs = vec![0.0f32; order];
    lpc_from_data(&work, &mut lpc_coeffs, n_data, order);

    // Predict n_predict samples forward in the reversed domain
    // The prime (initial values) are work[n_data-order..n_data]
    let prime: Vec<f32> = work[n_data - order..n_data].to_vec();
    let mut predicted = vec![0.0f32; n_predict];
    lpc_predict(&lpc_coeffs, &prime, order, &mut predicted, n_predict);

    // Append predicted to work
    work.extend_from_slice(&predicted);

    // Now extract the pre-stream samples by reversing back.
    // In libvorbis: `for j in 0..pcm_current: v->pcm[i][pcm_current-j-1] = work[j]`
    // This reversal writes work[0..pcm_current] back into pcm[pcm_current-1..0].
    // The predicted samples (work[n_data..n_data+n_predict]) get placed at
    // indices n_data+n_predict-1..n_data (= n_predict-1..0 in the output pcm).
    //
    // After the reversal, v->pcm[i][0..centerW] is filled with predicted values
    // in the right order. The sample at pcm[0] = work[pcm_current-1+n_predict] reversed...
    //
    // Actually let me trace through more carefully:
    // work[j] = pcm_reversed[j] for j in 0..n_data (= pcm_current in libvorbis)
    // work[n_data..n_data+n_predict] = predicted (= predicted samples in reverse time)
    //
    // The reversal back: v->pcm[i][pcm_current-j-1] = work[j]
    // For j in 0..n_data: this writes work[0..n_data] back into pcm[n_data-1..0]
    //   = restores original pcm
    // For j in n_data..n_data+n_predict: this writes predicted[0..n_predict] into
    //   pcm[n_data+n_predict-1-n_data..n_data+n_predict-1-n_data+n_predict]
    //   Wait, that's out of range...
    //
    // Actually libvorbis only does the reversal for j in 0..pcm_current = 0..n_data.
    // The predicted samples extend work beyond work[n_data-1], so they go into
    // v->pcm[i][n_data-1-n_data .. -1] which is negative indices!
    //
    // Hmm. Let me re-read _preextrapolate_helper:
    //
    //   for(j=0;j<v->pcm_current;j++)
    //     work[j]=v->pcm[i][v->pcm_current-j-1];   // work = reversed pcm[0..pcm_current]
    //   vorbis_lpc_from_data(work, lpc, pcm_current-centerW, order);
    //   vorbis_lpc_predict(lpc, work+pcm_current-centerW-order, order,
    //                      work+pcm_current-centerW, centerW);  // predict centerW samples
    //   for(j=0;j<v->pcm_current;j++)
    //     v->pcm[i][v->pcm_current-j-1]=work[j];   // write back
    //
    // KEY: the predict call fills work[pcm_current-centerW .. pcm_current-centerW+centerW]
    //      = work[pcm_current-centerW .. pcm_current]
    // Then the write-back writes work[j] -> pcm[pcm_current-j-1]:
    //   work[pcm_current-centerW] -> pcm[centerW-1]
    //   work[pcm_current-1]       -> pcm[0]
    //
    // So the predicted samples (work[pcm_current-centerW..pcm_current])
    // are written into pcm[0..centerW]!
    //
    // pcm[0..centerW] is the pre-stream region (the padded zeros).
    // pcm[centerW..pcm_current] is the actual audio data (unchanged).
    //
    // The predict fills work[pcm_current-centerW..pcm_current]:
    // Prime = work[pcm_current-centerW-order..pcm_current-centerW]
    //       = reversed audio data near the stream start
    // Predicted = work[pcm_current-centerW..pcm_current]
    //           = continuation of the reversed audio = backward extrapolation
    //
    // Write-back: work[pcm_current-centerW+k] -> pcm[centerW-1-k]
    // So predicted[0] -> pcm[centerW-1] (= virtual sample at -1)
    //    predicted[1] -> pcm[centerW-2] (= virtual sample at -2)
    //    ...
    //    predicted[centerW-1] -> pcm[0]  (= virtual sample at -centerW)
    //
    // So predicted[0] = virtual sample at -1 (closest to stream start)
    //    predicted[1] = virtual sample at -2
    //    etc.

    // Simpler implementation: predicted[i] corresponds to virtual sample at -(i+1)
    predicted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lpc_sine_prediction() {
        // Generate 256 samples of 440Hz sine at 44100Hz
        let n = 256usize;
        let rate = 44100.0f32;
        let freq = 440.0f32;
        let amp = 0.5f32;
        let pcm: Vec<f32> = (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / rate).sin())
            .collect();

        // Predict 128 pre-stream samples
        let predicted = preextrapolate(&pcm, 16, 128);

        // predicted[0] should be close to pcm[-1] = amp * sin(-2π*440/44100)
        let expected_neg1 = amp * (-2.0 * std::f32::consts::PI * freq / rate).sin();
        // LPC prediction may not be perfect with only 256 input samples,
        // but it should be in the right ballpark
        eprintln!(
            "predicted[0]={:.4} expected_neg1={:.4}",
            predicted[0], expected_neg1
        );
        // Just ensure it's not zero (prediction actually ran)
        assert!(
            predicted[0].abs() > 1e-6,
            "prediction should be non-zero for sine"
        );
    }
}
