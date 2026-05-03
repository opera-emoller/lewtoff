# Phase 9b — Parity Test + Debug Iterations Plan

> No worktrees. Work on `main`.

**Goal:** Add a parity test that runs `ffmpeg -c:a libvorbis -q:a 5` and `lewtoff::encode()` on the same PCM input and asserts byte-equality. Then iterate to fix divergences.

**Realistic expectation:** First parity attempt will almost certainly fail. The wiring landed in Phase 9a is mechanically correct (pipeline compiles, lewton decodes our output as silence-ish for silence input) but byte-equality is much stricter than "decodable" — every bit of every packet must match libvorbis. Expect 5–10 debug iterations.

This plan is structured into:
1. **Test infrastructure** (deterministic, single dispatch)
2. **First parity run** (capture initial divergence point)
3. **Iterative debug** (one or more dispatches per divergence cluster)

---

## Architecture

**Stream serial number:** `lewtoff::encode()` currently hardcodes `0xDEADBEEF`. Refactor:

```rust
// public — unchanged
pub fn encode(samples: &[i16], rate: SampleRate, channels: Channels) -> Vec<u8> {
    encode_with_serial(samples, rate, channels, generate_random_serial())
}

// internal — new — for tests
pub(crate) fn encode_with_serial(samples: &[i16], rate: SampleRate, channels: Channels, serial: u32) -> Vec<u8> { ... }

#[cfg(any(test, feature = "test-internals"))]
#[doc(hidden)]
pub fn __encode_with_serial_for_test(...) -> Vec<u8> { encode_with_serial(...) }
```

For now, since we control both sides, the parity test:
1. Runs ffmpeg, captures its bytes
2. Reads ffmpeg's serial from bytes 14–17 of the output
3. Calls `lewtoff::encode_with_serial(..., serial)` with the same serial
4. Diffs the bytes

**Diff helper:** Add `tools/parity-diff/` — a binary that takes (lewtoff_bytes, ffmpeg_bytes), parses both via the `ogg` crate, and prints where they diverge:
```
Page 0 (id header): match (28 bytes)
Page 1 (comment + setup): match (4023 bytes)
Page 2 (audio packet 1): DIVERGE at byte 12 of packet
  lewtoff: 0x4d 0x2a ...
  ffmpeg:  0x4d 0x2b ...
```

This makes debug iterations much faster than `assert_eq!(a, b)`'s `left != right`.

---

## Tasks

### Task 9b.1: Parity test infrastructure

- [ ] **Step 1: Refactor `encode_with_serial`** as described above. The public `encode()` keeps its signature and uses a random serial; tests use `encode_with_serial`.

- [ ] **Step 2: Create `tools/parity-diff/`** as a workspace member (Cargo.toml + src/main.rs). It takes two file paths, parses each via `ogg::PacketReader`, and emits a structured diff per page → per packet → per byte. Skip the serial-number bytes when comparing page headers (since they may differ; the test caller is supposed to align them but the diff tool should be robust).

