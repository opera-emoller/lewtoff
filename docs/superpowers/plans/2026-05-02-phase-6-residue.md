# Phase 6 — Residue Implementation Plan

> No worktrees. Work on `main`.

**Goal:** Literal port of libvorbis 1.3.7's residue 0/2 encoder from `~/Documents/src/libvorbis/lib/res0.c` (886 LOC) to `src/residue.rs`. Residue 2 is what Q5 stereo uses; residue 0 is what Q5 mono uses.

**Verification strategy:** Same as Phases 4-5: compile + clippy + math primitives + setup unpack round-trip. Phase 9 parity is the ultimate gate.

**Architecture:**
- `src/residue.rs` — port of res0.c
- Extends the setup-blob unpack to also handle residue configurations (in addition to codebooks from Phase 3 and floor1 from Phase 4)
- Exposes:
  - `ResidueSetup` — decoded from setup blob
  - `Residue::forward(in: &[Vec<f32>], setup: &ResidueSetup, codebooks: &[Codebook], w: &mut BitWriter)` — encodes residue 2 (interleaved if stereo, plain if mono)

**Source of truth:**
- `~/Documents/src/libvorbis/lib/res0.c` — `res0_info_unpack`, `res0_class`, `res0_forward`, `res0_inverse_part` (skip — decoder), `res2_class`, `res2_forward` (residue 2 is interleaved; for stereo)
- `~/Documents/src/libvorbis/lib/codec_internal.h` — for `vorbis_info_residue0` struct
- Vorbis I §8 (residue) — wire format

---

## Tasks

### Task 6.1: Setup-blob extension — residue unpack

- [ ] **Step 1: Read `res0_info_unpack` in res0.c.**
- [ ] **Step 2: Define `ResidueSetup`** mirroring `vorbis_info_residue0`. Fields include partition begin/end, partition step, classifications, books, etc.
- [ ] **Step 3: Port `res0_info_unpack`** as `unpack_residue(reader: &mut BitReader) -> ResidueSetup`.
- [ ] **Step 4: Extend the Q5 setup-blob unpacker** to consume: codebooks → time-transforms placeholder → floors → residues (this task) → mappings (just the count, skip data) → modes (count, skip data). The blob ends with the framing bit.
- [ ] **Step 5: Unit tests** verifying the Q5 blob unpacks to expected shapes (e.g., 2 residues for Q5).
- [ ] **Step 6: Commit** as `Phase 6: residue setup unpack`.

### Task 6.2: Port res0_class + res0_forward + res2_*

- [ ] **Step 1: Read `res0_class`, `res0_forward`, `res2_class`, `res2_forward`.** These are the encoder entry points.
- [ ] **Step 2: Port them.** The classification step assigns each partition to a "class" based on energy. Then the forward step encodes each partition via its class's codebook.
- [ ] **Step 3: Localized clippy `#[allow(...)]`** as needed, no restructuring.
- [ ] **Step 4: Unit-test what's testable** — partition classification on a synthetic energy vector.
- [ ] **Step 5: Commit** as `Phase 6: residue forward (literal port of res0.c)`.

### Task 6.3: Push, verify CI

- [ ] All previous tests pass + new residue tests pass
- [ ] `just verify` exits 0
- [ ] `cargo build --target wasm32-unknown-unknown --release` exits 0
- [ ] `git push origin main`
- [ ] CI green

**Phase 6 gate:**
- Code compiles, clippy clean, all tests pass
- `Residue::forward` is callable with the same shape signature as Phase 4's `Floor1::forward` (BitWriter sink)
- CI green

---

## Self-Review

Spec coverage (against README §7 Phase 6):
- "Partition the residue, classify each partition, encode via codebooks" → Task 6.2 ✓
- "encoded residue packet matches libvorbis byte-for-byte" → Phase 9
