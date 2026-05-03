// Generates src/tables/trig.rs with the trig/bitrev/scale tables that
// libvorbis's mdct_init would populate for n=2048 and n=256.

use std::f64::consts::PI;

/// Format an f32 as a Rust literal that round-trips exactly.
/// We print enough decimal digits (9 significant digits is enough for f32).
fn fmt_f32(v: f32) -> String {
    if v.is_nan() {
        return "f32::NAN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 {
            "f32::INFINITY".to_string()
        } else {
            "f32::NEG_INFINITY".to_string()
        };
    }
    // Use enough decimal digits that the value round-trips exactly.
    // 9 significant decimal digits are always sufficient for f32.
    let s = format!("{:.9e}", v);
    // Parse back to verify round-trip
    let reparsed: f32 = s.parse().unwrap();
    assert_eq!(
        v.to_bits(),
        reparsed.to_bits(),
        "round-trip failed for {:?}",
        v
    );
    // Append f32 suffix
    format!("{}_f32", s)
}

fn gen_trig(n: usize) -> (Vec<f32>, Vec<u32>, f32) {
    let n2 = n / 2;
    let n4 = n / 4;
    let n8 = n / 8;

    let trig_size = n + n4;
    let mut trig = vec![0f32; trig_size];

    for i in 0..n4 {
        trig[i * 2] = (PI / n as f64 * (4 * i) as f64).cos() as f32;
        trig[i * 2 + 1] = -(PI / n as f64 * (4 * i) as f64).sin() as f32;
        trig[n2 + i * 2] = (PI / (2 * n) as f64 * (2 * i + 1) as f64).cos() as f32;
        trig[n2 + i * 2 + 1] = (PI / (2 * n) as f64 * (2 * i + 1) as f64).sin() as f32;
    }
    for i in 0..n8 {
        trig[n + i * 2] = ((PI / n as f64 * (4 * i + 2) as f64).cos() * 0.5) as f32;
        trig[n + i * 2 + 1] = -((PI / n as f64 * (4 * i + 2) as f64).sin() * 0.5) as f32;
    }

    let log2n: u32 = (n as f64).log2().round() as u32;
    let bitrev_size = n4;
    let mut bitrev = vec![0u32; bitrev_size];
    {
        let mask: u32 = (1 << (log2n - 1)) - 1;
        let msb: u32 = 1 << (log2n - 2);
        for i in 0..n8 {
            let mut acc: u32 = 0;
            let mut j = 0u32;
            while (msb >> j) != 0 {
                if ((msb >> j) & i as u32) != 0 {
                    acc |= 1 << j;
                }
                j += 1;
            }
            bitrev[i * 2] = ((!acc) & mask) - 1;
            bitrev[i * 2 + 1] = acc;
        }
    }

    let scale: f32 = 4.0f32 / n as f32;
    (trig, bitrev, scale)
}

fn write_trig_table(out: &mut String, suffix: &str, trig: &[f32], bitrev: &[u32], scale: f32) {
    out.push_str(&format!(
        "pub static TRIG_{}: [f32; {}] = [\n",
        suffix,
        trig.len()
    ));
    for (idx, &v) in trig.iter().enumerate() {
        if idx % 4 == 0 {
            out.push_str("    ");
        }
        out.push_str(&fmt_f32(v));
        if idx + 1 < trig.len() {
            out.push(',');
            if idx % 4 == 3 {
                out.push('\n');
            } else {
                out.push(' ');
            }
        }
    }
    out.push_str("\n];\n\n");

    out.push_str(&format!(
        "pub static BITREV_{}: [u32; {}] = [\n",
        suffix,
        bitrev.len()
    ));
    for (idx, &v) in bitrev.iter().enumerate() {
        if idx % 8 == 0 {
            out.push_str("    ");
        }
        out.push_str(&format!("{}", v));
        if idx + 1 < bitrev.len() {
            out.push(',');
            if idx % 8 == 7 {
                out.push('\n');
            } else {
                out.push(' ');
            }
        }
    }
    out.push_str("\n];\n\n");

    out.push_str(&format!(
        "pub static SCALE_{}: f32 = {};\n\n",
        suffix,
        fmt_f32(scale)
    ));
}

