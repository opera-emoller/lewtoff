# Phase 2 — Forward MDCT Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use `- [ ]` checkbox syntax.
>
> **No worktrees.** Work directly on `main` in `/Users/emoller/Documents/src/lewtoff`.

**Goal:** Bit-exact port of libvorbis 1.3.7 `mdct_forward` (n=2048, the only block size we need) to Rust, validated by byte-equality against committed reference vectors that were produced by libvorbis itself.

**Architecture decisions (these guide every later phase too):**

1. **Workspace, not single-crate.** Add `[workspace]` to root `Cargo.toml` with members `[".", "tools/gen-tables"]`. `gen-tables` is a host-only Rust binary that writes generated `.rs` files. Other tools (per-kernel C harnesses) live under `tools/gen-vectors-<kernel>/` and are NOT cargo packages — they're compiled by `justfile` recipes against the local `~/Documents/src/libvorbis` clone + brew libvorbis 1.3.7.
2. **Tables are committed, not build-time generated.** This is the **only** way to be cross-platform deterministic: macOS Apple libm and Linux glibc libm produce ulp-level different `cos`/`sin` values, so a `build.rs` that runs on each host produces different bytes per host. Solution: developer runs `just regen-tables` on a canonical host (macOS arm64), commits `src/tables/*.rs` as bytes; CI just consumes them. The README §4.1 hedged ("recommendation: build script") but it's wrong for this mission — committed is required.
3. **Reference vectors are committed.** `tools/gen-vectors-mdct/harness.c` links libvorbis (`-lvorbis -lvorbisenc -lm`), runs `mdct_forward` on a deterministic battery of input vectors, dumps inputs and outputs as raw `f32` bytes to `tests/vectors/mdct/*.bin`. The Rust port must reproduce the *output* bytes from the *input* bytes byte-for-byte. CI doesn't need libvorbis installed — it just runs the Rust test against the committed `.bin` files.
4. **MDCT is f32-only with no transcendentals at runtime.** All sin/cos values come from the precomputed `TRIG_2048` table. As long as we don't enable FMA contraction (Rust default: off), f32 arithmetic is IEEE 754 deterministic across macOS arm64, Linux x86_64, and wasm32-unknown-unknown. The spike already proved this end-to-end on macOS arm64; this phase extends the verification surface to Linux + wasm via CI.

**Tech stack additions:** None at the Cargo.toml [dependencies] level. `tools/gen-tables` has no deps. The C harness uses libvorbis from `~/Documents/src/libvorbis` (headers) and `/opt/homebrew/lib/libvorbis.dylib` (linkable; brew libvorbis 1.3.7).

**Source of truth for the port:** `~/Documents/src/libvorbis/lib/mdct.c` (562 LOC), specifically `mdct_forward` and its private helpers (`mdct_butterfly_first`, `mdct_butterfly_generic`, `mdct_butterflies`, `mdct_bitreverse`). Also `lib/mdct.h` for the `mdct_lookup` struct shape. We do NOT need `mdct_init` or `mdct_clear` — those exist to populate the lookup tables, which we replace with our own precomputed `src/tables/trig.rs`.

---

## File Structure

**Created:**
- `Cargo.toml` — modified: add `[workspace]` section
- `tools/gen-tables/Cargo.toml`
- `tools/gen-tables/src/main.rs`
- `src/tables/mod.rs`
- `src/tables/trig.rs` — generated, committed
- `src/mdct.rs`
- `src/lib.rs` — modified: add `mod tables;` and `mod mdct;`
- `tools/gen-vectors-mdct/harness.c`
- `tools/gen-vectors-mdct/.gitignore` (ignore compiled binary)
- `tests/vectors/mdct/input_<case>.bin` — generated, committed
- `tests/vectors/mdct/output_<case>.bin` — generated, committed
- `tests/mdct.rs` — integration test
- `justfile` — modified: add `regen-trig-table` and `regen-mdct-vectors` recipes