- [ ] **Step 3: Add `tests/parity.rs`** gated behind the `oracle` feature already declared in `Cargo.toml`:

  ```rust
  #![cfg(feature = "oracle")]
  
  use std::io::Write;
  use std::process::{Command, Stdio};
  
  fn ffmpeg_encode_q5(samples: &[i16], rate: u32, channels: u16) -> Vec<u8> {
      // (same as templates/ci.yml describes)
  }
  
  fn extract_serial(ogg_bytes: &[u8]) -> u32 {
      u32::from_le_bytes(ogg_bytes[14..18].try_into().unwrap())
  }
  
  fn assert_parity(samples: &[i16], rate: SampleRate, channels: Channels) {
      let ffmpeg_bytes = ffmpeg_encode_q5(samples, rate as u32 (or w/e), channels.to_u16());
      let serial = extract_serial(&ffmpeg_bytes);
      let lewtoff_bytes = lewtoff::encode_with_serial(samples, rate, channels, serial);
      
      if lewtoff_bytes != ffmpeg_bytes {
          // print first divergence offset and surrounding context
          let div = first_diff(&lewtoff_bytes, &ffmpeg_bytes);
          panic!("parity diverged at byte {div}\n  lewtoff len: {}\n  ffmpeg len:  {}\n  context: lewtoff={:02x?} ffmpeg={:02x?}",
                 lewtoff_bytes.len(), ffmpeg_bytes.len(),
                 &lewtoff_bytes[div.saturating_sub(8)..(div + 16).min(lewtoff_bytes.len())],
                 &ffmpeg_bytes[div.saturating_sub(8)..(div + 16).min(ffmpeg_bytes.len())]);
      }
  }
  
  #[test] fn parity_silence_mono44() { assert_parity(&vec![0i16; 44100], SampleRate::Hz44100, Channels::Mono); }
  #[test] fn parity_silence_mono48() { ... }
  #[test] fn parity_silence_stereo44() { ... }
  #[test] fn parity_silence_stereo48() { ... }
  #[test] fn parity_sine_440_mono44() { ... }
  #[test] fn parity_ramp_stereo44() { ... }
  ```

- [ ] **Step 4: Update `Cargo.toml`** to expose `encode_with_serial` to integration tests (via `#[doc(hidden)] pub mod encode;` is fine — same pattern as `mdct`).

- [ ] **Step 5: Run `just parity` (which is `cargo nextest run --features oracle parity_`).** **Capture the first divergence point** — that's the start of debug iteration.

- [ ] **Step 6: Commit** the test infrastructure (without expecting it to pass yet) as `Phase 9b: parity test infrastructure`. Mark the failing tests with `#[ignore]` if necessary so CI stays green; or include a note in the commit that local parity runs reveal divergence at byte X.

### Task 9b.2: First debug iteration

After capturing the first divergence point in Task 9b.1, this task investigates and fixes it. The structure depends on what diverges:

- **If divergence is in the comment/setup headers** (pages 0–1): Phase 7 missed something. Re-check.
- **If divergence is in the first audio packet's mode bits**: mapping0_forward's mode-emit prefix is wrong.
- **If divergence is in floor1 bits**: floor1_forward port has a bug. Compare specific bit patterns.
- **If divergence is in residue bits**: residue port has a bug.
- **If divergence is in granule positions or page boundaries**: encode.rs's block scheduling is wrong.

**Approach:**
1. Use `tools/parity-diff` to identify which page/packet diverges.
2. Use lewton + a hand-rolled bit-level decoder to figure out which field within the packet differs.
3. Cross-reference with libvorbis source.
4. Fix.
5. Re-run.

**Common first-failure patterns** (predictions):
- "Pre-padding samples" — Vorbis encodes a half-block of "lookback" pre-padded zeros at the start. Easy to get wrong.
- "Granule position of first audio page" — should be `0` (since the first block has no decoded output yet) but a naive impl might emit the block size.
- "Mode bit width" — Vorbis modes are encoded with `ilog(num_modes - 1)` bits. Q5 has 2 modes (mode 0 = long, mode 1 = short), so 1 bit. Easy to conflate with "always 1 bit".

### Task 9b.3+ : Iterate

For each divergence found, repeat: identify, fix, re-run, commit. After each fix, push and confirm CI green.

---

## Phase 9b Done Criteria

- All 6+ parity tests pass byte-identically
- The parity tests run as `cargo nextest run --features oracle parity_` locally
- `just verify` exits 0 (parity tests are gated by `oracle` feature, so non-oracle runs aren't affected)
- CI green (CI's existing oracle parity step at the end of `ci.yml` will now actually run tests instead of `--no-tests=warn`)
- Update `ci.yml` to flip `--no-tests=warn` → `--no-tests=fail` for the oracle step (the TODO from Phase 0)
