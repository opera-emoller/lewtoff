// Literal port of drftf1 + butterfly helpers from libvorbis/lib/smallft.c.
// Variable names, loop structure, and indexing match the C source exactly.
// Only the forward transform is needed; no backward path.
//
// For n=256 and n=2048 (both pure powers of 2) the factorizations only produce
// radix-2 and radix-4 factors, so dradfg (generic radix) is never reached.

#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_range_loop)]
#![allow(non_snake_case)]

use crate::tables::drft::{DRFT_SPLIT_2048, DRFT_SPLIT_256, DRFT_TRIG_2048, DRFT_TRIG_256};
use crate::window::{LONG_BLOCK, SHORT_BLOCK};

// ---------------------------------------------------------------------------
// dradf2  (radix-2 forward butterfly)
// Port of smallft.c lines 113-166.
// ---------------------------------------------------------------------------
fn dradf2(ido: usize, l1: usize, cc: &[f32], ch: &mut [f32], wa1: &[f32]) {
    let t0 = l1 * ido;

    // DC butterfly
    let mut t1 = 0usize;
    let mut t2 = t0;
    let t3_const = ido << 1;
    for _k in 0..l1 {
        ch[t1 << 1] = cc[t1] + cc[t2];
        ch[(t1 << 1) + t3_const - 1] = cc[t1] - cc[t2];
        t1 += ido;
        t2 += ido;
    }

    if ido < 2 {
        return;
    }
    if ido != 2 {
        // Twiddle loop
        let mut t1 = 0usize;
        let mut t2 = t0;
        for _k in 0..l1 {
            let mut t3 = t2;
            let mut t4 = (t1 << 1) + (ido << 1);
            let mut t5 = t1;
            let mut t6 = t1 << 1;
            let mut i = 2usize;
            while i < ido {
                t3 += 2;
                t4 -= 2;
                t5 += 2;
                t6 += 2;
                let tr2 = wa1[i - 2] * cc[t3 - 1] + wa1[i - 1] * cc[t3];
                let ti2 = wa1[i - 2] * cc[t3] - wa1[i - 1] * cc[t3 - 1];
                ch[t6] = cc[t5] + ti2;
                ch[t4] = ti2 - cc[t5];
                ch[t6 - 1] = cc[t5 - 1] + tr2;
                ch[t4 - 1] = cc[t5 - 1] - tr2;
                i += 2;
            }
            t1 += ido;
            t2 += ido;
        }
        if ido % 2 == 1 {
            return;
        }
    }

    // L105: Nyquist terms (ido even, or ido==2 falls through here)
    // C: t3=(t2=(t1=ido)-1); t2+=t0;
    // So: t1=ido, t2=ido-1+t0, t3=ido-1
    let mut t1 = ido;
    let mut t2 = (ido - 1) + t0;
    let mut t3 = ido - 1;
    for _k in 0..l1 {
        ch[t1] = -cc[t2];
        ch[t1 - 1] = cc[t3];
        t1 += ido << 1;
        t2 += ido;
        t3 += ido;
    }
}

