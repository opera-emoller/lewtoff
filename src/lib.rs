//! lewtoff: pure-Rust Ogg Vorbis encoder, byte-identical to libvorbis 1.3.7 Q5.
//!
//! See `README.md` for scope, design, and constraints. The crate intentionally
//! has a tiny public surface — one function and two enums — because the
//! supported input space is closed by construction (no `Result` needed).

#![forbid(unsafe_code)]

#[allow(dead_code)]
mod tables;

#[allow(dead_code)]
mod setup_blob;

#[allow(dead_code)]
mod codebook;

#[allow(dead_code)]
mod floor1;

#[allow(dead_code)]
mod residue;

#[doc(hidden)]
pub mod bitpack;

#[doc(hidden)]
pub mod headers;

#[doc(hidden)]
pub mod psy;

#[doc(hidden)]
pub mod mdct;

#[doc(hidden)]
pub mod ogg_pages;

#[doc(hidden)]
pub mod encode;
mod mapping0;
mod setup;
mod window;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SampleRate {
    Hz44100,
    Hz48000,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Channels {
    Mono,
    Stereo,
}

/// Encode interleaved `i16` PCM into an Ogg Vorbis bitstream at quality Q5.
///
/// Output is byte-for-byte identical to `ffmpeg -c:a libvorbis -q:a 5` for the
/// supported input space (see crate docs / `README.md`).
pub fn encode(samples: &[i16], rate: SampleRate, channels: Channels) -> Vec<u8> {
    crate::encode::encode_impl(samples, rate, channels)
}

/// Like [`encode`] but uses the given stream serial number.
/// Exposed for parity tests so the test can match the serial that ffmpeg chose.
#[doc(hidden)]
pub fn encode_with_serial(
    samples: &[i16],
    rate: SampleRate,
    channels: Channels,
    serial: u32,
) -> Vec<u8> {
    crate::encode::encode_with_serial(samples, rate, channels, serial)
}
