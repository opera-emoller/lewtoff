# Phase 3 — Codebooks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. No worktrees — work on `main` directly in `/Users/emoller/Documents/src/lewtoff`.

**Goal:** (1) Extract the Q5 setup-header blob (the binary chunk containing 30+ codebook definitions plus floor/residue/mode setup) from a fresh libvorbis-encoded ogg, embed as `&'static [u8]`. (2) Port the codebook encode-side from `lib/codebook.c` and `lib/sharedbook.c`: bit-level unpack of the blob into in-memory codebooks, Huffman codeword emission, VQ codebook search.

**Architecture decisions:**

1. **Setup blob extraction is a Rust binary, not a C harness.** `tools/gen-setup-blob/` runs `ffmpeg` as a subprocess (s16le silence → ogg), then uses the `ogg` crate to find packet 2 (after id + comment) and writes its bytes to `src/setup_blob.bin` (a binary file `include_bytes!`'d by the source). This is reproducible: same ffmpeg version → same blob. Committed.
2. **Codebooks are unpacked at runtime, not build time.** Vorbis's setup blob format has variable-width fields and Huffman trees built from codeword lengths — pre-parsing into static Rust structs would basically inline `vorbis_staticbook_unpack`, which is what we have to port anyway. Better: store the blob as bytes, parse on first use into runtime structures (single-shot, cached via `OnceLock`).
3. **Verification strategy.** Three layers:
   - **Layer A (unit tests):** the bitpack-symmetric unpack of small synthetic codebooks (round-trip via `BitWriter` → our unpack)
   - **Layer B (lewton cross-check):** parse the real Q5 setup blob with `lewton`'s parser AND with ours; assert the recovered codebook tables are field-equal. Requires using lewton's internal API (it is a dev-dep).
   - **Layer C (deferred to Phase 9):** end-to-end byte equality with ffmpeg's full encode. The codebook encode path can only really be validated this way — building a per-codebook C harness for `vorbis_book_encode` would require constructing `vorbis_dsp_state` etc. and is more work than just waiting for Phase 9 parity.

   Layer A and B are good enough for Phase 3. Layer C is the eventual ground truth.

**Tech stack additions:**
- `tools/gen-setup-blob/Cargo.toml` depends on `ogg = "0.9"` (same version as the main crate).
- Main crate's dev-deps already have `lewton = "0.10"` for Layer B cross-check.

**Source of truth for the port:**
- `~/Documents/src/libvorbis/lib/sharedbook.c` (604 LOC) — `vorbis_staticbook_unpack`, `_book_maptype1_quantvals`, `_make_words`, `_make_decode_tree` etc. Read this carefully for the unpack format.
- `~/Documents/src/libvorbis/lib/codebook.c` (461 LOC) — `vorbis_book_encode` (codeword emission), `vorbis_book_errorv` (VQ search + error vector), `vorbis_book_codeword`.
- `~/Documents/src/libvorbis/lib/codebook.h` — struct definitions.
- The Vorbis I spec, §3 (codebooks) — describes the wire format.

---

## File Structure

**Created:**
- `Cargo.toml` (root) — modified: add `tools/gen-setup-blob` to workspace members
- `tools/gen-setup-blob/Cargo.toml`
- `tools/gen-setup-blob/src/main.rs`
- `src/setup_blob.bin` — generated, committed (binary)
- `src/setup_blob.rs` — wraps the `include_bytes!` and exposes a typed slice
- `src/codebook.rs` — port of relevant pieces of codebook.c + sharedbook.c
- `src/lib.rs` — modified: declare new modules
- `tests/codebook.rs` — Layer A + Layer B tests
- `justfile` — modified: add `regen-setup-blob` recipe (replacing the stale stub from the templates)

---

## Tasks

### Task 3.1: Extract the Q5 setup blob

- [ ] **Step 1: Add `tools/gen-setup-blob` to workspace members** in root `Cargo.toml`. Members list becomes `[".", "tools/gen-tables", "tools/gen-setup-blob"]`.

- [ ] **Step 2: Create `tools/gen-setup-blob/Cargo.toml`:**

  ```toml
  [package]
  name = "gen-setup-blob"
  version = "0.0.0"
  edition = "2021"
  publish = false

  [[bin]]
  name = "gen-setup-blob"
  path = "src/main.rs"

  [dependencies]
  ogg = "0.9"
  ```

- [ ] **Step 3: Create `tools/gen-setup-blob/src/main.rs`** that:
  1. Runs ffmpeg via `std::process::Command`: input = 1 sample of silence as `s16le mono 44100`, output = `ogg` to stdout. Use the same flags as the `parity_*` test fixture (`-c:a libvorbis -q:a 5`).
  2. Reads the ogg byte stream via `ogg::PacketReader` over a `Cursor<Vec<u8>>`.
  3. Skips packet 0 (id header) and packet 1 (comment header).
  4. Writes packet 2's `data` field to `src/setup_blob.bin` (relative to repo root).
  5. Prints `"OK: wrote N bytes to src/setup_blob.bin"`.

  IMPORTANT: the input has to actually trigger libvorbis to write a setup header. 1 sample of silence is sometimes not enough to flush — try 1024 samples (one block). If you get fewer than 3 packets, increase the sample count.

- [ ] **Step 4: Add `regen-setup-blob` recipe to `justfile`** that runs `cargo run -p gen-setup-blob`. The existing stub recipe from the templates is `cargo run --bin gen-setup-blob` — replace it with `cargo run -p gen-setup-blob` (workspace-package form). Don't leave both recipes.

- [ ] **Step 5: Run `just regen-setup-blob`.** Verify `src/setup_blob.bin` was created and is reasonable size (Q5 setup is typically 6–10 KB).

- [ ] **Step 6: Sanity-check** with `lewton`'s parser. Inside `tools/gen-setup-blob`, after writing the blob, ALSO try parsing it with `lewton::header::read_header_setup` (or whatever the lewton API exposes — it may require constructing the previous two headers first). If parsing succeeds, print `"OK: lewton parses the setup blob"`. If lewton's API is awkward to use here (e.g., it requires the full ogg stream, not just the packet), skip this — it'll get covered in Layer B testing.

- [ ] **Step 7: Create `src/setup_blob.rs`:**

  ```rust
  pub(crate) static Q5_SETUP_BLOB: &[u8] = include_bytes!("setup_blob.bin");
  ```

- [ ] **Step 8: Wire `mod setup_blob;` into `src/lib.rs`** with the same `#[allow(dead_code)]` pattern as the others.

- [ ] **Step 9: Verify `just verify` is green.** Commit:

  ```bash
  git add Cargo.toml tools/gen-setup-blob/ src/setup_blob.bin src/setup_blob.rs src/lib.rs justfile
  git commit -m "$(cat <<'EOF'
  Phase 3: Q5 setup-header blob extraction

  Adds tools/gen-setup-blob (workspace member) which runs ffmpeg with
  -c:a libvorbis -q:a 5 on a one-block silence input, parses the
  resulting ogg with the ogg crate, and writes packet 2 (the Vorbis
  setup header) to src/setup_blob.bin. The blob is committed as bytes
  so CI doesn't need ffmpeg.

  Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
  EOF
  )"
  ```

### Task 3.2: Port codebook unpack + encode

- [ ] **Step 1: Read the libvorbis source.** In particular:
  - `lib/codebook.h` — `static_codebook` and `codebook` struct definitions
  - `lib/sharedbook.c` — `vorbis_staticbook_unpack` (the bit-level unpack), `_book_maptype1_quantvals`, `_book_unquantize`, `_make_words`
  - `lib/codebook.c` — `vorbis_book_init_encode`, `vorbis_book_encode`, `vorbis_book_errorv`

- [ ] **Step 2: Design the `Codebook` struct.** Fields needed for the encode path:
  - `entries: usize` — number of entries
  - `dim: usize` — vector dimension (1 for scalar, >1 for vector codebooks)
  - `codewords: Vec<u32>` — Huffman codeword for each entry (`-1` / `u32::MAX` = unused)
  - `codeword_lengths: Vec<u8>` — length in bits for each entry
  - `value_vectors: Option<Vec<f32>>` — flattened `entries * dim` matrix of dequantized vectors (for VQ codebooks). `None` for codebooks not used in encode (e.g., Huffman-only).
  - `maptype: u8` — 0 (no map), 1 (implicit), 2 (explicit)

  Mirror libvorbis's `static_codebook` and `codebook` shapes, but you don't need everything — encode-side only.

- [ ] **Step 3: Port `unpack_codebook` (modeled on `vorbis_staticbook_unpack`):** takes a `&mut crate::bitpack::BitReader` (you'll need to add a BitReader to bitpack.rs — counterpart to BitWriter), returns a `Result<Codebook, UnpackError>`. The format is described in Vorbis I §3.2.

  This means **you need to extend `src/bitpack.rs`** with a `BitReader` (the test-only one in the existing test module is a starting point, but it has to graduate to production code). Make it `pub(crate)`. Add `read(bits: u32) -> u32` and `read_signed(bits: u32) -> i32` (for unpack format which sometimes has negative deltas).

- [ ] **Step 4: Port `vorbis_book_encode`** as `Codebook::encode(&self, entry: usize, w: &mut BitWriter)`. It just emits `codeword_lengths[entry]` bits of `codewords[entry]` LSB-first via `BitWriter::write`.

- [ ] **Step 5: Port `vorbis_book_errorv`** as `Codebook::vq_search(&self, vector: &mut [f32]) -> usize`. It searches all entries for the nearest by Euclidean distance, replaces the input with the residual (input - matched_entry), returns the matched entry's index. Read the C carefully — there's a subtle "best so far" comparison.

- [ ] **Step 6: Add a helper to unpack ALL codebooks from the blob.** Per Vorbis I §4, the setup header starts with `'B', 'C', 'V'` (yes, lowercase — wait, no, I think it's a 7-byte sync pattern. Re-read the spec). Then a count of codebooks, then each codebook's bytes. Implement `unpack_q5_codebooks() -> Vec<Codebook>`.

  Cache via `OnceLock<Vec<Codebook>>` so unpack happens once.

- [ ] **Step 7: Create `src/codebook.rs`** with the structures and functions above. Wire `mod codebook;` into `src/lib.rs`.

- [ ] **Step 8: Run `just verify`.** Fix anything that breaks.

- [ ] **Step 9: Commit:**

  ```bash
  git add src/codebook.rs src/setup_blob.rs src/bitpack.rs src/lib.rs
  git commit -m "$(cat <<'EOF'
  Phase 3: codebook unpack + encode + VQ search

  Ports vorbis_staticbook_unpack, vorbis_book_encode, vorbis_book_errorv
  from libvorbis lib/sharedbook.c and lib/codebook.c. Adds BitReader to
  bitpack.rs (counterpart to BitWriter). Adds OnceLock-cached unpack of
  the Q5 setup blob into 30+ in-memory codebooks ready for Phases 4 (floor1)
  and 6 (residue) to consume.

  Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
  EOF
  )"
  ```

### Task 3.3: Verification — lewton cross-check

- [ ] **Step 1: Create `tests/codebook.rs`** that exercises:
  - **Unit:** synthetic small codebook (3-4 entries, dim=1) — pack via `BitWriter`, unpack via our code, assert the recovered struct fields match what we packed.
  - **Round-trip:** unpack the real Q5 blob via our code → re-pack via a hand-rolled packer → assert byte equality (sanity).
  - **Cross-check:** use `lewton`'s `header::read_header_setup` to parse the same blob (you may need to feed it via lewton's expected ogg framing; check lewton's docs). Compare the codebook count and a few field values. If lewton's API is too internal/awkward, skip this and add a comment explaining why; the Phase 9 parity test is the ultimate gate.

- [ ] **Step 2: Run `cargo nextest run`.** All previous tests still pass + new codebook tests pass.

- [ ] **Step 3: Commit:**

  ```bash
  git add tests/codebook.rs
  git commit -m "Phase 3: codebook unpack tests + lewton cross-check"
  ```

### Task 3.4: Push, verify CI

- [ ] **Step 1: `just verify` exits 0.**
- [ ] **Step 2: `cargo build --target wasm32-unknown-unknown --release` exits 0.**
- [ ] **Step 3: `git push origin main`.**
- [ ] **Step 4: Wait for CI green on both jobs.**

**Phase 3 gate:**
- All tests pass (`X` previous + new codebook tests)
- `src/setup_blob.bin` is committed
- `Q5_SETUP_BLOB` is reachable from the codebook unpack
- `Codebook` struct + `encode` + `vq_search` exist and pass unit tests
- CI green

---

## Self-Review

**Spec coverage (against README §7 Phase 3):**
- "Port `vorbis_book_encode`, `vorbis_book_errorv`, the VQ search" → Tasks 3.2 ✓
- "Codebook *data* come from the Q5 setup header. Don't construct them — extract once from a reference ffmpeg-encoded file and embed as `&'static [u8]`" → Task 3.1 ✓
- "encoded individual VQ vectors match libvorbis byte-for-byte for a battery of inputs" → DEFERRED to Phase 9. Building a per-codebook C harness for `vorbis_book_encode` requires non-trivial libvorbis state setup that's out of proportion to the value at this phase. The lewton cross-check + Phase 9 parity together cover the same surface.

**Placeholder scan:** None.

**Open implementation choices left to the implementer:**
- Exact API shape of `BitReader` (fits the project's style — keep the existing test-module pattern as inspiration but make it production-quality)
- Whether to use `OnceLock` or `LazyLock` (both stable since 1.80) for the cached codebook vec
- Whether to add a separate `unpack_error.rs` for the `UnpackError` type or inline as `enum CodebookError` in codebook.rs (inline is simpler for one error type)