---

## Tasks

### Task 2.1: Workspace + tables infrastructure

- [ ] **Step 1: Convert root `Cargo.toml` to a workspace root.** Add at the top, before `[package]`:

  ```toml
  [workspace]
  members = [".", "tools/gen-tables"]
  ```

  Sanity-check: `cargo build` from repo root still succeeds.

- [ ] **Step 2: Create `tools/gen-tables/Cargo.toml`:**

  ```toml
  [package]
  name = "gen-tables"
  version = "0.0.0"
  edition = "2021"
  publish = false

  [[bin]]
  name = "gen-tables"
  path = "src/main.rs"
  ```

- [ ] **Step 3: Create `tools/gen-tables/src/main.rs`** that, when run from the repo root via `cargo run -p gen-tables`, writes `src/tables/trig.rs` containing:

  - `pub static TRIG_2048: [f32; 1024]` — the trig values libvorbis's `mdct_init` would populate for n=2048. Reference: `~/Documents/src/libvorbis/lib/mdct.c::mdct_init`. Specifically, libvorbis fills `T[0..n*2]` with `cos(2π*i/n)` and `sin(2π*i/n)` interleaved in a specific pattern — read `mdct_init` carefully and match its layout exactly. The output is whatever your Rust port's `mdct.rs` will index.
  - `pub static BITREV_2048: [u32; 256]` — the bit-reverse permutation table libvorbis builds in `mdct_init` (size `n/8`).
  - `pub static SCALE_2048: f32` — the scale factor `4.0 / n` libvorbis stores as `init->scale`.

  Use `f64::cos` / `f64::sin` to compute, then cast to `f32` for storage. The cast point is the only place ulp-level decisions matter; mirror libvorbis (which also stores `f32`).

  The generator writes one big file using `std::fs::write` with formatted output. No `quote` or `syn` deps — just `format!` into a string.

  Add a header comment at the top of the generated file: `// AUTO-GENERATED by tools/gen-tables. Do not edit. Regenerate via 'just regen-trig-table'.`

- [ ] **Step 4: Create `src/tables/mod.rs`:**

  ```rust
  pub mod trig;
  ```

- [ ] **Step 5: Add to `justfile`:**

  ```just
  # Regenerate src/tables/trig.rs from a fresh run of tools/gen-tables.
  # Must be run on a canonical host (macOS arm64) for byte-identical reproducibility.
  regen-trig-table:
      cargo run -p gen-tables
      cargo fmt --all
  ```

- [ ] **Step 6: Run `just regen-trig-table`.** Verify `src/tables/trig.rs` exists and contains the three items. Spot-check a few values: `TRIG_2048[0]` should match a known libvorbis value (compute by reading `mdct_init` for n=2048 yourself and confirming the first few entries).

- [ ] **Step 7: Wire `mod tables;` into `src/lib.rs`.** Place it next to the existing `mod bitpack;` line. The module currently has no callers, so use the same `#[allow(dead_code)]` pattern.

- [ ] **Step 8: Verify `just verify` is green.** Then commit:

  ```bash
  git add Cargo.toml tools/gen-tables/ src/tables/ src/lib.rs justfile
  git commit -m "$(cat <<'EOF'
  Phase 2: tables infrastructure

  Adds the workspace, the gen-tables Rust binary, and the committed
  precomputed trig/bitrev/scale tables for n=2048 MDCT. Tables are
  generated on the dev host (macOS arm64) and committed as bytes —
  this is the only way to be cross-platform deterministic, since
  libm cos/sin diverges by ulp between Apple/glibc/Rust-libm.

  Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
  EOF
  )"
  ```

### Task 2.2: Reference-vector C harness

