# Phase 9a — Pipeline Wiring Implementation Plan

> No worktrees. Work on `main`.

**Goal:** Wire `encode(samples, rate, channels) -> Vec<u8>` end-to-end across Phases 1–8 so that calling it produces *some* ogg bytes (not yet byte-identical to ffmpeg). The byte-equality validation is Phase 9b.

This phase is the integration that Phases 2–8 deferred to. It requires porting three additional libvorbis pieces:

1. **The Vorbis sin window** (`~/Documents/src/libvorbis/lib/window.c`'s `_vorbis_window_get` / inline application). For n=2048 only. Precompute the window samples in `tools/gen-tables` and commit as `src/tables/window.rs`.
2. **`mapping0_forward`** (`~/Documents/src/libvorbis/lib/mapping0.c` lines 230–697, ~470 LOC). The mapping that drives psy → floor1 → residue per channel.
3. **The block-analysis loop** (`~/Documents/src/libvorbis/lib/block.c`'s `vorbis_analysis_blockout` and friends, plus `~/Documents/src/libvorbis/lib/analysis.c`'s `vorbis_analysis_wrote` and `vorbis_analysis`). We don't need all of it — just enough to drive: input PCM → windowing + overlap → MDCT → mapping0_forward → audio packet bytes.

Plus the actual `encode()` body: validate input length is a multiple of the block-size (or pad with zeros), iterate over blocks, emit headers via `OggStreamWriter`, emit audio packets, EOS.

**Verification (this phase, not parity yet):**
- Code compiles + types check
- Clippy clean
- `encode()` doesn't panic on basic inputs (silence, a few short non-silent inputs) for all 4 (rate, channels) combos
- `lewton::inside_ogg::OggStreamReader` can decode our output without erroring (this is a "not blatantly broken" gate; it doesn't mean parity)

**Architecture additions:**
- `src/tables/window.rs` — committed `pub static SIN_WINDOW_2048: [f32; 2048]`
- `src/window.rs` — applies the window + overlap-add buffer
- `src/mapping0.rs` — port of mapping0_forward and friends
- `src/encode.rs` — the orchestration
- `src/lib.rs::encode` — replace `unimplemented!` with `crate::encode::encode_impl(samples, rate, channels)`

---

## Tasks

### Task 9a.1: Window table

- [ ] Extend `tools/gen-tables/src/main.rs` to emit `src/tables/window.rs` containing `pub static SIN_WINDOW_2048: [f32; 2048]` computed from libvorbis's window formula:
  ```
  y[i] = sin(0.5 * PI * sin(PI * (i + 0.5) / 2048).powi(2))
  ```
  for `i in 0..2048`. Compute in `f64`, cast to `f32` per-element. Verify the window sums to N/2 = 1024.0 (Vorbis window has the constant-overlap-add property).
- [ ] Add `tables::window` to `src/tables/mod.rs`.
- [ ] Run `just regen-trig-table` (or whatever the gen-tables recipe is now). Commit `src/tables/window.rs`.
- [ ] Commit as `Phase 9a: window table`.

### Task 9a.2: Window + overlap-add

- [ ] Create `src/window.rs` with:
  - `WindowingBuffer` struct that holds the previous block's *unwindowed* second half (so the next block's first half can be combined with it via overlap-add to produce the next MDCT input)
  - `WindowingBuffer::new() -> Self`
  - `WindowingBuffer::push_block(&mut self, samples: &[i16; 2048]) -> [f32; 2048]` — converts to f32, applies window, overlaps with held data
  
  Read `~/Documents/src/libvorbis/lib/block.c::_vds_shared_init` and `vorbis_analysis_blockout` to understand the overlap layout. There's a small twist: the FIRST block needs to be pre-padded (no previous data); the LAST block needs to be post-padded (no next data). Reference: how libvorbis handles `eofflag` in `vorbis_analysis_blockout`.

- [ ] Add unit tests for the windowing math: window the all-ones signal, assert `output[i] == SIN_WINDOW_2048[i]`.

- [ ] Wire `mod window;` into `src/lib.rs`.

- [ ] Commit as `Phase 9a: windowing + overlap-add buffer`.

### Task 9a.3: Port mapping0_forward

- [ ] Read `~/Documents/src/libvorbis/lib/mapping0.c` lines 230–697.
- [ ] The function does roughly:
  1. Compute log magnitude spectrum from MDCT output
  2. For each channel: run psy compute_mask, classify floor1 posts, encode floor1 packet
  3. Compute couplings (Q5 stereo uses lossless coupling — check the setup blob's mode/mapping definitions to confirm)
  4. Compute residue = MDCT - rendered_floor; quantize/normalize via `_vp_couple_quantize_normalize`
  5. Encode residue packet
  6. Stitch the bits into one audio packet body
- [ ] Create `src/mapping0.rs` mirroring this structure. The function signature roughly:
  ```rust
  pub(crate) fn mapping0_forward(
      mdct_outputs: &[Vec<f32>],     // per-channel MDCT output (1024 each for n=2048)
      psy: &[Psy],                   // per-channel psy state (or shared)
      floor1: &[Floor1State],
      residue: &ResidueSetup,
      codebooks: &[Codebook],
      mode_index: usize,             // always 0 for our long-block config
      w: &mut BitWriter,
  )
  ```
- [ ] Match libvorbis's bit emission order EXACTLY. The audio packet body has a specific layout: mode bits → window flags (only for short blocks; we skip) → floor1 bits per channel → coupling bits → residue bits.
- [ ] **The audio packet does NOT have the `0x05 + "vorbis"` sync prefix.** Audio packets are unframed. Only headers have sync bytes.
- [ ] Add localized clippy `#[allow(...)]` as needed (no restructuring).

- [ ] Commit as `Phase 9a: mapping0_forward (literal port)`.

### Task 9a.4: Encode orchestration

- [ ] Create `src/encode.rs` with:
  ```rust
  pub(crate) fn encode_impl(
      samples: &[i16],
      rate: SampleRate,
      channels: Channels,
  ) -> Vec<u8> {
      // 1. Validate (or just silently round-trip) — input is i16 interleaved
      // 2. De-interleave into per-channel buffers
      // 3. Determine number of blocks: ceil(N / 1024) or so (depends on overlap math)
      // 4. Pre-pad with silence to handle first-block lookahead
      // 5. Init OggStreamWriter with serial = ??? (see below)
      // 6. Emit id, comment, setup headers (via headers::write_*)
      // 7. For each block:
      //    - window + overlap (per channel)
      //    - mdct_forward (per channel)
      //    - mapping0_forward (combines channels into one packet)
      //    - emit audio packet via OggStreamWriter
      //    - update granule position
      // 8. Final block: emit with EOS
      // 9. into_bytes()
  }
  ```
- [ ] **Stream serial number**: for byte-identical to ffmpeg we need the same serial. ffmpeg's serial is randomised per encode. **Make `encode()` use a fixed serial** (e.g., 0xDEADBEEF) — Phase 9b's parity test will work around the serial mismatch by either patching ffmpeg's output to use the same serial, or by extracting the serial from ffmpeg's output and re-feeding it into a re-run of `encode()`. (Spoiler: simplest is to make `encode()` accept an internal `Option<u32>` serial parameter, default random, used by tests.)
  
  For now: hardcode `0xDEADBEEF` as the serial. Phase 9b will refactor.

- [ ] Replace `lib.rs::encode`'s `unimplemented!()` with the call.

- [ ] Add a smoke test in `tests/encode_smoke.rs`:
  ```rust
  #[test]
  fn encode_silence_mono44_does_not_panic() {
      let samples = vec![0i16; 44100];  // 1 second of silence
      let bytes = lewtoff::encode(&samples, lewtoff::SampleRate::Hz44100, lewtoff::Channels::Mono);
      assert!(bytes.len() > 1000, "should produce non-trivial output");
      // basic sanity: starts with OggS magic
      assert_eq!(&bytes[0..4], b"OggS");
  }
  ```
  Add similar for the other 3 combos with silence and a simple sine wave.

- [ ] **Lewton smoke test** — try decoding our output:
  ```rust
  #[test]
  fn lewton_can_decode_our_silence_mono44() {
      let samples = vec![0i16; 44100];
      let bytes = lewtoff::encode(&samples, lewtoff::SampleRate::Hz44100, lewtoff::Channels::Mono);
      let mut reader = lewton::inside_ogg::OggStreamReader::new(std::io::Cursor::new(bytes)).expect("lewton init");
      let mut decoded = Vec::new();
      while let Some(packet) = reader.read_dec_packet_itl().expect("decode") {
          decoded.extend_from_slice(&packet);
      }
      assert!(!decoded.is_empty(), "decoded output should be non-empty");
      // For silence, max |sample| should be small
      let max = decoded.iter().map(|s| s.abs()).max().unwrap_or(0);
      assert!(max < 1000, "silence should decode to near-zero, got max={max}");
  }
  ```

  This is a "not blatantly wrong" test — if lewton CAN decode our output AND it's silence-ish for silence input, we have a working pipeline (modulo byte-identity).

- [ ] Commit as `Phase 9a: encode() orchestration + smoke tests`.

### Task 9a.5: Push, verify CI

- [ ] All previous tests pass + new smoke tests pass
- [ ] `just verify` exits 0
- [ ] `cargo build --target wasm32-unknown-unknown --release` exits 0
- [ ] `git push origin main`
- [ ] CI green

**Phase 9a gate:**
- `lewtoff::encode(silence, ...)` for all 4 combos returns ogg bytes that:
  - Start with `OggS` magic
  - Lewton can decode without erroring
  - For silence input, decoded samples are near-zero
- All Phase 1–8 tests still pass
- CI green

This does NOT validate byte-identity with ffmpeg. That's Phase 9b.

---

## Self-Review

**Spec coverage (against README §7 Phase 9):**
- "encode() wires phases 1–8 together" → Tasks 9a.1–9a.4 ✓
- "high-level invariants (block alignment, eos granule)" → Task 9a.4 ✓
- "expose the public API" → Task 9a.4 ✓
- "every parity test in §6.4 passes on Linux + macOS + wasm32" → Phase 9b

**Open concerns:**
- The first call to `mapping0_forward` is the place that ties together psy + floor1 + residue + the encode-side fields of `Floor1Setup` (`maxover`, `maxunder`, etc. that Phase 4 left at 0.0). Wire those up by reading the Q5 mode/mapping defs from the setup blob OR by hard-coding (per Q5's known values from libvorbis source).
- Coupling: README §2 says "Coupling: None (left/right, no mid/side)". But Q5's mapping0 in libvorbis DOES enable lossless point/stereo coupling for stereo inputs. Either (a) accept the coupling and port it (matches ffmpeg byte-for-byte; honors the byte-identical mission over the README's coupling promise), or (b) refuse stereo entirely. **Pick (a).** The README §2 is wrong on coupling for the same reason it was wrong on the vendor string.
