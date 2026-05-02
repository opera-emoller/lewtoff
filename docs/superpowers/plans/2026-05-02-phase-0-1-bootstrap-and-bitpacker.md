# Phase 0 (Repo Bootstrap) + Phase 1 (Bitpacker) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Worktrees:** This project uses the main checkout — do NOT create a git worktree. Work directly on `main` in `/Users/emoller/Documents/src/lewtoff`.

**Goal:** Stand up the lewtoff Rust crate scaffolding (Phase 0) and land the LSB-first Vorbis bit writer with property-based tests (Phase 1).

**Architecture:** Single-crate Rust repo at the repo root (no workspace). Templates are pre-staged in `templates/`; copy verbatim with one CI tweak (skip `wasm-pack test --node` until Phase 10). The bitpacker is an in-crate `bitpack.rs` module exposing a `BitWriter` struct with LSB-first packing per Vorbis I §2.1.4. Drop `spike/` and `templates/` after they're consumed.

**Tech Stack:** Rust 1.95.0 (pinned via `rust-toolchain.toml`), `ogg = "0.9"` runtime dep, `lewton = "0.10"` dev-dep, `cargo-nextest`, `just`, GitHub Actions.

---

## File Structure

**Phase 0 creates / modifies:**
- Create: `Cargo.toml` (from `templates/Cargo.toml`)
- Create: `rust-toolchain.toml` (from `templates/rust-toolchain.toml`)
- Create: `justfile` (from `templates/justfile`)
- Create: `.gitignore` (from `templates/.gitignore`)
- Create: `.githooks/pre-commit` (from `templates/pre-commit`, chmod +x)
- Create: `.github/workflows/ci.yml` (from `templates/ci.yml`, with wasm-pack test step removed)
- Create: `.config/nextest.toml` (from `templates/nextest.toml`)
- Create: `src/lib.rs` (public API skeleton: `encode`, `SampleRate`, `Channels`)
- Delete: `spike/` (served its purpose; preserved in git history)
- Delete: `templates/` (consumed)
- Delete: `.DS_Store`, `spike/.DS_Store` (macOS cruft)

**Phase 1 creates / modifies:**
- Create: `src/bitpack.rs` (the `BitWriter` struct + inline `#[cfg(test)]` tests)
- Modify: `src/lib.rs` (declare `mod bitpack;` — keep it `pub(crate)`, not part of the public API)

---

## Phase 0 — Repo Bootstrap

### Task 0.1: Stage the root template files

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `justfile`
- Create: `.gitignore`

- [ ] **Step 1: Copy `templates/Cargo.toml` → `Cargo.toml` verbatim**

The exact contents required (from `templates/Cargo.toml`):

```toml
[package]
name = "lewtoff"
version = "0.1.0"
edition = "2021"
rust-version = "1.80"
license = "MIT OR Apache-2.0"
description = "Pure-Rust Ogg Vorbis encoder, byte-identical to libvorbis Q5."
repository = "https://github.com/<owner>/lewtoff"
categories = ["multimedia::audio", "encoding"]
keywords = ["vorbis", "ogg", "audio", "encoder", "lossy"]

[features]
default = []
# Enables the parity tests under tests/parity.rs. Requires `ffmpeg` on PATH
# with libvorbis 1.3.7. Off by default so contributors without ffmpeg can
# still run the bulk of the suite locally.
oracle = []

[dependencies]
ogg = "0.9"

[dev-dependencies]
lewton = "0.10"

[profile.release]
debug = "line-tables-only"

[lints.rust]
unsafe_code = "forbid"
```

- [ ] **Step 2: Copy `templates/rust-toolchain.toml` → `rust-toolchain.toml` verbatim**

