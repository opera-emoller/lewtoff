# Phase 5 — Psymodel Implementation Plan

> No worktrees. Work on `main`.

**Goal:** Literal port of libvorbis 1.3.7's psychoacoustic model from `~/Documents/src/libvorbis/lib/psy.c` (1213 LOC) to `src/psy.rs`. Per README §3.3.1, replace all runtime `exp` / `log` / `pow` / `sin` / `cos` calls with precomputed-table lookups so the output is deterministic across macOS Apple libm, Linux glibc, and wasm32 Rust libm.

**Verification strategy (continuing the Phase 4-8 strategy):**
1. Code compiles + types check.
2. Clippy clean (localized `#[allow(...)]`, no restructuring).
3. **Cross-platform numerical determinism is the special concern of Phase 5.** Add a unit test that computes the masking thresholds for a known synthetic spectrum and asserts byte-identical f32 output. CI runs this on Ubuntu — if Linux's f32 ops produce different bytes than macOS's, we discover here, not in Phase 9.
4. **Phase 9 end-to-end parity is the ultimate gate.**

**Architecture:**
- `src/psy.rs` — port of psy.c
- Tables in `src/tables/`:
  - `src/tables/bark.rs` — bark-scale conversion table (already mentioned by README §4.1)
  - `src/tables/atan_log.rs` — replaces runtime atan/log calls (if used)
  - Extend `tools/gen-tables` to emit these in addition to `trig.rs`
- Identify all runtime transcendentals during the port. For each, the choice is: (a) precompute as a lookup table, or (b) keep but verify `f32` op portability via the Phase 5 numerical-determinism test. Prefer (a). The README's stance: **bake all transcendentals at gen-tables time so the runtime is pure +-*/.**

**Source of truth:**
- `~/Documents/src/libvorbis/lib/psy.c`
- `~/Documents/src/libvorbis/lib/psy.h`
- `~/Documents/src/libvorbis/lib/lookup.c` (if psy.c calls into it for `vorbis_fromdBlook` etc.)
- Vorbis I spec doesn't fully constrain the psy model (it's encoder-internal), so libvorbis IS the spec here.

---

## Tasks

### Task 5.1: Audit psy.c's transcendentals

- [ ] **Step 1: Grep psy.c for `cos|sin|exp|log|pow|atan|sqrt`.** List each call site. (Note: `sqrt` is IEEE 754-specified across platforms — does NOT need a table; only the transcendentals do.)
- [ ] **Step 2: For each transcendental, decide precompute strategy:**
  - Inputs over a small fixed range → fixed-size lookup table
  - Inputs over a continuous range → coarse table + linear interp (still cross-platform-safe if the table itself is bytes-identical)
  - One-shot constants → just bake the constants, no table
- [ ] **Step 3: Extend `tools/gen-tables/src/main.rs`** to emit the additional tables required.
- [ ] **Step 4: Add `regen-bark-tables` (or extend `regen-trig-table`) to the justfile** so the developer regenerates tables in one shot.
- [ ] **Step 5: Run table generation. Commit the generated `.rs` files.**

### Task 5.2: Port psy.c

- [ ] **Step 1: Read `psy.c` and `psy.h` end-to-end.** Mark out the encoder-side functions: `_vp_psy_init`, `_vp_psy_clear`, `_vp_remove_floor`, `_vp_compute_mask`, `_vp_couple_quantize_normalize`, helpers.
- [ ] **Step 2: Define `Psy` struct** — mirrors `vorbis_look_psy`. Field-for-field.
- [ ] **Step 3: Port the init/clear functions** — Rust uses Drop for clear; the init becomes `Psy::new(setup: &PsySetup) -> Self`.
- [ ] **Step 4: Port `_vp_compute_mask` and helpers** — replace transcendentals with table lookups added in Task 5.1. Keep the dB scale, bark-band logic, masking decay etc. literally.
- [ ] **Step 5: Port `_vp_remove_floor` and `_vp_couple_quantize_normalize`** similarly.
- [ ] **Step 6: Add `psy.rs`-level `#![allow(...)]` for the same family of clippy lints Phase 4 needed** — needless_range_loop, manual_clamp, etc.
- [ ] **Step 7: `cargo build` clean.**

### Task 5.3: Numerical-determinism unit test

- [ ] **Step 1: Add `tests/psy_determinism.rs`:**
  - Construct a deterministic synthetic spectrum (e.g., `[i as f32 * 0.001 ; 1024]`).
  - Call `_vp_compute_mask` (or whatever Rust name we use).
  - Capture the output threshold array as bytes (`f32::to_le_bytes`).
  - Hard-code expected bytes (computed once on macOS arm64 by running the test in record-mode, then committed).
  
  Approach: write the test once with `assert_eq!(output_bytes, EXPECTED_BYTES)` where `EXPECTED_BYTES` is a `&[u8]` literal you fill in by running the test once in print-mode. Commit the expected.

- [ ] **Step 2: Run on macOS, get the bytes, paste them in. Push.** CI runs the same test on Ubuntu — that's the cross-platform-determinism check.

  **If CI fails this test:** it means an f32 op or table lookup IS NOT cross-platform deterministic. Likely cause: a transcendental sneaked through. Audit psy.rs for any remaining `f32::ln/exp/cos/sin/etc.` calls and replace with table lookups.

### Task 5.4: Push, verify CI

- [ ] `just verify` exits 0
- [ ] `cargo build --target wasm32-unknown-unknown --release` exits 0
- [ ] `git push origin main`
- [ ] CI green on both jobs (especially the new determinism test on Ubuntu)

**Phase 5 gate:**
- All previous tests + new psy unit tests + cross-platform determinism test pass
- CI green
- Zero runtime transcendentals in src/psy.rs (grep `cos\|sin\|exp\|ln\|log10\|powf\|powi\|atan2?` should return empty matches in src/psy.rs — only `sqrt` allowed)

---

## Self-Review

Spec coverage (against README §7 Phase 5):
- "for each bark band, compute the masking threshold" → Task 5.2 ✓
- "Lots of `exp`/`log`/`pow` in the C — replace with table lookups" → Task 5.1 ✓
- "produced masking thresholds match libvorbis bit-for-bit per bark band" → DEFERRED to Phase 9. The cross-platform determinism test in Task 5.3 verifies our OWN output is bit-stable across platforms; matching libvorbis specifically is the Phase 9 gate.

**Implementer reminder:** Phase 5 has TWO discipline failure modes:
1. **"Improve" the port** — same risk as Phase 4. Don't.
2. **Sneak a transcendental through** — easy to miss `f32::ln` somewhere and not notice until CI fails on Ubuntu (or worse, until Phase 9 parity fails on wasm). Audit aggressively in Task 5.1, re-grep before commit.