- [ ] **Step 1: Create `tools/gen-vectors-mdct/harness.c`** that:

  - Includes `<vorbis/codec.h>` and `<mdct.h>` (the latter from `~/Documents/src/libvorbis/lib/mdct.h` — a private header).
  - Initializes a `mdct_lookup` with `mdct_init(&lookup, 2048)`.
  - Defines a battery of input vectors of length n=2048 (`f32[2048]`):
    - `silence`: all 0.0
    - `dc`: all 0.5
    - `impulse`: 1.0 at index 0, 0 elsewhere
    - `ramp`: `i / 2048.0` for `i in 0..2048`
    - `sine_440hz_44100`: `0.5 * sin(2π * 440 * i / 44100)` for `i in 0..2048`
    - `negative_impulse`: -1.0 at index 1024
  - For each input, runs `mdct_forward(&lookup, in, out)`, then writes:
    - `tests/vectors/mdct/input_<name>.bin` — raw input f32 bytes (8192 bytes)
    - `tests/vectors/mdct/output_<name>.bin` — raw output f32 bytes (4096 bytes — MDCT halves the size)
  - Calls `mdct_clear(&lookup)`.
  - Prints "OK: wrote N vector pairs" on success.

  Reference for compile invocation (from the original spike, README §3.4):

  ```
  clang -O2 -ffp-contract=off -o harness harness.c \
    ~/Documents/src/libvorbis/lib/mdct.c \
    -I~/Documents/src/libvorbis/lib \
    -I~/Documents/src/libvorbis/include \
    -I/opt/homebrew/include \
    -lm
  ```

  Note: `mdct.c` is statically compiled in (not linked from the dylib) so we get the exact 1.3.7 source. `-ffp-contract=off` is mandatory — it disables FMA contraction, which the spike depended on.