```toml
[toolchain]
channel = "1.95.0"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 3: Copy `templates/justfile` → `justfile` verbatim**

(Use `cp templates/justfile justfile` — the file is ~70 lines and reproduced from the template unchanged.)

- [ ] **Step 4: Copy `templates/.gitignore` → `.gitignore` verbatim**

```
/target
*.log
.DS_Store
/oracle-cache
/tests/fixtures/oracle-out/*.ogg
!/tests/fixtures/oracle-out/.keep
```

### Task 0.2: Stage the dot-directories

**Files:**
- Create: `.githooks/pre-commit`
- Create: `.github/workflows/ci.yml`
- Create: `.config/nextest.toml`

- [ ] **Step 1: `mkdir -p .githooks .github/workflows .config`**

- [ ] **Step 2: Copy `templates/pre-commit` → `.githooks/pre-commit` and `chmod +x .githooks/pre-commit`**

Verify executable bit: `ls -l .githooks/pre-commit` should show `-rwxr-xr-x`.

- [ ] **Step 3: Copy `templates/nextest.toml` → `.config/nextest.toml` verbatim**

```toml
[profile.default]
status-level = "fail"
final-status-level = "flaky"
slow-timeout = { period = "30s", terminate-after = 2 }
```

- [ ] **Step 4: Copy `templates/ci.yml` → `.github/workflows/ci.yml` with the `wasm-pack test --node` step REMOVED**

The wasm job ships only with the build check (no tests yet — wasm-pack would fail with `error: no tests in wasm-bindgen-test target` until Phase 10 wires up wasm-bindgen-test). Final contents:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - uses: actions-rust-lang/setup-rust-toolchain@v1
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - uses: taiki-e/install-action@v2
        with:
          tool: cargo-nextest
      - name: Install ffmpeg (oracle parity tests shell out to it)
        run: sudo apt-get update && sudo apt-get install -y ffmpeg
      - name: Verify libvorbis version
        run: |
          ffmpeg -version 2>&1 | grep -E "libvorbis 1\.3" \
            || { echo "Wrong libvorbis version — parity tests pin to 1.3.x"; exit 1; }
      - run: cargo fmt --all -- --check
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo nextest run --status-level=fail
      - name: Oracle parity (Linux glibc libm path)
        run: cargo nextest run --features oracle parity_

  wasm:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - uses: actions-rust-lang/setup-rust-toolchain@v1
        with:
          target: wasm32-unknown-unknown
      - uses: Swatinem/rust-cache@v2
      - name: Build for wasm32-unknown-unknown
        run: cargo build --target wasm32-unknown-unknown --release
```

(Diff vs `templates/ci.yml`: removed the `taiki-e/install-action@v2` step that installs wasm-pack, and removed the final `wasm-pack test --node` step. Kept everything else verbatim.)

### Task 0.3: Create the public-API skeleton

**Files:**
- Create: `src/lib.rs`

- [ ] **Step 1: `mkdir -p src`**

- [ ] **Step 2: Write `src/lib.rs` with the exact contents below**

```rust
//! lewtoff: pure-Rust Ogg Vorbis encoder, byte-identical to libvorbis 1.3.7 Q5.
//!
//! See `README.md` for scope, design, and constraints. The crate intentionally
//! has a tiny public surface — one function and two enums — because the
//! supported input space is closed by construction (no `Result` needed).

#![forbid(unsafe_code)]

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
    // Phase 9 will wire this through the encoder. Until then, calling this is
    // a programmer error.
    unimplemented!("end-to-end encode is wired up in Phase 9")
}
```

### Task 0.4: Drop the consumed staging directories

**Files:**
- Delete: `spike/`
- Delete: `templates/`
- Delete: `.DS_Store` files

- [ ] **Step 1: `rm -rf spike templates`**

- [ ] **Step 2: `find . -name .DS_Store -not -path './.git/*' -delete`**

- [ ] **Step 3: `git status` to confirm the deletions are staged for tracking**

Expected: `spike/` and `templates/` show as deleted; new files show as untracked under `Cargo.toml`, `src/`, `.githooks/`, `.github/`, `.config/`, `rust-toolchain.toml`, `justfile`, `.gitignore`.

### Task 0.5: Install hooks, install nextest, run verify

- [ ] **Step 1: `just install-hooks`**

Expected output: `hooks installed: .githooks/pre-commit will run before each commit`. This sets `git config core.hooksPath .githooks`.

- [ ] **Step 2: `just install-tools`** (skip if `cargo-nextest --version` already works)

- [ ] **Step 3: `just verify`**

Expected: fmt-check passes (no .rs files to fail on yet other than `lib.rs`), clippy passes (lib.rs has one `unimplemented!` which is fine), nextest runs `0 tests` and exits success.

If clippy complains about anything (likely candidates: missing docs warnings if any, dead code on the unused parameter underscores), fix in place — the goal is a green `just verify`.

- [ ] **Step 4: Sanity-check the wasm target builds**

```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
```

Expected: clean build. (The `ogg` crate is pure Rust and supports wasm32; lib.rs doesn't yet use it, but cargo still resolves it.)

### Task 0.6: Commit and push

- [ ] **Step 1: Stage everything new**

```bash
git add Cargo.toml rust-toolchain.toml justfile .gitignore \
        .githooks/pre-commit .github/workflows/ci.yml .config/nextest.toml \
        src/lib.rs docs/superpowers/plans/
git add -u    # picks up the spike/ + templates/ deletions
git status    # confirm nothing accidental is staged (no .DS_Store, no Cargo.lock under target/)
```

- [ ] **Step 2: Commit**

```bash
git commit -m "$(cat <<'EOF'
Phase 0: repo bootstrap

Wire up Cargo.toml, rust-toolchain pin, justfile, CI, pre-commit hooks,
and the public API skeleton. Drop the consumed spike/ and templates/
staging dirs (preserved in git history).

CI's wasm-pack test step is intentionally omitted — it'll come back in
Phase 10 when wasm-bindgen-test gets wired up.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 3: Push**

```bash
git push origin main
```

- [ ] **Step 4: Watch CI**

```bash
gh run watch  # interactive
# or
gh run list --limit 1
gh run view --log-failed  # if it fails
```

Expected: both `test` and `wasm` jobs green. If `Verify libvorbis version` fails (Ubuntu shipped a non-1.3.x version), file a follow-up — that's not a blocker for the bootstrap.

**Phase 0 gate (do not start Phase 1 until all are green):**
- `just verify` exits 0 locally
- `cargo build --target wasm32-unknown-unknown --release` exits 0 locally
- CI's `test` job is green
- CI's `wasm` job is green

---

## Phase 1 — Bitpacker

The Vorbis bitpacker is LSB-first within bytes. Spec reference: Vorbis I §2.1.4 ("Bitpacking convention"). Reference implementation: `lib/bitwise.c::oggpack_write` in libvorbis. We're writing only — no reader needed yet (parsing tests come in Phase 7+).

**Bit-packing semantics (the only thing the implementation has to get right):**
- A byte fills LSB-first. The first bit written goes into bit 0 of byte 0; the next into bit 1; and so on. Bit 8 spills into bit 0 of byte 1.
- `write(value, bits)` emits the low `bits` bits of `value`, LSB-first. `bits` is in `0..=32`. Bits above bit `bits-1` of `value` are discarded.
- Writing 0 bits is a no-op (matches libvorbis).
- A multi-byte value is therefore little-endian: `write(0x12345678, 32)` produces `[0x78, 0x56, 0x34, 0x12]`.

### Task 1.1: Write the failing test for the empty-writer invariant

**Files:**
- Create: `src/bitpack.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create `src/bitpack.rs` with the test scaffold and a stub struct that fails the test**

```rust
//! LSB-first bit writer, per Vorbis I §2.1.4.
//!
//! This is the only bit-level I/O the encoder does. Reads come later (Phase 7+
//! when we need to verify the setup-header round-trip via lewton); for now we
//! only emit.

#[derive(Default)]
pub(crate) struct BitWriter {
    bytes: Vec<u8>,
    /// Number of bits already written into the *last* byte of `bytes`. In
    /// `1..=8`. When `bytes` is empty, no bits have been written yet.
    bits_in_last: u8,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Total number of bits written so far.
    pub fn bit_len(&self) -> usize {
        if self.bytes.is_empty() {
            0
        } else {
            (self.bytes.len() - 1) * 8 + self.bits_in_last as usize
        }
    }

    /// Consume the writer and return the underlying bytes.
    /// The final byte is zero-padded in its unused high bits.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Write the low `bits` bits of `value`, LSB-first. `bits` must be `<= 32`.
    pub fn write(&mut self, _value: u32, _bits: u32) {
        unimplemented!("Phase 1 Task 1.2")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_writer_has_zero_bit_len_and_no_bytes() {
        let w = BitWriter::new();
        assert_eq!(w.bit_len(), 0);
        assert_eq!(w.into_bytes(), Vec::<u8>::new());
    }
}
```

- [ ] **Step 2: Wire the module into `lib.rs`** by adding this line right after the `#![forbid(unsafe_code)]` attribute:

```rust
mod bitpack;
```

- [ ] **Step 3: Run the test to confirm it passes (this one doesn't need an implementation — it tests the empty-writer state)**

Run: `cargo nextest run bitpack::tests::empty_writer_has_zero_bit_len_and_no_bytes`
Expected: PASS.

(Yes, this first test is "trivial" — it locks in the API shape before we touch `write`.)

- [ ] **Step 4: Commit the scaffold**

```bash
git add src/bitpack.rs src/lib.rs
git commit -m "$(cat <<'EOF'
Phase 1: bitpack scaffold

Adds the BitWriter struct shape and the empty-state test. write() is
unimplemented; subsequent commits flesh it out under TDD.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 1.2: TDD — single sub-byte write

- [ ] **Step 1: Add the failing test**

Append to the `tests` module in `src/bitpack.rs`:

```rust
#[test]
fn write_low_nibble_lands_in_low_bits_of_first_byte() {
    let mut w = BitWriter::new();
    w.write(0xA, 4);                  // 0b1010
    assert_eq!(w.bit_len(), 4);
    assert_eq!(w.into_bytes(), vec![0x0A]);
}
```

- [ ] **Step 2: Run, expect FAIL with `unimplemented!`**

Run: `cargo nextest run bitpack::tests::write_low_nibble`
Expected: panic with `not implemented: Phase 1 Task 1.2`.

- [ ] **Step 3: Implement `write` — minimal version that handles a single sub-byte write**

Replace the body of `write` with:

```rust
pub fn write(&mut self, value: u32, bits: u32) {
    debug_assert!(bits <= 32, "bits must be <= 32, got {bits}");
    if bits == 0 {
        return;
    }
    // Mask off bits above the requested width.
    let mut value = if bits == 32 { value } else { value & ((1u32 << bits) - 1) };
    let mut bits_remaining = bits as u8;

    while bits_remaining > 0 {
        // If the current last byte is full (or there is no last byte), append a fresh zero byte.
        if self.bytes.is_empty() || self.bits_in_last == 8 {
            self.bytes.push(0);
            self.bits_in_last = 0;
        }

        let space = 8 - self.bits_in_last;
        let take = bits_remaining.min(space);

        // Take the low `take` bits of `value` and OR them into the last byte at offset `bits_in_last`.
        let chunk = (value & ((1u32 << take) - 1)) as u8;
        let last = self.bytes.last_mut().expect("just pushed if needed");
        *last |= chunk << self.bits_in_last;

        self.bits_in_last += take;
        bits_remaining -= take;
        value >>= take;
    }
}
```

- [ ] **Step 4: Re-run — expect PASS**

Run: `cargo nextest run bitpack`
Expected: both tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/bitpack.rs
git commit -m "$(cat <<'EOF'
Phase 1: implement BitWriter::write

LSB-first packing within bytes, per Vorbis I §2.1.4. Handles arbitrary
bit widths up to 32 bits per call, spanning byte boundaries by chunking.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 1.3: TDD — concatenation within a byte

- [ ] **Step 1: Add the failing test**

```rust
#[test]
fn two_nibbles_pack_into_one_byte() {
    let mut w = BitWriter::new();
    w.write(0xA, 4);            // low nibble 0xA
    w.write(0x5, 4);            // high nibble 0x5
    assert_eq!(w.bit_len(), 8);
    assert_eq!(w.into_bytes(), vec![0x5A]);
}
```

- [ ] **Step 2: Run — expect PASS** (the impl from 1.2 already handles this)

Run: `cargo nextest run bitpack::tests::two_nibbles_pack_into_one_byte`

If it fails, the bug is in `bits_in_last` accounting — fix in place.

- [ ] **Step 3: Commit**

```bash
git add src/bitpack.rs
git commit -m "Phase 1: test sub-byte concatenation"
```

(Use the standard Co-Authored-By trailer for all subsequent commits — omitted from this point in the plan for brevity but always include it.)

### Task 1.4: TDD — byte-spanning write

- [ ] **Step 1: Add the failing test**

```rust
#[test]
fn write_spans_byte_boundary() {
    let mut w = BitWriter::new();
    w.write(0xF, 4);            // 4 bits: 0b1111 in low nibble of byte 0
    w.write(0xFF, 8);           // 8 more bits, splits 4 into byte 0 high nibble + 4 into byte 1 low nibble
    assert_eq!(w.bit_len(), 12);
    assert_eq!(w.into_bytes(), vec![0xFF, 0x0F]);
}
```

Reasoning: after the first write, byte 0 = `0b0000_1111` with `bits_in_last = 4`. The second write
emits `0xFF` (bits `1,1,1,1,1,1,1,1`) LSB-first; the first 4 fill byte 0's high nibble (→ `0xFF`),
and the remaining 4 land in byte 1's low nibble (→ `0x0F`).

- [ ] **Step 2: Run — expect PASS**

Run: `cargo nextest run bitpack::tests::write_spans_byte_boundary`

- [ ] **Step 3: Commit**

```bash
git add src/bitpack.rs
git commit -m "Phase 1: test byte-spanning write"
```

### Task 1.5: TDD — 32-bit value emits little-endian bytes

- [ ] **Step 1: Add the failing test**

```rust
#[test]
fn write_u32_emits_little_endian_bytes() {
    let mut w = BitWriter::new();
    w.write(0x12345678, 32);
    assert_eq!(w.bit_len(), 32);
    assert_eq!(w.into_bytes(), vec![0x78, 0x56, 0x34, 0x12]);
}
```

- [ ] **Step 2: Run — expect PASS**

Run: `cargo nextest run bitpack::tests::write_u32_emits_little_endian_bytes`

If it fails, suspect (a) the `value & ((1u32 << bits) - 1)` mask overflowing when `bits == 32`
(should be guarded by the `if bits == 32` branch — verify), or (b) the `value >>= take` shifting
by 32 when `take == 32` (also UB-adjacent: `>>= 32` on a `u32` is UB in C, but in Rust it panics
in debug and wraps to 0 in release — fine here because the loop exits after consuming all bits).

- [ ] **Step 3: Commit**

```bash
git add src/bitpack.rs
git commit -m "Phase 1: test 32-bit LE write"
```

### Task 1.6: TDD — write of 0 bits is a no-op, and write of 0 value works

- [ ] **Step 1: Add the failing tests**

```rust
#[test]
fn writing_zero_bits_is_a_noop() {
    let mut w = BitWriter::new();
    w.write(0xFFFF_FFFF, 0);
    assert_eq!(w.bit_len(), 0);
    assert_eq!(w.into_bytes(), Vec::<u8>::new());
}

#[test]
fn writing_zero_value_advances_position() {
    let mut w = BitWriter::new();
    w.write(0, 8);
    assert_eq!(w.bit_len(), 8);
    assert_eq!(w.into_bytes(), vec![0x00]);
}
```

- [ ] **Step 2: Run — expect PASS**

Run: `cargo nextest run bitpack`

- [ ] **Step 3: Commit**

```bash
git add src/bitpack.rs
git commit -m "Phase 1: test zero-width and zero-value writes"
```

### Task 1.7: TDD — high bits of value above `bits` are discarded

- [ ] **Step 1: Add the failing test**

```rust
#[test]
fn high_bits_above_width_are_discarded() {
    let mut w = BitWriter::new();
    // 0xFF requested at 4 bits should emit only 0xF (low nibble).
    w.write(0xFF, 4);
    assert_eq!(w.bit_len(), 4);
    assert_eq!(w.into_bytes(), vec![0x0F]);
}
```

- [ ] **Step 2: Run — expect PASS**

Run: `cargo nextest run bitpack::tests::high_bits_above_width_are_discarded`

- [ ] **Step 3: Commit**

```bash
git add src/bitpack.rs
git commit -m "Phase 1: test that bits above the requested width are masked off"
```

### Task 1.8: TDD — round-trip property test against a hand-rolled reference reader

A property test catches regressions the hand-picked tests above don't. We hand-roll the matching
reader inside the test module — it's not part of the public API (Phase 7+ will pull in a real
reader from `lewton` for setup-header validation), but it's the smallest possible cross-check.

- [ ] **Step 1: Add a `arbitrary` dev-dependency**

(skipped — we'll do property testing without a crate, by iterating over a deterministic set of
seeds. Adding `proptest` here is YAGNI for a 50-LOC component.)

- [ ] **Step 2: Add the failing test**

Append to the `tests` module:

```rust
/// LSB-first bit reader — only used inside this test module to round-trip
/// against `BitWriter`. Not part of the public API.
struct BitReader<'a> {
    bytes: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, bit_pos: 0 }
    }

    fn read(&mut self, bits: u32) -> u32 {
        let mut acc: u32 = 0;
        for i in 0..bits {
            let byte = self.bytes[self.bit_pos / 8];
            let bit = (byte >> (self.bit_pos % 8)) & 1;
            acc |= (bit as u32) << i;
            self.bit_pos += 1;
        }
        acc
    }
}

#[test]
fn round_trip_against_hand_rolled_reader() {
    // A deterministic mix of widths and values that exercises every byte
    // boundary alignment.
    let cases: Vec<(u32, u32)> = vec![
        (0,            0),
        (0b1,          1),
        (0b101,        3),
        (0xA,          4),
        (0x5A,         8),
        (0x1234,       16),
        (0xDEADBEEF,   32),
        (0b1,          1),
        (0b11_1111_1111, 10),
        (0,            7),
        (0xFFFF_FFFF,  32),
        (0xFF,         4),     // tests masking
    ];

    let mut w = BitWriter::new();
    let mut total_bits = 0u32;
    for (v, b) in &cases {
        w.write(*v, *b);
        total_bits += *b;
    }
    assert_eq!(w.bit_len(), total_bits as usize);

    let bytes = w.into_bytes();
    let mut r = BitReader::new(&bytes);
    for (v, b) in &cases {
        let got = r.read(*b);
        // Only the low `b` bits of `v` should round-trip; high bits are dropped on write.
        let expected = if *b == 32 { *v } else { *v & ((1u32 << b) - 1) };
        assert_eq!(
            got, expected,
            "round-trip mismatch for write({v:#x}, {b}): got {got:#x}, expected {expected:#x}"
        );
    }
}
```

- [ ] **Step 3: Run — expect PASS**

Run: `cargo nextest run bitpack::tests::round_trip`

If it fails, the most likely culprit is the `write` impl's `value >>= take` line shifting by
the full width of `value` when `take == 32`, which panics in debug. Add a `if take == 32 { break; }`
short-circuit before the shift to guard.

- [ ] **Step 4: Commit**

```bash
git add src/bitpack.rs
git commit -m "Phase 1: round-trip property test for BitWriter"
```

### Task 1.9: Verify, push, watch CI

- [ ] **Step 1: `just verify`**

Expected: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo nextest run` all green. The bitpack tests should be the only tests reported.

- [ ] **Step 2: `cargo build --target wasm32-unknown-unknown --release`**

Expected: clean build.

- [ ] **Step 3: Push**

```bash
git push origin main
```

- [ ] **Step 4: `gh run watch`** (or `gh run list --limit 1` if non-interactive)

Expected: both CI jobs green.

**Phase 1 gate:**
- All bitpack tests pass under `cargo nextest run`
- `just verify` exits 0
- `cargo build --target wasm32-unknown-unknown --release` exits 0
- CI is green on the latest push

---

## Self-Review

**Spec coverage (against §1, §4.1, §5, §6 of `README.md`):**
- §4.1 file `bitpack.rs` → Task 1.1–1.8 ✓
- §5.1 Cargo.toml → Task 0.1 ✓
- §5.2 rust-toolchain.toml → Task 0.1 ✓
- §5.3 justfile → Task 0.1 ✓
- §5.4 .gitignore → Task 0.1 ✓
- §5.5 .githooks/pre-commit → Task 0.2 ✓
- §5.6 .github/workflows/ci.yml → Task 0.2 (with documented wasm-pack omission) ✓
- §5.7 nextest.toml → Task 0.2 ✓
- §1 public API (`encode`, `SampleRate`, `Channels`) → Task 0.3 ✓
- §6 oracle parity tests → DEFERRED to Phase 9 (no parity tests yet, so the CI step is a no-op pass)
- §7 Phase 1 gate ("write N bits, read them back, assert equality") → Task 1.8 ✓

**Placeholder scan:** No "TBD"/"TODO"/"implement later" found. The lib.rs `unimplemented!("end-to-end encode is wired up in Phase 9")` is intentional, not a placeholder for this plan.

**Type consistency:**
- `BitWriter::new() → Self` consistent across tasks ✓
- `BitWriter::write(&mut self, value: u32, bits: u32)` consistent across tasks ✓
- `BitWriter::bit_len(&self) → usize` consistent across tasks ✓
- `BitWriter::into_bytes(self) → Vec<u8>` consistent across tasks ✓
- `bits_in_last: u8` invariant (`1..=8` after the first write, `0` only when `bytes.is_empty()`) — comment documents this ✓
- Visibility: `pub(crate) struct BitWriter` — not in the public API, only internal use ✓
