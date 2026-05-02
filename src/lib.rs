//! lewtoff: pure-Rust Ogg Vorbis encoder, byte-identical to libvorbis 1.3.7 Q5.
//!
//! See `README.md` for scope, design, and constraints. The crate intentionally
//! has a tiny public surface — one function and two enums — because the
//! supported input space is closed by construction (no `Result` needed).

#![forbid(unsafe_code)]

#[allow(dead_code)]
mod bitpack;

#[allow(dead_code)]
mod tables;

#[allow(dead_code)]
mod setup_blob;

#[allow(dead_code)]
mod codebook;

#[doc(hidden)]
pub mod mdct;

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
pub fn encode(_samples: &[i16], _rate: SampleRate, _channels: Channels) -> Vec<u8> {
    unimplemented!("not yet implemented — see Phase 9 in README.md")
}