// ---------------------------------------------------------------------------
// dradf4  (radix-4 forward butterfly)
// Port of smallft.c lines 168-268.
// ---------------------------------------------------------------------------
fn dradf4(
    ido: usize,
    l1: usize,
    cc: &[f32],
    ch: &mut [f32],
    wa1: &[f32],
    wa2: &[f32],
    wa3: &[f32],
) {
    // libvorbis: static const float hsqt2 = .70710678118654752f;
    // 17 sig digits round-trip to f32 bits 0x3f3504f3 (Rust's `7.0710678e-1` does too).
    // The shorter `7.071068e-1` rounds to 0x3f3504f4 (1 ULP off), which is wrong.
    #[allow(clippy::approx_constant)]
    const HSQT2: f32 = 7.0710678e-1_f32;
    let t0 = l1 * ido;

    // DC butterfly
    // C: t1=t0; t4=t1<<1; t2=t1+(t1<<1); t3=0;
    let mut t1 = t0;
    let mut t4 = t1 << 1;
    let mut t2 = t1 + (t1 << 1);
    let mut t3 = 0usize;
    for _k in 0..l1 {
        let tr1 = cc[t1] + cc[t2];
        let tr2 = cc[t3] + cc[t4];
        // ch[t5=t3<<2]=tr1+tr2  (t5 is a new local computed inline)
        let t5 = t3 << 2;
        ch[t5] = tr1 + tr2;
        ch[(ido << 2) + t5 - 1] = tr2 - tr1;
        // ch[(t5+=(ido<<1))-1] = ...  means t5 becomes t5+(ido<<1), then index t5-1
        let t5b = t5 + (ido << 1);
        ch[t5b - 1] = cc[t3] - cc[t4];
        ch[t5b] = cc[t2] - cc[t1];
        t1 += ido;
        t2 += ido;
        t3 += ido;
        t4 += ido;
    }

    if ido < 2 {
        return;
    }
    if ido != 2 {
        // Twiddle loop
        // C: t1=0; for(k=0;k<l1;k++){ t2=t1; t4=t1<<2; t5=(t6=ido<<1)+t4;
        let mut t1 = 0usize;
        for _k in 0..l1 {
            let mut t2 = t1;
            let mut t4 = t1 << 2;
            let t6 = ido << 1;
            let mut t5 = t6 + t4;
            let mut i = 2usize;
            while i < ido {
                t2 += 2;
                t4 += 2;
                t5 -= 2;
                // t3=(t2+=2); t4+=2; t5-=2; t3+=t0;
                let mut t3 = t2 + t0;
                let cr2 = wa1[i - 2] * cc[t3 - 1] + wa1[i - 1] * cc[t3];
                let ci2 = wa1[i - 2] * cc[t3] - wa1[i - 1] * cc[t3 - 1];
                t3 += t0;
                let cr3 = wa2[i - 2] * cc[t3 - 1] + wa2[i - 1] * cc[t3];
                let ci3 = wa2[i - 2] * cc[t3] - wa2[i - 1] * cc[t3 - 1];
                t3 += t0;
                let cr4 = wa3[i - 2] * cc[t3 - 1] + wa3[i - 1] * cc[t3];
                let ci4 = wa3[i - 2] * cc[t3] - wa3[i - 1] * cc[t3 - 1];

                let tr1 = cr2 + cr4;
                let tr4 = cr4 - cr2;
                let ti1 = ci2 + ci4;
                let ti4 = ci2 - ci4;
                let ti2 = cc[t2] + ci3;
                let ti3 = cc[t2] - ci3;
                let tr2 = cc[t2 - 1] + cr3;
                let tr3 = cc[t2 - 1] - cr3;

                ch[t4 - 1] = tr1 + tr2;
                ch[t4] = ti1 + ti2;
                ch[t5 - 1] = tr3 - ti4;
                ch[t5] = tr4 - ti3;
                ch[t4 + t6 - 1] = ti4 + tr3;
                ch[t4 + t6] = tr4 + ti3;
                ch[t5 + t6 - 1] = tr2 - tr1;
                ch[t5 + t6] = ti1 - ti2;

                i += 2;
            }
            t1 += ido;
        }
        if ido & 1 != 0 {
            return;
        }
    }

    // L105: Nyquist half-sample terms (reached when ido is even, including ido==2)
    // C: t2=(t1=t0+ido-1)+(t0<<1); t3=ido<<2; t4=ido; t5=ido<<1; t6=ido;
    let mut t1 = t0 + ido - 1;
    let mut t2 = t1 + (t0 << 1);
    let t3 = ido << 2;
    let mut t4 = ido;
    let t5 = ido << 1;
    let mut t6 = ido;

    for _k in 0..l1 {
        let ti1 = -HSQT2 * (cc[t1] + cc[t2]);
        let tr1 = HSQT2 * (cc[t1] - cc[t2]);
        ch[t4 - 1] = tr1 + cc[t6 - 1];
        ch[t4 + t5 - 1] = cc[t6 - 1] - tr1;
        ch[t4] = ti1 - cc[t1 + t0];
        ch[t4 + t5] = ti1 + cc[t1 + t0];
        t1 += ido;
        t2 += ido;
        t4 += t3;
        t6 += ido;
    }
}

