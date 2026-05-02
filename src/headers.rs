//! Vorbis header packet construction (id, comment, setup) per Vorbis I §4.2.
//!
//! The three packets are byte-identical to what `ffmpeg -c:a libvorbis -q:a 5`
//! produces for the supported (rate × channels) input space.

use crate::bitpack::BitWriter;
use crate::setup_blob::{Q5_SETUP_MONO44, Q5_SETUP_MONO48, Q5_SETUP_STEREO44, Q5_SETUP_STEREO48};
use crate::{Channels, SampleRate};

/// Vendor string written by ffmpeg-libvorbis into the comment header.
/// This is the libavformat identification string, not the libvorbis string.
const VENDOR: &[u8] = b"Lavf61.7.100";

/// User comment written by ffmpeg into the comment header.
const ENCODER_TAG: &[u8] = b"encoder=Lavc61.19.101 libvorbis";

/// Nominal bitrate for mono streams at Q5 (bits/sec), as written by
/// ffmpeg-libvorbis into the id header.
const BITRATE_NOMINAL_MONO: i32 = 96_000;

/// Nominal bitrate for stereo streams at Q5 (bits/sec).
const BITRATE_NOMINAL_STEREO: i32 = 160_000;

/// log2 of the short block size (256 = 2^8).
const BLOCKSIZE_0_LOG2: u32 = 8;

/// log2 of the long block size (2048 = 2^11).
const BLOCKSIZE_1_LOG2: u32 = 11;

/// Write the Vorbis identification header packet (packet type 0x01).
///
/// Per Vorbis I §4.2.2.
pub fn write_id_header(rate: SampleRate, channels: Channels, w: &mut BitWriter) {
    let audio_sample_rate: u32 = match rate {
        SampleRate::Hz44100 => 44_100,
        SampleRate::Hz48000 => 48_000,
    };
    let audio_channels: u32 = match channels {
        Channels::Mono => 1,
        Channels::Stereo => 2,
    };
    let bitrate_nominal: i32 = match channels {
        Channels::Mono => BITRATE_NOMINAL_MONO,
        Channels::Stereo => BITRATE_NOMINAL_STEREO,
    };

    w.write(0x01, 8);
    for &b in b"vorbis" {
        w.write(b as u32, 8);
    }

    w.write(0, 32);
    w.write(audio_channels, 8);
    w.write(audio_sample_rate, 32);
    w.write(0, 32);
    w.write(bitrate_nominal as u32, 32);
    w.write(0, 32);

    w.write(BLOCKSIZE_0_LOG2, 4);
    w.write(BLOCKSIZE_1_LOG2, 4);

    w.write(1, 1);
}

/// Write the Vorbis comment header packet (packet type 0x03).
///
/// Per Vorbis I §4.2.3. The vendor string and encoder tag match what
/// ffmpeg-libvorbis writes on the reference machine.
pub fn write_comment_header(w: &mut BitWriter) {
    w.write(0x03, 8);
    for &b in b"vorbis" {
        w.write(b as u32, 8);
    }

    w.write(VENDOR.len() as u32, 32);
    for &b in VENDOR {
        w.write(b as u32, 8);
    }

    w.write(1, 32);

    w.write(ENCODER_TAG.len() as u32, 32);
    for &b in ENCODER_TAG {
        w.write(b as u32, 8);
    }

    w.write(1, 1);
}

/// Write the Vorbis setup header packet (packet type 0x05).
///
/// The packet is the embedded Q5 setup blob byte-for-byte; it already
/// starts with the 0x05 + "vorbis" sync sequence. The blob varies by
/// both sample rate and channel count.
pub fn write_setup_header(rate: SampleRate, channels: Channels, w: &mut BitWriter) {
    let blob = match (rate, channels) {
        (SampleRate::Hz44100, Channels::Mono) => Q5_SETUP_MONO44,
        (SampleRate::Hz48000, Channels::Mono) => Q5_SETUP_MONO48,
        (SampleRate::Hz44100, Channels::Stereo) => Q5_SETUP_STEREO44,
        (SampleRate::Hz48000, Channels::Stereo) => Q5_SETUP_STEREO48,
    };
    for &b in blob {
        w.write(b as u32, 8);
    }
}
