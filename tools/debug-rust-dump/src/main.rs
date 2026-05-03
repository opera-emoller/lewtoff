//! debug-rust-dump: trigger LEWTOFF_DEBUG_DUMP=1 on a 440 Hz sine input
//! matching the C harness (1 second, 44100 Hz mono, peak amplitude 32767).
//!
//! Run with: LEWTOFF_DEBUG_DUMP=1 cargo run --bin debug-rust-dump
//! Dumps land in /tmp/lewtoff-debug/r_*.

fn main() {
    std::env::set_var("LEWTOFF_DEBUG_DUMP", "1");

    let rate_hz = 44100usize;
    let duration_secs = 1usize;
    let n_samples = rate_hz * duration_secs;
    let freq = 440.0f64;
    let amplitude = 32767.0f64;

    let samples: Vec<i16> = (0..n_samples)
        .map(|i| {
            let t = i as f64 / rate_hz as f64;
            let v = amplitude * (2.0 * std::f64::consts::PI * freq * t).sin();
            v.round() as i16
        })
        .collect();

    lewtoff::encode_with_serial(
        &samples,
        lewtoff::SampleRate::Hz44100,
        lewtoff::Channels::Mono,
        0,
    );

    println!("Dumps written to /tmp/lewtoff-debug/r_*");
}
