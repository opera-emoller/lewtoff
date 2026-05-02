use lewtoff::bitpack::BitWriter;
use lewtoff::headers::{write_comment_header, write_id_header, write_setup_header};
use lewtoff::{Channels, SampleRate};

fn make_writer() -> BitWriter {
    BitWriter::new()
}

#[test]
fn id_header_mono_44100_matches_ffmpeg() {
    let mut w = make_writer();
    write_id_header(SampleRate::Hz44100, Channels::Mono, &mut w);
    let actual = w.into_bytes();
    let expected = include_bytes!("vectors/headers/id_mono44.bin");
    assert_eq!(
        actual.as_slice(),
        expected.as_slice(),
        "id header mono44 mismatch"
    );
}

#[test]
fn id_header_mono_48000_matches_ffmpeg() {
    let mut w = make_writer();
    write_id_header(SampleRate::Hz48000, Channels::Mono, &mut w);
    let actual = w.into_bytes();
    let expected = include_bytes!("vectors/headers/id_mono48.bin");
    assert_eq!(
        actual.as_slice(),
        expected.as_slice(),
        "id header mono48 mismatch"
    );
}

#[test]
fn id_header_stereo_44100_matches_ffmpeg() {
    let mut w = make_writer();
    write_id_header(SampleRate::Hz44100, Channels::Stereo, &mut w);
    let actual = w.into_bytes();
    let expected = include_bytes!("vectors/headers/id_stereo44.bin");
    assert_eq!(
        actual.as_slice(),
        expected.as_slice(),
        "id header stereo44 mismatch"
    );
}

#[test]
fn id_header_stereo_48000_matches_ffmpeg() {
    let mut w = make_writer();
    write_id_header(SampleRate::Hz48000, Channels::Stereo, &mut w);
    let actual = w.into_bytes();
    let expected = include_bytes!("vectors/headers/id_stereo48.bin");
    assert_eq!(
        actual.as_slice(),
        expected.as_slice(),
        "id header stereo48 mismatch"
    );
}

#[test]
fn comment_header_mono_44100_matches_ffmpeg() {
    let mut w = make_writer();
    write_comment_header(&mut w);
    let actual = w.into_bytes();
    let expected = include_bytes!("vectors/headers/comment_mono44.bin");
    assert_eq!(
        actual.as_slice(),
        expected.as_slice(),
        "comment header mono44 mismatch"
    );
}

#[test]
fn comment_header_mono_48000_matches_ffmpeg() {
    let mut w = make_writer();
    write_comment_header(&mut w);
    let actual = w.into_bytes();
    let expected = include_bytes!("vectors/headers/comment_mono48.bin");
    assert_eq!(
        actual.as_slice(),
        expected.as_slice(),
        "comment header mono48 mismatch"
    );
}

#[test]
fn comment_header_stereo_44100_matches_ffmpeg() {
    let mut w = make_writer();
    write_comment_header(&mut w);
    let actual = w.into_bytes();
    let expected = include_bytes!("vectors/headers/comment_stereo44.bin");
    assert_eq!(
        actual.as_slice(),
        expected.as_slice(),
        "comment header stereo44 mismatch"
    );
}

#[test]
fn comment_header_stereo_48000_matches_ffmpeg() {
    let mut w = make_writer();
    write_comment_header(&mut w);
    let actual = w.into_bytes();
    let expected = include_bytes!("vectors/headers/comment_stereo48.bin");
    assert_eq!(
        actual.as_slice(),
        expected.as_slice(),
        "comment header stereo48 mismatch"
    );
}

#[test]
fn setup_header_matches_ffmpeg() {
    let mut w = make_writer();
    write_setup_header(SampleRate::Hz44100, Channels::Mono, &mut w);
    let actual = w.into_bytes();
    let expected = include_bytes!("vectors/headers/setup.bin");
    assert_eq!(
        actual.as_slice(),
        expected.as_slice(),
        "setup header mismatch"
    );
}