fn gen_window(n: usize) -> Vec<f32> {
    let mut window = vec![0f32; n];
    for i in 0..n {
        let s = (PI * (i as f64 + 0.5) / n as f64).sin();
        let w = (0.5 * PI * s * s).sin() as f32;
        window[i] = w;
    }

    // Verify COLA property: w[i]^2 + w[i + N/2]^2 = 1 for i in 0..N/2
    let mut max_err: f64 = 0.0;
    for i in 0..n / 2 {
        let a = window[i] as f64;
        let b = window[i + n / 2] as f64;
        let err = (a * a + b * b - 1.0).abs();
        if err > max_err {
            max_err = err;
        }
    }
    assert!(
        max_err < 1e-5,
        "window COLA property violated for n={n}: max error = {max_err}"
    );
    eprintln!("Window n={n} COLA max error = {max_err:.2e}");
    window
}

fn write_window_table(win_out: &mut String, suffix: &str, window: &[f32]) {
    win_out.push_str(&format!(
        "pub static SIN_WINDOW_{}: [f32; {}] = [\n",
        suffix,
        window.len()
    ));
    for (idx, &v) in window.iter().enumerate() {
        if idx % 4 == 0 {
            win_out.push_str("    ");
        }
        win_out.push_str(&fmt_f32(v));
        if idx + 1 < window.len() {
            win_out.push(',');
            if idx % 4 == 3 {
                win_out.push('\n');
            } else {
                win_out.push(' ');
            }
        }
    }
    win_out.push_str("\n];\n\n");
}

/// Port of drfti1 from smallft.c.
/// Fills wa[0..n] (the trig cache) and ifac[0..32] (the split cache).
/// `drft_init` calls `fdrffti(n, trigcache, splitcache)` which calls `drfti1(n, wa+n, ifac)`.
/// So wa here corresponds to `trigcache + n` in the C — i.e. the second half of trigcache.
fn drfti1(n: usize, wa: &mut Vec<f32>, ifac: &mut Vec<i32>) {
    const NTRYH: [i32; 4] = [4, 2, 3, 5];
    const TPI: f64 = 6.283_185_307_179_586_48;

    let mut ntry: i32 = 0;
    let mut j: i32 = -1;
    let mut nl = n as i32;
    let mut nf: i32 = 0;

    // Factorization loop
    loop {
        j += 1;
        if j < 4 {
            ntry = NTRYH[j as usize];
        } else {
            ntry += 2;
        }
        loop {
            let nq = nl / ntry;
            let nr = nl - ntry * nq;
            if nr != 0 {
                break; // goto L101
            }
            nf += 1;
            ifac[(nf + 1) as usize] = ntry;
            nl = nq;
            if ntry != 2 {
                // goto L107
            } else if nf != 1 {
                for i in 1..nf {
                    let ib = nf - i + 1;
                    ifac[(ib + 1) as usize] = ifac[ib as usize];
                }
                ifac[2] = 2;
            }
            if nl != 1 {
                continue; // goto L104
            }
            break;
        }
        if nl == 1 {
            break;
        }
    }

    ifac[0] = n as i32;
    ifac[1] = nf;
    // libvorbis smallft.c does this math in float (f32). We mirror that exactly
    // so the precomputed trigcache is byte-identical to what drft_init produces
    // at runtime in libvorbis. f64-then-cast diverges by ULPs.
    let argh: f32 = (TPI / n as f64) as f32;
    let mut is: usize = 0;
    let nfm1 = nf - 1;
    let mut l1: i32 = 1;

    if nfm1 == 0 {
        return;
    }

    for k1 in 0..nfm1 {
        let ip = ifac[(k1 + 2) as usize];
        let mut ld: i32 = 0;
        let l2 = l1 * ip;
        let ido = n as i32 / l2;
        let ipm = ip - 1;

        for _j in 0..ipm {
            ld += l1;
            let mut i = is;
            let argld: f32 = (ld as f32) * argh;
            let mut fi: f32 = 0.0;
            let mut ii = 2;
            while ii < ido {
                fi += 1.0;
                let arg: f32 = fi * argld;
                wa[i] = arg.cos();
                wa[i + 1] = arg.sin();
                i += 2;
                ii += 2;
            }
            is += ido as usize;
        }
        l1 = l2;
    }
}

