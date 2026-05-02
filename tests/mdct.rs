use lewtoff::mdct::mdct_forward;

fn run_case(input_bytes: &[u8], expected_output_bytes: &[u8]) {
    assert_eq!(input_bytes.len(), 2048 * 4);
    assert_eq!(expected_output_bytes.len(), 1024 * 4);

    let mut input = [0f32; 2048];
    for (i, chunk) in input_bytes.chunks_exact(4).enumerate() {
        input[i] = f32::from_le_bytes(chunk.try_into().unwrap());
    }

    let mut output = [0f32; 1024];
    mdct_forward(&input, &mut output);

    let mut actual_bytes = Vec::with_capacity(1024 * 4);
    for v in &output {
        actual_bytes.extend_from_slice(&v.to_le_bytes());
    }

    assert_eq!(
        actual_bytes, expected_output_bytes,
        "MDCT output bytes diverged"
    );
}

#[test]
fn mdct_silence() {
    run_case(
        include_bytes!("vectors/mdct/input_silence.bin"),
        include_bytes!("vectors/mdct/output_silence.bin"),
    );
}

#[test]
fn mdct_dc() {
    run_case(
        include_bytes!("vectors/mdct/input_dc.bin"),
        include_bytes!("vectors/mdct/output_dc.bin"),
    );
}
#[test]
fn mdct_impulse() {
    run_case(
        include_bytes!("vectors/mdct/input_impulse.bin"),
        include_bytes!("vectors/mdct/output_impulse.bin"),
    );
}
#[test]
fn mdct_ramp() {
    run_case(
        include_bytes!("vectors/mdct/input_ramp.bin"),
        include_bytes!("vectors/mdct/output_ramp.bin"),
    );
}
#[test]
fn mdct_sine() {
    run_case(
        include_bytes!("vectors/mdct/input_sine_440hz_44100.bin"),
        include_bytes!("vectors/mdct/output_sine_440hz_44100.bin"),
    );
}
#[test]
fn mdct_negative_impulse() {
    run_case(
        include_bytes!("vectors/mdct/input_negative_impulse.bin"),
        include_bytes!("vectors/mdct/output_negative_impulse.bin"),
    );
}