// ---------------------------------------------------------------------------
// drftf1  (forward real FFT driver)
// Port of smallft.c lines 572-631.
// ---------------------------------------------------------------------------
fn drftf1(n: usize, c: &mut [f32], ch: &mut [f32], wa: &[f32], ifac: &[i32]) {
    let nf = ifac[1] as usize;
    let mut na = 1usize;
    let mut l2 = n;
    let mut iw = n;

    // Debug dump: input to drftf1 (first call only).
    if crate::debug_dump::dump_enabled() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static FIRED: AtomicBool = AtomicBool::new(false);
        if !FIRED.swap(true, Ordering::Relaxed) {
            let mut bytes = Vec::with_capacity(n * 4);
            for v in c.iter() {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            let _ = std::fs::write("/tmp/lewtoff-debug/r_drftf1_in.bin", &bytes);

            let mut wabytes = Vec::with_capacity(2 * n * 4);
            for v in wa.iter().take(2 * n) {
                wabytes.extend_from_slice(&v.to_le_bytes());
            }
            let _ = std::fs::write("/tmp/lewtoff-debug/r_drftf1_wa.bin", &wabytes);
        }
    }

    for k1 in 0..nf {
        let kh = nf - k1;
        let ip = ifac[kh + 1] as usize;
        let l1 = l2 / ip;
        let ido = n / l2;
        iw -= (ip - 1) * ido;
        na = 1 - na;

        if ip == 4 {
            let ix2 = iw + ido;
            let ix3 = ix2 + ido;
            if na != 0 {
                dradf4(
                    ido,
                    l1,
                    ch,
                    c,
                    &wa[iw - 1..],
                    &wa[ix2 - 1..],
                    &wa[ix3 - 1..],
                );
            } else {
                dradf4(
                    ido,
                    l1,
                    c,
                    ch,
                    &wa[iw - 1..],
                    &wa[ix2 - 1..],
                    &wa[ix3 - 1..],
                );
            }
            if crate::debug_dump::dump_enabled() {
                use std::sync::atomic::{AtomicUsize, Ordering};
                static ITER: AtomicUsize = AtomicUsize::new(0);
                let iter = ITER.fetch_add(1, Ordering::Relaxed);
                if iter < 4 {
                    let buf = if na != 0 { &c[..] } else { &ch[..] };
                    let mut bytes = Vec::with_capacity(buf.len() * 4);
                    for v in buf {
                        bytes.extend_from_slice(&v.to_le_bytes());
                    }
                    let _ = std::fs::write(
                        format!("/tmp/lewtoff-debug/r_drftf1_iter{iter}_out.bin"),
                        &bytes,
                    );
                }
            }
        } else if ip == 2 {
            if na != 0 {
                dradf2(ido, l1, ch, c, &wa[iw - 1..]);
            } else {
                dradf2(ido, l1, c, ch, &wa[iw - 1..]);
            }
        } else {
            // Generic radix -- unreachable for n=256 or n=2048 (only radix 2,4 factors)
            panic!("drft: generic radix ip={ip} not supported for this FFT size");
        }

        l2 = l1;
    }

    if na == 1 {
        return;
    }
    c.copy_from_slice(ch);
}

// ---------------------------------------------------------------------------
// Public API: in-place real forward FFT matching libvorbis drft_forward.
// ---------------------------------------------------------------------------

pub(crate) fn drft_forward_long(data: &mut [f32; LONG_BLOCK]) {
    let n = LONG_BLOCK;
    let mut ch = vec![0.0f32; n];
    // trigcache layout: [0..n unused zeros][n..3n = wa used by drftf1]
    // drft_forward passes trigcache+n as wa, which is DRFT_TRIG_2048[n..]
    let wa = &DRFT_TRIG_2048[n..];
    drftf1(n, data.as_mut_slice(), &mut ch, wa, &DRFT_SPLIT_2048);
}

pub(crate) fn drft_forward_short(data: &mut [f32; SHORT_BLOCK]) {
    let n = SHORT_BLOCK;
    let mut ch = vec![0.0f32; n];
    let wa = &DRFT_TRIG_256[n..];
    drftf1(n, data.as_mut_slice(), &mut ch, wa, &DRFT_SPLIT_256);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drft_short_dc_only() {
        // Input: constant 1.0 → DC bin = N, all other bins = 0
        let n = SHORT_BLOCK;
        let mut data = [1.0f32; SHORT_BLOCK];
        drft_forward_short(&mut data);
        // DC bin
        let dc = data[0];
        assert!(
            (dc - n as f32).abs() < 0.1,
            "DC bin expected {}, got {}",
            n,
            dc
        );
        // All other bins should be ~0
        for i in 1..n {
            assert!(
                data[i].abs() < 0.01,
                "bin {} expected ~0, got {}",
                i,
                data[i]
            );
        }
    }

    #[test]
    fn drft_long_dc_only() {
        let n = LONG_BLOCK;
        let mut data = [1.0f32; LONG_BLOCK];
        drft_forward_long(&mut data);
        let dc = data[0];
        assert!(
            (dc - n as f32).abs() < 0.1,
            "DC bin expected {}, got {}",
            n,
            dc
        );
        for i in 1..n {
            assert!(
                data[i].abs() < 0.01,
                "bin {} expected ~0, got {}",
                i,
                data[i]
            );
        }
    }
}

