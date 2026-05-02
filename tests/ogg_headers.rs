use lewtoff::bitpack::BitWriter;
use lewtoff::headers::{write_comment_header, write_id_header, write_setup_header};
use lewtoff::ogg_pages::OggStreamWriter;
use lewtoff::{Channels, SampleRate};

fn write_header_pages(rate: SampleRate, channels: Channels, serial: u32) -> Vec<u8> {
    let mut w = OggStreamWriter::new(serial);

    let mut bw = BitWriter::new();
    write_id_header(rate, channels, &mut bw);
    w.write_packet(&bw.into_bytes(), 0, false, true);

    let mut bw = BitWriter::new();
    write_comment_header(&mut bw);
    w.write_packet(&bw.into_bytes(), 0, false, false);

    let mut bw = BitWriter::new();
    write_setup_header(rate, channels, &mut bw);
    w.write_packet(&bw.into_bytes(), 0, false, true);

    w.into_bytes()
}

#[test]
fn ogg_headers_mono44_match_ffmpeg() {
    let serial = u32::from_le_bytes(*include_bytes!("vectors/ogg/serial_mono44.bin"));
    let actual = write_header_pages(SampleRate::Hz44100, Channels::Mono, serial);
    let expected = include_bytes!("vectors/ogg/headers_mono44.ogg");
    assert_eq!(
        actual.as_slice(),
        expected.as_slice(),
        "ogg header bytes diverged for mono44"
    );
}

#[test]
fn ogg_headers_mono48_match_ffmpeg() {
    let serial = u32::from_le_bytes(*include_bytes!("vectors/ogg/serial_mono48.bin"));
    let actual = write_header_pages(SampleRate::Hz48000, Channels::Mono, serial);
    let expected = include_bytes!("vectors/ogg/headers_mono48.ogg");
    assert_eq!(
        actual.as_slice(),
        expected.as_slice(),
        "ogg header bytes diverged for mono48"
    );
}

#[test]
fn ogg_headers_stereo44_match_ffmpeg() {
    let serial = u32::from_le_bytes(*include_bytes!("vectors/ogg/serial_stereo44.bin"));
    let actual = write_header_pages(SampleRate::Hz44100, Channels::Stereo, serial);
    let expected = include_bytes!("vectors/ogg/headers_stereo44.ogg");
    assert_eq!(
        actual.as_slice(),
        expected.as_slice(),
        "ogg header bytes diverged for stereo44"
    );
}

#[test]
fn ogg_headers_stereo48_match_ffmpeg() {
    let serial = u32::from_le_bytes(*include_bytes!("vectors/ogg/serial_stereo48.bin"));
    let actual = write_header_pages(SampleRate::Hz48000, Channels::Stereo, serial);
    let expected = include_bytes!("vectors/ogg/headers_stereo48.ogg");
    assert_eq!(
        actual.as_slice(),
        expected.as_slice(),
        "ogg header bytes diverged for stereo48"
    );
}
