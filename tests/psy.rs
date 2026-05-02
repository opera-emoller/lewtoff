//! Psychoacoustic model tests.
//!
//! Task 5.3: numerical determinism test.  Run once to generate the expected
//! binary, commit it, then the test locks cross-platform byte equality.

#![allow(clippy::field_reassign_with_default)]

use lewtoff::psy::{vp_noisemask, vp_psy_init, vp_tonemask, VorbisInfoPsy, VorbisInfoPsyGlobal};

/// Build a minimal VorbisInfoPsy matching libvorbis Q5 long-block params.
/// These values are taken from lib/modes/setup_44.h / psych_44.h for the
/// "short" psy_param[0] used at Q5.  They are approximate but sufficient to
/// exercise the code paths.
fn make_vi() -> VorbisInfoPsy {
    use lewtoff::psy::{NOISE_COMPAND_LEVELS, P_BANDS, P_NOISECURVES};
    let mut vi = VorbisInfoPsy::default();
    vi.blockflag = 1;
    vi.ath_adjatt = -140.0;
    vi.ath_maxatt = -140.0;
    vi.tone_masteratt = [0.0, -14.0, -6.0];
    vi.tone_centerboost = 0.0;
    vi.tone_decay = -8.0;
    vi.tone_abs_limit = -200.0;
    vi.toneatt = [40.0; P_BANDS];
    vi.noisemaskp = 1;
    vi.noisemaxsupp = 10.0;
    vi.noisewindowlo = 0.5;
    vi.noisewindowhi = 0.5;
    vi.noisewindowlomin = 4;
    vi.noisewindowhimin = 6;
    vi.noisewindowfixed = 8;
    vi.noiseoff = [[0.0; P_BANDS]; P_NOISECURVES];
    vi.noisecompand = {
        let mut c = [0.0_f32; NOISE_COMPAND_LEVELS];
        for (i, v) in c.iter_mut().enumerate() {
            *v = i as f32 * 0.1;
        }
        c
    };
    vi.max_curve_dB = 100.0;
    vi.normal_p = 0;
    vi.normal_start = 256;
    vi.normal_partition = 32;
    vi.normal_thresh = 0.25;
    vi
}

fn make_gi() -> VorbisInfoPsyGlobal {
    let mut gi = VorbisInfoPsyGlobal::default();
    gi.eighth_octave_lines = 8;
    gi
}

#[test]
fn psy_smoke() {
    let vi = make_vi();
    let gi = make_gi();
    let n = 256usize;
    let rate = 44100i64;
    let p = vp_psy_init(vi, &gi, n, rate);
    assert_eq!(p.n, n);
    assert_eq!(p.ath.len(), n);
    assert_eq!(p.bark.len(), n);
    assert_eq!(p.octave.len(), n);
}

#[test]
fn psy_noisemask_finite() {
    let vi = make_vi();
    let gi = make_gi();
    let n = 256usize;
    let rate = 44100i64;
    let p = vp_psy_init(vi, &gi, n, rate);

    let logmdct: Vec<f32> = (0..n).map(|i| i as f32 * 0.01 - 40.0).collect();
    let mut logmask = vec![0.0f32; n];
    vp_noisemask(&p, &logmdct, &mut logmask);

    for &v in &logmask {
        assert!(v.is_finite(), "logmask contains non-finite value: {v}");
    }
}

#[test]
fn psy_tonemask_finite() {
    let vi = make_vi();
    let gi = make_gi();
    let n = 256usize;
    let rate = 44100i64;
    let p = vp_psy_init(vi, &gi, n, rate);

    let logfft: Vec<f32> = (0..n).map(|i| i as f32 * 0.01 - 50.0).collect();
    let mut logmask = vec![0.0f32; n];
    vp_tonemask(&p, &logfft, &mut logmask, 0.0, -50.0);

    for &v in &logmask {
        assert!(v.is_finite() || v == -9999.0, "unexpected value: {v}");
    }
}

#[test]
fn psy_compute_mask_determinism() {
    let vi = make_vi();
    let gi = make_gi();
    let n = 1024usize;
    let rate = 44100i64;
    let p = vp_psy_init(vi, &gi, n, rate);

    let spectrum: Vec<f32> = (0..n).map(|i| (i as f32) * 0.001 - 30.0).collect();

    let mut noise_mask = vec![0.0f32; n];
    vp_noisemask(&p, &spectrum, &mut noise_mask);

    let mut tone_mask = vec![0.0f32; n];
    vp_tonemask(&p, &spectrum, &mut tone_mask, -10.0, -30.0);

    // combine: element-wise max of noise and tone masks
    let mask: Vec<f32> = noise_mask
        .iter()
        .zip(tone_mask.iter())
        .map(|(&n, &t)| n.max(t))
        .collect();

    let actual_bytes: Vec<u8> = mask.iter().flat_map(|f| f.to_le_bytes()).collect();

    let expected_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/psy/mask_default.bin");

    if !expected_path.exists() {
        // First run: write the expected file and pass
        std::fs::write(&expected_path, &actual_bytes).expect("failed to write expected bytes");
        eprintln!("Wrote {} bytes to {:?}", actual_bytes.len(), expected_path);
        return;
    }

    let expected_bytes = std::fs::read(&expected_path).expect("failed to read expected bytes");
    assert_eq!(
        actual_bytes.as_slice(),
        expected_bytes.as_slice(),
        "psy mask output differs from committed expected bytes — cross-platform determinism failure"
    );
}