- [ ] **Step 2: Add to `justfile`:**

  ```just
  # Regenerate tests/vectors/mdct/*.bin from libvorbis 1.3.7.
  # Requires ~/Documents/src/libvorbis cloned and clang on PATH.
  regen-mdct-vectors:
      mkdir -p tests/vectors/mdct
      clang -O2 -ffp-contract=off -o tools/gen-vectors-mdct/harness \
          tools/gen-vectors-mdct/harness.c \
          ~/Documents/src/libvorbis/lib/mdct.c \
          -I$HOME/Documents/src/libvorbis/lib \
          -I$HOME/Documents/src/libvorbis/include \
          -I/opt/homebrew/include \
          -lm
      cd tests/vectors/mdct && $PWD/../../../tools/gen-vectors-mdct/harness
      rm tools/gen-vectors-mdct/harness
  ```

  (The `cd` makes the harness's relative-path output land in the right place; the binary is removed after to keep the working tree clean.)

- [ ] **Step 3: Add `tools/gen-vectors-mdct/.gitignore`:** containing `harness` (the compiled binary), in case the user interrupts mid-run.

- [ ] **Step 4: Run `just regen-mdct-vectors`.** Verify 12 `.bin` files in `tests/vectors/mdct/` (6 inputs × 2 = 12).

- [ ] **Step 5: Commit:**

  ```bash
  git add tools/gen-vectors-mdct/ tests/vectors/mdct/ justfile
  git commit -m "$(cat <<'EOF'
  Phase 2: MDCT reference-vector C harness

  Compiles a C tool against libvorbis 1.3.7 sources that emits raw f32
  binary input/output vectors for a battery of MDCT inputs (silence,
  dc, impulse, ramp, sine, negative impulse). The Rust port in the
  next commit must reproduce these output bytes byte-for-byte.

  Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
  EOF
  )"
  ```

### Task 2.3: Port `mdct_forward` to Rust

This is the main porting task. The libvorbis source at `~/Documents/src/libvorbis/lib/mdct.c` is the source of truth — port it **literally**, line-by-line, function-by-function. Do not "improve" anything. Do not collapse loops, do not add iterator chains, do not refactor. The goal is byte-equality, not idiomatic Rust.

- [ ] **Step 1: Create `src/mdct.rs`** with the following structure:

  ```rust
  //! Forward MDCT for n=2048, port of libvorbis 1.3.7 lib/mdct.c.
  //!
  //! Tables come from src/tables/trig.rs. No runtime transcendentals.

  use crate::tables::trig::{BITREV_2048, SCALE_2048, TRIG_2048};

  pub(crate) fn mdct_forward(input: &[f32; 2048], output: &mut [f32; 1024]) {
      // Port of mdct_forward in lib/mdct.c, specialized to n=2048.
      // ...
  }

  // Helpers, ported literally:
  // mdct_butterfly_first
  // mdct_butterfly_generic
  // mdct_butterflies
  // mdct_bitreverse
  ```

  Port the whole `mdct_forward` function plus its private helpers. `n` and `log2n` are constants (2048 and 11). All `MDCT_lookup` field accesses become array index lookups into the constants.

  **No `unsafe` allowed** (`unsafe_code = "forbid"` is enforced). All array accesses use safe indexing; the compiler will optimize bounds checks where it can. If clippy flags any (e.g., manual loop where `iter()` would be idiomatic), suppress with `#[allow(clippy::needless_range_loop)]` etc. — DO NOT rewrite the loop. Literal port discipline.

- [ ] **Step 2: Wire `mod mdct;` into `src/lib.rs`** with the same `#[allow(dead_code)]` pattern as bitpack/tables.

- [ ] **Step 3: Run `cargo build`** to confirm it compiles.

- [ ] **Step 4: Run `cargo clippy --all-targets -- -D warnings`** to confirm no warnings. Add localized allows ONLY where the literal-port-of-C code triggers a stylistic lint that would require restructuring to satisfy.

### Task 2.4: Integration test

- [ ] **Step 1: Create `tests/mdct.rs`:**

  ```rust
  use lewtoff::mdct::mdct_forward;

  fn run_case(input_bytes: &[u8], expected_output_bytes: &[u8]) {
      assert_eq!(input_bytes.len(), 2048 * 4);
      assert_eq!(expected_output_bytes.len(), 1024 * 4);

      let mut input = [0f32; 2048];
      for (i, chunk) in input_bytes.chunks_exact(4).enumerate() {
          input[i] = f32::from_le_bytes(chunk.try_into().unwrap());
      }

      let mut output = [0f32; 1024];
      mdct_forward(&input, &mut output);

      let mut actual_bytes = Vec::with_capacity(1024 * 4);
      for v in &output {
          actual_bytes.extend_from_slice(&v.to_le_bytes());
      }

      assert_eq!(actual_bytes, expected_output_bytes, "MDCT output bytes diverged");
  }

  #[test]
  fn mdct_silence() {
      run_case(
          include_bytes!("vectors/mdct/input_silence.bin"),
          include_bytes!("vectors/mdct/output_silence.bin"),
      );
  }

  #[test] fn mdct_dc() { run_case(include_bytes!("vectors/mdct/input_dc.bin"), include_bytes!("vectors/mdct/output_dc.bin")); }
  #[test] fn mdct_impulse() { run_case(include_bytes!("vectors/mdct/input_impulse.bin"), include_bytes!("vectors/mdct/output_impulse.bin")); }
  #[test] fn mdct_ramp() { run_case(include_bytes!("vectors/mdct/input_ramp.bin"), include_bytes!("vectors/mdct/output_ramp.bin")); }
  #[test] fn mdct_sine() { run_case(include_bytes!("vectors/mdct/input_sine_440hz_44100.bin"), include_bytes!("vectors/mdct/output_sine_440hz_44100.bin")); }
  #[test] fn mdct_negative_impulse() { run_case(include_bytes!("vectors/mdct/input_negative_impulse.bin"), include_bytes!("vectors/mdct/output_negative_impulse.bin")); }
  ```

- [ ] **Step 2: Make `mdct_forward` reachable from integration tests.** Integration tests (in `tests/`) only see `pub` items. Currently `mdct_forward` is `pub(crate)`, which the test can't see. Two options:
  - Promote to `pub fn mdct_forward(...)` in `src/mdct.rs` AND make the module `pub` in `lib.rs`. Pollutes the public API.
  - Add a `#[doc(hidden)] pub mod mdct;` in `src/lib.rs` so it's pub but not part of the rustdoc surface.
  
  Pick option 2 (`#[doc(hidden)] pub mod mdct;`) and `pub fn mdct_forward(...)` in the module. Same pattern for `tables` (so the test could verify table values too if needed). Update bitpack the same way for consistency? No — bitpack is purely internal, integration tests don't use it. Leave it `pub(crate)`.

- [ ] **Step 3: Run `cargo nextest run`.** Expect:
  - 9 bitpack tests still passing
  - 6 mdct tests, all passing

  If any mdct test fails: read the C source for `mdct_forward` AGAIN, compare the Rust port line-by-line, find the divergence. If you can't find it in 10 minutes, report BLOCKED with the failing test name and the first byte index that diverges (computable by diffing the actual vs expected bytes).

- [ ] **Step 4: `cargo build --target wasm32-unknown-unknown --release`** must also succeed.

- [ ] **Step 5: Commit:**

  ```bash
  git add src/mdct.rs src/lib.rs tests/mdct.rs
  git commit -m "$(cat <<'EOF'
  Phase 2: forward MDCT, byte-identical to libvorbis 1.3.7

  Literal port of mdct_forward (n=2048) from libvorbis lib/mdct.c.
  Validated against 6 committed reference vectors produced by
  libvorbis itself: silence, dc, impulse, ramp, 440Hz sine, and
  negative impulse. Byte-identical on macOS arm64; CI extends the
  surface to Linux x86_64 + wasm32-unknown-unknown.

  Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
  EOF
  )"
  ```

### Task 2.5: Push, verify CI

- [ ] **Step 1: `just verify` exits 0.**
- [ ] **Step 2: `cargo build --target wasm32-unknown-unknown --release` exits 0.**
- [ ] **Step 3: `git push origin main`.**
- [ ] **Step 4: Wait for CI.** Both `test` and `wasm` jobs must be green. The 6 mdct tests must pass on Ubuntu — that's the cross-platform determinism gate. If any mdct test fails on Ubuntu but passes on macOS, FMA contraction is the most likely cause; check `Cargo.toml` profile settings and ensure no `target-cpu=native` or `+fma` flags are sneaking in.

**Phase 2 gate:**
- All 15 tests (9 bitpack + 6 mdct) pass under `cargo nextest run`
- `just verify` exits 0 locally
- CI green on `test` and `wasm` jobs
- `tests/vectors/mdct/` has 12 committed `.bin` files
- `src/tables/trig.rs` is committed and matches what `just regen-trig-table` would produce on macOS arm64

---

## Self-Review

**Spec coverage (against README §7 Phase 2):**
- "byte-identical to libvorbis on macOS arm64 AND ubuntu-latest x86_64 AND wasm32-unknown-unknown" → Tests 2.4 + CI ✓
- "use `wasm-pack test --node`" — DEFERRED to Phase 10. Phase 2's wasm guarantee comes via CI's `cargo build --target wasm32-unknown-unknown --release` (compiles correctly) plus the assertion that the Rust port uses no transcendentals at runtime (so output is platform-deterministic). Wasm-test's actual runtime check happens in Phase 10.

**Placeholder scan:** None.

**Type consistency:**
- `mdct_forward(input: &[f32; 2048], output: &mut [f32; 1024])` — fixed-size array references, matches libvorbis n=2048 specialization
- `TRIG_2048: [f32; 1024]`, `BITREV_2048: [u32; 256]`, `SCALE_2048: f32` — sizes derived from n=2048 (n/2, n/8, scalar)