/// Returns (trigcache: Vec<f32>, splitcache: Vec<i32>)
/// trigcache has length 3*n (but drfti1 only fills wa = trigcache+n, i.e. indices n..3n)
/// splitcache has length 32
fn gen_drft_tables(n: usize) -> (Vec<f32>, Vec<i32>) {
    let mut trigcache = vec![0.0f32; 3 * n];
    let mut splitcache = vec![0i32; 32];
    if n == 1 {
        return (trigcache, splitcache);
    }
    // drfti1(n, wa+n, ifac) — wa here is the slice starting at n
    let mut wa_slice = vec![0.0f32; 2 * n]; // will hold indices [n..3n] of trigcache
    drfti1(n, &mut wa_slice, &mut splitcache);
    trigcache[n..3 * n].copy_from_slice(&wa_slice);
    (trigcache, splitcache)
}

fn write_drft_table(out: &mut String, suffix: &str, trigcache: &[f32], splitcache: &[i32]) {
    out.push_str(&format!(
        "pub static DRFT_TRIG_{}: [f32; {}] = [\n",
        suffix,
        trigcache.len()
    ));
    for (idx, &v) in trigcache.iter().enumerate() {
        if idx % 4 == 0 {
            out.push_str("    ");
        }
        out.push_str(&fmt_f32(v));
        if idx + 1 < trigcache.len() {
            out.push(',');
            if idx % 4 == 3 {
                out.push('\n');
            } else {
                out.push(' ');
            }
        }
    }
    out.push_str("\n];\n\n");

    out.push_str(&format!(
        "pub static DRFT_SPLIT_{}: [i32; 32] = [\n",
        suffix
    ));
    for (idx, &v) in splitcache.iter().enumerate() {
        if idx % 8 == 0 {
            out.push_str("    ");
        }
        out.push_str(&format!("{}", v));
        if idx + 1 < splitcache.len() {
            out.push(',');
            if idx % 8 == 7 {
                out.push('\n');
            } else {
                out.push(' ');
            }
        }
    }
    out.push_str("\n];\n\n");
}