#[test]
fn drft_sine_peak() {
    use std::f32::consts::PI;
    let n = SHORT_BLOCK; // 256
    let mut data = [0.0f32; SHORT_BLOCK];
    // Sine at bin 10 (freq = 10/256 cycles/sample)
    for i in 0..n {
        data[i] = (2.0 * PI * 10.0 * i as f32 / n as f32).sin();
    }
    drft_forward_short(&mut data);

    // Find peak in Fortran-packed output
    let mut max_mag = 0.0f32;
    let mut max_bin = 0usize;
    for b in 0..n / 2 {
        let mag = if b == 0 {
            data[0].abs()
        } else {
            (data[2 * b - 1].powi(2) + data[2 * b].powi(2)).sqrt()
        };
        if mag > max_mag {
            max_mag = mag;
            max_bin = b;
        }
    }
    assert_eq!(max_bin, 10, "peak should be at bin 10, got {max_bin}");
    assert!(
        (max_mag - n as f32 / 2.0).abs() < 1.0,
        "peak magnitude expected ~{}, got {max_mag}",
        n / 2
    );
}

#[test]
fn drft_logfft_sine_440() {
    use crate::psy::to_db;
    use std::f32::consts::PI;

    let rate = 44100u32;
    let freq = 440.0f32;
    let n = SHORT_BLOCK; // 256
    let _n2 = n / 2;

    // Build 256 samples of sine at 440Hz (matching lewtoff test amplitude)
    // In the test: (sin(2pi*440*t) * 16384.0) as i16, normalized to f32 by /32768.0
    // Actually lewtoff normalizes: pcm[i] = sample as f32 / 32768.0
    let mut pcm = [0.0f32; SHORT_BLOCK];
    for i in 0..n {
        let t = i as f32 / rate as f32;
        let sample = (2.0 * PI * freq * t).sin() * 16384.0;
        pcm[i] = sample / 32768.0;
    }

    // Apply a basic symmetric window (approximation)
    // Actually this is before windowing. Let's skip window for now.

    drft_forward_short(&mut pcm);

    let scale = 4.0f32 / n as f32;
    let scale_db = to_db(scale) + 0.345_f32;

    // Compute logfft
    let mut logfft = [0.0f32; 128]; // n2
    let lam = scale_db + to_db(pcm[0]) + 0.345_f32;
    logfft[0] = lam;
    let mut local_ampmax = lam;
    let mut j = 1usize;
    while j < n - 1 {
        let temp = pcm[j] * pcm[j] + pcm[j + 1] * pcm[j + 1];
        let t = scale_db + 0.5_f32 * to_db(temp) + 0.345_f32;
        logfft[(j + 1) >> 1] = t;
        if t > local_ampmax {
            local_ampmax = t;
        }
        j += 2;
    }

    // Find peak bin
    let (peak_bin, peak_val) = logfft[1..]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(i, v)| (i + 1, *v))
        .unwrap();

    // Expected bin for 440Hz at 44100Hz, n=256
    // bin = round(440 * 256 / 44100) = round(2.55) ≈ 2 or 3
    eprintln!(
        "Peak at logfft[{}] = {:.2} dB (local_ampmax={:.2})",
        peak_bin, peak_val, local_ampmax
    );
    eprintln!("logfft[1..5] = {:?}", &logfft[1..5]);

    // Peak should be near 0 dB (we computed ~0.69 dB earlier)
    assert!(
        peak_val > -20.0,
        "peak logfft expected > -20dB, got {peak_val}"
    );
    assert!(peak_val < 5.0, "peak logfft expected < 5dB, got {peak_val}");
}
