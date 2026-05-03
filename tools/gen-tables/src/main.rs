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

fn main() {
    // Write the Rust source file for trig tables
    let mut out = String::new();
    out.push_str("// AUTO-GENERATED by tools/gen-tables. Do not edit. Regenerate via 'just regen-trig-table'.\n");
    out.push_str("#![allow(clippy::excessive_precision)]\n\n");

    let (trig_2048, bitrev_2048, scale_2048) = gen_trig(2048);
    write_trig_table(&mut out, "2048", &trig_2048, &bitrev_2048, scale_2048);

    let (trig_256, bitrev_256, scale_256) = gen_trig(256);
    write_trig_table(&mut out, "256", &trig_256, &bitrev_256, scale_256);

    std::fs::write("src/tables/trig.rs", &out).expect("failed to write src/tables/trig.rs");
    eprintln!("Wrote src/tables/trig.rs");

    // Generate window tables
    let mut win_out = String::new();
    win_out.push_str(
        "// AUTO-GENERATED by tools/gen-tables. Do not edit. Regenerate via 'just regen-tables'.\n",
    );
    win_out.push_str("#![allow(clippy::excessive_precision)]\n\n");

    let window_2048 = gen_window(2048);
    write_window_table(&mut win_out, "2048", &window_2048);

    let window_256 = gen_window(256);
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
    let n_short = 256usize;
    let win_half_256: Vec<f32> = (0..n_short / 2)
        .map(|i| {
            let s = (PI * (i as f64 + 0.5) / n_short as f64).sin();
            (0.5 * PI * s * s).sin() as f32
        })
        .collect();
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
    let win_half_2048: Vec<f32> = (0..n_long / 2)
        .map(|i| {
            let s = (PI * (i as f64 + 0.5) / n_long as f64).sin();
            (0.5 * PI * s * s).sin() as f32
        })
        .collect();
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
}
