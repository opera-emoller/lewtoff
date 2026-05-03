# Phase 9c — Short Block Support Plan

> No worktrees. Work on `main`.

**Goal:** Add n=256 (short) block support so lewtoff matches ffmpeg's actual block-switching behavior. ffmpeg uses mode 0 (short) for the first audio packet, mode 1 (long) for subsequent packets when there are no transients. Our test corpus is non-transient (silence, sine, ramp) so we don't need to implement transient detection — we hardcode the "first block is short, rest are long" pattern.

**Scope-of-change estimate:** ~600-1000 LOC additions. Most expensive: window-transition logic and MDCT generalization.

---

## Architecture

**Tables:**
- `src/tables/trig.rs` (existing) — TRIG_2048, BITREV_2048, SCALE_2048
- New entries OR sibling file: `TRIG_256`, `BITREV_256`, `SCALE_256`
- `src/tables/window.rs` (existing) — `SIN_WINDOW_2048`
- New entry: `SIN_WINDOW_256`

**MDCT:**
- `src/mdct.rs::mdct_forward` is currently hardcoded for n=2048.
- Make it dispatch on `n`:
  - `pub(crate) fn mdct_forward_long(input: &[f32; 2048], output: &mut [f32; 1024])` — current
  - `pub(crate) fn mdct_forward_short(input: &[f32; 256], output: &mut [f32; 128])` — new, same code structure but uses TRIG_256/BITREV_256/SCALE_256
- Easiest: copy mdct_forward and make a sibling `mdct_short.rs` (or inside the same file) parameterized by the constants.

**Windowing:**
- `src/window.rs` already has the basic windowing buffer. Extend it to:
  - Track previous block size (lW)
  - Apply mode-aware window: full long window when lW=long, transition window when lW=short and W=long
  - Reference: `~/Documents/src/libvorbis/lib/window.c::_vorbis_apply_window`
- A long block following a short block uses windows that are flat in the middle and short-sized at the edges. This is the trickiest math here.

**Encode loop:**
- Currently `encode.rs` always uses long blocks.
- Update to:
  1. First block: emit a SHORT block (256 samples worth)
  2. Subsequent blocks: emit LONG blocks (2048 samples)
  3. Track lW (previous block size) and pass to mapping0_forward so it can emit the right window flags
- The short-first-block uses 256 input samples (assuming pre-padding); the second block has prev=short so window flags reflect that.

**mapping0_forward:**
- Already takes a mode index but always called with mode 1.
- Make it dispatch to:
  - mode 0 → uses floor 0 (the Q5 short floor1 config), residue 0 (short residue config)
  - mode 1 → uses floor 1 (long), residue 1 (long)
- The Q5 setup blob has both modes already unpacked in `Q5Setup`.

---

## Tasks

### Task 9c.1: Tables for n=256

- [ ] Extend `tools/gen-tables/src/main.rs` to also write TRIG_256, BITREV_256, SCALE_256, SIN_WINDOW_256.
- [ ] Run `just regen-trig-table` (or whatever the recipe is). Commit `src/tables/trig.rs` and `src/tables/window.rs` (now with both sizes).
- [ ] Sanity-check the values: SIN_WINDOW_256 should sum to 128.0; the window's COLA property holds.
- [ ] Commit as `Phase 9c: n=256 tables`.

### Task 9c.2: MDCT for n=256

- [ ] Modify `src/mdct.rs` to expose two functions: `mdct_forward_long` (existing renamed) and `mdct_forward_short`.
- [ ] The implementation: same algorithm, different constants (n, log2n, table refs).
- [ ] Ideally factor common code into a private generic that takes `&[f32]` slices and the relevant table refs. Two thin wrappers call it.
- [ ] Update `tests/mdct.rs` to keep the existing 6 vector tests passing for the long path. Optionally generate new reference vectors for n=256 via `tools/gen-vectors-mdct` (extend it to emit n=256 vectors too) and add n=256 tests.
- [ ] Commit as `Phase 9c: MDCT n=256`.

### Task 9c.3: Window with long↔short transitions

- [ ] Read `~/Documents/src/libvorbis/lib/window.c::_vorbis_apply_window` to understand the 4 window-mode combinations: short/short, short/long-with-prev-short, long-with-prev-short/short, long/long, long/long-with-next-short, etc. (There are roughly 4 or 6 distinct cases.)
- [ ] Update `src/window.rs::WindowingBuffer` to:
  - Track `lW: bool` (true = previous was long)
  - `push_block(samples: &[i16], block_size: BlockSize) -> Vec<f32>` — accepts 256 or 2048 samples; applies the right window
  - The window-mode-aware logic is in here
- [ ] Add unit tests for at least the long/long case (regression) and one transition case.
- [ ] Commit as `Phase 9c: long↔short window transitions`.

### Task 9c.4: mapping0 + encode dispatch

- [ ] Update `src/mapping0.rs::mapping0_forward` to take a `mode_index: usize` (0 or 1) and dispatch to the right floor/residue from `Q5Setup.modes[mode_index]`.
- [ ] Update `src/encode.rs::encode_with_serial` to:
  - Block 0: `block_size = SHORT, mode = 0`
  - Blocks 1+: `block_size = LONG, mode = 1`
  - Track lW (previous block was long?) and pass to window + mapping0 for prev_window flag
  - Update granule position bookkeeping: a short block decodes to 128 samples; long to 1024.
- [ ] The audio-packet header bits emitted by mapping0_forward must match the layout: mode bit (1 bit since 2 modes → ilog(1) = 1 bit), then for long blocks the prev_window+next_window flags (1 bit each).
- [ ] Commit as `Phase 9c: encode dispatch for short-then-long pattern`.

### Task 9c.5: Run parity, iterate

- [ ] Un-ignore at least `parity_silence_mono44` and run it.
- [ ] If it passes: un-ignore the rest, run them all.
- [ ] If it fails: use `parity-diff` to identify the new divergence point. Common candidates:
  - Granule position: short block decodes to 128 samples, not 1024
  - Audio packet header bits in long blocks: prev_window flag depends on lW
  - Window math: edge cases in the transition window
- [ ] Iterate until at least `parity_silence_mono44` passes. Report what does/doesn't pass.

### Task 9c.6: Push, verify CI

- [ ] All previous tests pass (the long-only tests should still pass)
- [ ] At least one parity test passes (un-ignored)
- [ ] `just verify` exits 0
- [ ] `cargo build --target wasm32-unknown-unknown --release` exits 0
- [ ] `git push origin main`
- [ ] CI green (the parity test will only run with `--features oracle` which CI does have)

---

## Self-Review

**Spec coverage:** Adds short-block support that the README §2 originally excluded but is required for byte-equality with ffmpeg's actual behavior.

**Out of scope (intentional):** Transient detection. Real-world audio with attacks (drums, etc.) would have ffmpeg emit short blocks mid-stream. Our test corpus avoids this.

**Risks:**
- Window math at long↔short transitions is the most likely place for subtle bit-level divergence. Test the transition unit tests carefully.
- Granule position bookkeeping has changed: if I miscalculate the EOS granule, ffmpeg will see a different bit pattern in the last page header.
