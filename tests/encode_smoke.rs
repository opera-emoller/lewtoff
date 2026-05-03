use lewtoff::{Channels, SampleRate};

fn silence(n: usize) -> Vec<i16> {
    vec![0i16; n]
}

fn sine_stereo(n_samples: usize, rate: u32) -> Vec<i16> {
    let mut out = Vec::with_capacity(n_samples * 2);
    for i in 0..n_samples {
        let t = i as f32 / rate as f32;
        let v = (2.0 * std::f32::consts::PI * 440.0 * t).sin();
        let s = (v * 16000.0) as i16;
        out.push(s);
        out.push(s);
    }
    out
}

fn check_ogg(bytes: &[u8]) {
    assert!(
        bytes.len() > 1000,
        "output too short: {} bytes",
        bytes.len()
    );
    assert_eq!(&bytes[0..4], b"OggS", "missing OggS magic");
}

#[test]
fn encode_silence_mono44_does_not_panic() {
    let bytes = lewtoff::encode(&silence(44100), SampleRate::Hz44100, Channels::Mono);
    check_ogg(&bytes);
}

#[test]
fn encode_silence_mono48_does_not_panic() {
    let bytes = lewtoff::encode(&silence(48000), SampleRate::Hz48000, Channels::Mono);
    check_ogg(&bytes);
}

#[test]
fn encode_silence_stereo44_does_not_panic() {
    let bytes = lewtoff::encode(&silence(44100 * 2), SampleRate::Hz44100, Channels::Stereo);
    check_ogg(&bytes);
}

#[test]
fn encode_silence_stereo48_does_not_panic() {
    let bytes = lewtoff::encode(&silence(48000 * 2), SampleRate::Hz48000, Channels::Stereo);
    check_ogg(&bytes);
}

#[test]
fn encode_sine_stereo44_does_not_panic() {
    let samples = sine_stereo(44100, 44100);
    let bytes = lewtoff::encode(&samples, SampleRate::Hz44100, Channels::Stereo);
    check_ogg(&bytes);
}

#[test]
fn lewton_can_decode_our_silence_mono44() {
    let bytes = lewtoff::encode(&silence(44100), SampleRate::Hz44100, Channels::Mono);
    let cursor = std::io::Cursor::new(bytes);
    let mut reader =
        lewton::inside_ogg::OggStreamReader::new(cursor).expect("lewton failed to open stream");
    let mut max_abs = 0i16;
    loop {
        match reader.read_dec_packet_itl() {
            Ok(Some(pkt)) => {
                for s in pkt {
                    if s.abs() > max_abs {
                        max_abs = s.abs();
                    }
                }
            }
            Ok(None) => break,
            Err(e) => panic!("lewton decode error: {:?}", e),
        }
    }
    assert!(
        max_abs < 1000,
        "silence decoded with unexpected amplitude: {}",
        max_abs
    );
}

#[test]
fn lewton_can_decode_our_silence_stereo44() {
    let bytes = lewtoff::encode(&silence(44100 * 2), SampleRate::Hz44100, Channels::Stereo);
    let cursor = std::io::Cursor::new(bytes);
    let mut reader =
        lewton::inside_ogg::OggStreamReader::new(cursor).expect("lewton failed to open stream");
    loop {
        match reader.read_dec_packet_itl() {
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(e) => panic!("lewton decode error: {:?}", e),
        }
    }
}
