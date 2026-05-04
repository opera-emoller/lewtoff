# lewtoff

A pure-Rust Ogg Vorbis **encoder** that produces byte-for-byte identical
output to libvorbis 1.3.7 at quality Q5 for a constrained input space.
Named after [lewton](https://github.com/RustAudio/lewton) — the pure-Rust
Vorbis decoder. lewtoff is its encoder counterpart.

The crate forbids `unsafe` and pulls in a single runtime dependency
(the `ogg` crate, for page framing and CRC).

## Status

All synthetic parity tests (silence, sine 440, ramps, mono+stereo at
44.1k and 48k) match libvorbis 1.3.7 byte-for-byte. A real-audio corpus
sweep (`tests/corpus_sweep.rs::corpus_parity_sweep`) walks any
`<repo_root>/corpus/` directory the contributor has symlinked in and
asserts the same parity for every file recursively; the most recent
sweep over a 5954-file corpus passed 100% byte-identical. The EOS
`eofflag` / post-extrap `n_train` derivation runs a faithful chunk-by-
chunk streaming simulation of libvorbis's `vorbis_analysis_blockout`
(see `envelope::simulate_eofflag`) so no per-input hardcodes are needed.

## Use

```rust
let pcm: &[i16] = /* interleaved 16-bit PCM */;
let ogg = lewtoff::encode(
    pcm,
    lewtoff::SampleRate::Hz44100,
    lewtoff::Channels::Stereo,
);
```

Public surface is one function plus two enums. No `Result` (the input
space is closed by construction), no streaming, no `Write` trait.

## Supported input

| | |
|---|---|
| Sample format | Interleaved `i16` PCM |
| Sample rate   | 44100 Hz **or** 48000 Hz |
| Channels      | Mono **or** Stereo |
| Quality       | Q5 (≈160 kbps stereo, ≈80 kbps mono) |

Outside this space, behavior is undefined.

## How parity is enforced

`tests/parity.rs` shells out to `tools/oracle-encoder/oracle-encoder` —
a small C program that statically links the libvorbis 1.3.7 sources
vendored at `tools/debug-libvorbis-dump/vendored-libvorbis/`, compiled
with `-O0 -ffp-contract=off -std=c99` so the bytes are deterministic
across hosts.

Build the oracle once after cloning:

```sh
sudo apt install libogg-dev    # or: brew install libogg
./tools/oracle-encoder/build.sh
```

Then:

```sh
cargo nextest run --features oracle parity_
```

For the `corpus_parity_sweep` test, symlink your audio corpus at
`<repo_root>/corpus/` and run

```sh
cargo nextest run --features oracle --no-tests=warn corpus_parity_sweep -- --include-ignored
```

The directory is gitignored; the sweep walks it recursively and accepts
`.wav`, `.mp3`, `.ogg`, `.flac`, `.m4a`, `.aif`, and `.aiff`. Files are
decoded via `ffmpeg` to s16le 44.1kHz stereo before encoding. Set
`CORPUS_LIMIT=N` to test only the first N files for a quick smoke run.

## Tests

```sh
cargo nextest run                              # unit + integration
cargo nextest run --features oracle parity_    # parity vs oracle
```

CI (`.github/workflows/ci.yml`) builds the oracle from the vendored
sources and runs both passes on Ubuntu, plus a `wasm32-unknown-unknown`
build to lock in target portability.

## Why this works

libvorbis's encoder is f32-throughout and does not use FMA when built
with `-ffp-contract=off`. Rust's `f32` arithmetic on every tier-1
target produces the same IEEE 754 results as that C build, provided no
operation is contracted into FMA. Transcendentals (`sin`/`cos`/`log`/
`exp`) are precomputed at build time — at runtime the encoder only
does table lookups and arithmetic, so libm differences across platforms
don't matter.

The detailed precision pitfalls hit during the port — f32-vs-f64
promotion, `rint` vs `.round()`, the IEEE_FLOAT32 `todB` inline, and so
on — live in the commit log.

## License

Apache-2.0 (see `LICENSE`). lewtoff is a clean-room port of portions of
libvorbis 1.3.7, which is BSD-3-Clause from the Xiph.Org Foundation; the
upstream attribution is in `NOTICE`. The vendored libvorbis source under
`tools/debug-libvorbis-dump/vendored-libvorbis/` retains its own
`COPYING`.