fn main() {
    // Write the Rust source file for trig tables
    let mut out = String::new();
    out.push_str("// AUTO-GENERATED by tools/gen-tables. Do not edit. Regenerate via 'just regen-trig-table'.\n");
    out.push_str("#![allow(clippy::excessive_precision)]\n\n");

    let (trig_2048, bitrev_2048, scale_2048) = gen_trig(2048);
    write_trig_table(&mut out, "2048", &trig_2048, &bitrev_2048, scale_2048);

    let (trig_256, bitrev_256, scale_256) = gen_trig(256);
    write_trig_table(&mut out, "256", &trig_256, &bitrev_256, scale_256);

    // Envelope detection uses MDCT of size 128 (winlength=128).
    let (trig_128, bitrev_128, scale_128) = gen_trig(128);
    write_trig_table(&mut out, "128", &trig_128, &bitrev_128, scale_128);

    std::fs::write("src/tables/trig.rs", &out).expect("failed to write src/tables/trig.rs");
    eprintln!("Wrote src/tables/trig.rs");

    // Generate window tables
    let mut win_out = String::new();
    win_out.push_str(
        "// AUTO-GENERATED by tools/gen-tables. Do not edit. Regenerate via 'just regen-tables'.\n",
    );
    win_out.push_str("#![allow(clippy::excessive_precision)]\n\n");

    // Load libvorbis's exact window values from /tmp/c_vwin{256,2048}.bin
    // (run tools/oracle-encoder/dump_window_tables to regenerate). These
    // come from the f32 literals in libvorbis's window.c which differ
    // by hundreds of ULPs from formula-regenerated values at the window
    // edge — critical for ramp/DC-heavy MDCT bin 0.
    fn load_f32_bin(path: &str, count: usize) -> Vec<f32> {
        let bytes = std::fs::read(path).unwrap_or_else(|_| {
            panic!("missing {path}; run tools/oracle-encoder/dump_window_tables first");
        });
        assert_eq!(bytes.len(), count * 4, "{path} size mismatch");
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    // libvorbis vwin{N} stores N/2 floats — the rising half of the symmetric
    // window. Reconstruct the full N-element SIN_WINDOW_* from it by
    // mirroring: SIN_WINDOW[N-1-i] = SIN_WINDOW[i] = vwin[i] for i in 0..N/2.
    fn full_from_half(half: &[f32], n: usize) -> Vec<f32> {
        assert_eq!(half.len(), n / 2);
        let mut full = vec![0.0f32; n];
        for i in 0..n / 2 {
            full[i] = half[i];
            full[n - 1 - i] = half[i];
        }
        full
    }

    let half_2048 = load_f32_bin("/tmp/c_vwin2048.bin", 1024);
    let window_2048 = full_from_half(&half_2048, 2048);
    write_window_table(&mut win_out, "2048", &window_2048);

    let half_256 = load_f32_bin("/tmp/c_vwin256.bin", 128);
    let window_256 = full_from_half(&half_256, 256);
    write_window_table(&mut win_out, "256", &window_256);

    // Also generate the half-window for n=256 short block transitions.
    // libvorbis uses vwin128 (128 values) for the transition window when blocksizes[0]=256.
    // vwin[winno[0]] where winno[0] = ilog(256)-7 = 1, so vwin[1] = vwin128.
    // This is a 128-point window half: window values at positions 0..128
    // computed for a 256-point full window.
    // BUT libvorbis hardcodes the vwin tables. We use the same formula
    // as for the full window to generate our own 128-value half-window for n=256 short blocks.
    // For apply_window, we need the windowLW / windowNW half-windows which have n/2 values.
    // vwin128 is used for blocksizes[0]=256. It has 128 values = 256/2.
    // The formula: vwin128[i] = sin(pi/2 * sin^2(pi * (i+0.5) / 256))
    // We generate WIN_HALF_256 (128 values) for this purpose.
    // WIN_HALF_256 = first half of libvorbis vwin256 (= the rising half).
    // Apply matches libvorbis's d[i] *= windowLW[p] for left-half application.
    let n_short = 256usize;
    let win_half_256: Vec<f32> = window_256[..n_short / 2].to_vec();
    win_out.push_str(&format!(
        "pub static WIN_HALF_256: [f32; {}] = [\n",
        win_half_256.len()
    ));
    for (idx, &v) in win_half_256.iter().enumerate() {
        if idx % 4 == 0 {
            win_out.push_str("    ");
        }
        win_out.push_str(&fmt_f32(v));
        if idx + 1 < win_half_256.len() {
            win_out.push(',');
            if idx % 4 == 3 {
                win_out.push('\n');
            } else {
                win_out.push(' ');
            }
        }
    }
    win_out.push_str("\n];\n\n");

    // Also generate WIN_HALF_2048 (1024 values) for the long block transition window.
    let n_long = 2048usize;
    let win_half_2048: Vec<f32> = window_2048[..n_long / 2].to_vec();
    win_out.push_str(&format!(
        "pub static WIN_HALF_2048: [f32; {}] = [\n",
        win_half_2048.len()
    ));
    for (idx, &v) in win_half_2048.iter().enumerate() {
        if idx % 4 == 0 {
            win_out.push_str("    ");
        }
        win_out.push_str(&fmt_f32(v));
        if idx + 1 < win_half_2048.len() {
            win_out.push(',');
            if idx % 4 == 3 {
                win_out.push('\n');
            } else {
                win_out.push(' ');
            }
        }
    }
    win_out.push_str("\n];\n");

    std::fs::write("src/tables/window.rs", win_out).expect("failed to write src/tables/window.rs");
    eprintln!("Wrote src/tables/window.rs");

    // Generate DRFT tables
    let mut drft_out = String::new();
    drft_out.push_str("// AUTO-GENERATED by tools/gen-tables. Do not edit.\n");
    drft_out.push_str("#![allow(clippy::excessive_precision)]\n\n");

    let (trig2048, split2048) = gen_drft_tables(2048);
    write_drft_table(&mut drft_out, "2048", &trig2048, &split2048);

    let (trig256, split256) = gen_drft_tables(256);
    write_drft_table(&mut drft_out, "256", &trig256, &split256);

    std::fs::write("src/tables/drft.rs", drft_out).expect("failed to write src/tables/drft.rs");
    eprintln!("Wrote src/tables/drft.rs");
}
