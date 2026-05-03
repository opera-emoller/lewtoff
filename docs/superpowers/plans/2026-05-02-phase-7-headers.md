# Phase 7 — Headers Implementation Plan

> No worktrees. Work on `main`.

**Goal:** Emit the three Vorbis header packets (id, comment, setup) byte-identical to what `ffmpeg -c:a libvorbis -q:a 5` writes for the supported (rate × channels) input space.

**Important README correction:** README §2 says the comment header should use a fixed vendor string `lewtoff <version>`. **This is incompatible with the byte-identical mission.** ffmpeg-libvorbis writes the libvorbis vendor string `Xiph.Org libVorbis I 20200704` (the libvorbis 1.3.7 release date as a string). To match it byte-for-byte, lewtoff must emit the exact same vendor string. We do that.

The constraint isn't "lewtoff is honestly identifiable in the file"; the constraint is "the file is byte-identical to ffmpeg's output". We honor the explicit mission over the field-level recommendation.

**Architecture:**
- `src/headers.rs` — three functions: `write_id_header(rate, channels, w)`, `write_comment_header(w)`, `write_setup_header(w)`.
- Headers are written via the existing `BitWriter` (LSB-first) and the resulting bytes match the wire format.
- The setup header bytes are `Q5_SETUP_BLOB` from Phase 3.
- The id and comment headers are constructed per Vorbis I §4.2.

**Verification (this phase HAS a real byte-equality gate):**
- Extend `tools/gen-setup-blob` to also dump packet 0 (id header) and packet 1 (comment header) for each (rate, channels) combo we support: `mono44`, `mono48`, `stereo44`, `stereo48`. Commit them as `tests/vectors/headers/id_<combo>.bin` and `tests/vectors/headers/comment_<combo>.bin`.
- Add `tests/headers.rs` that constructs the headers via our code and asserts byte-equality with the committed reference vectors.

This is the first phase since Phase 2 with a per-phase byte-equality gate, and it's free because the reference vectors come from the same ffmpeg invocation we already automate.

**Source of truth:**
- Vorbis I §4.2.2 (id header), §4.2.3 (comment header)
- `~/Documents/src/libvorbis/lib/info.c::vorbis_pack_info` (id), `vorbis_pack_comment` (comment) — for cross-reference

---

## Tasks

### Task 7.1: Extend gen-setup-blob to dump id + comment headers

- [ ] **Step 1: Modify `tools/gen-setup-blob/src/main.rs`** to:
  1. Accept a `--combo <mono44|mono48|stereo44|stereo48>` arg, defaulting to `mono44` (the original behavior produces the setup blob for any combo, since setup is rate/channel-independent at Q5; but id/comment ARE per-combo).
  2. For each invocation, write three files: `tests/vectors/headers/id_<combo>.bin`, `tests/vectors/headers/comment_<combo>.bin`, and `src/setup_blob.bin` (last one only on `mono44`, as it's the canonical setup-extraction path).

  Or (cleaner): add a separate binary `tools/gen-header-vectors` that does only id + comment dumping for all 4 combos.

  Implementer's choice. **Either is fine** — pick whichever produces less awkward code.

- [ ] **Step 2: Run `just regen-setup-blob` (or equivalent recipe).** Verify 8 header bin files exist in `tests/vectors/headers/`.

- [ ] **Step 3: Commit** the new binary tool + 8 reference header files.

### Task 7.2: Construct id + comment headers in Rust

- [ ] **Step 1: Create `src/headers.rs`** with:

  ```rust
  use crate::bitpack::BitWriter;
  use crate::setup_blob::Q5_SETUP_BLOB;
  use crate::{Channels, SampleRate};

  /// libvorbis 1.3.7 vendor string. Required for byte-identical output to
  /// ffmpeg -c:a libvorbis -q:a 5.
  const VENDOR: &[u8] = b"Xiph.Org libVorbis I 20200704";

  pub(crate) fn write_id_header(rate: SampleRate, channels: Channels, w: &mut BitWriter) {
      // Per Vorbis I §4.2.2:
      // 0x01 (packet type), "vorbis" (sync), then:
      //   vorbis_version: u32 = 0
      //   audio_channels: u8
      //   audio_sample_rate: u32
      //   bitrate_maximum: i32
      //   bitrate_nominal: i32
      //   bitrate_minimum: i32
      //   blocksize_0: u4 = 11 (we use long blocks only, so blocksize_0 == blocksize_1 == 11)
      //   blocksize_1: u4 = 11
      //   framing_flag: 1 bit = 1
      // ...write each via BitWriter::write
  }

  pub(crate) fn write_comment_header(w: &mut BitWriter) {
      // Per Vorbis I §4.2.3:
      // 0x03 (packet type), "vorbis" (sync), then:
      //   vendor_length: u32 = VENDOR.len() as u32
      //   vendor_string: VENDOR.len() bytes
      //   user_comment_list_length: u32 = 0
      //   framing_flag: 1 bit = 1
  }

  pub(crate) fn write_setup_header(w: &mut BitWriter) {
      // The packet IS Q5_SETUP_BLOB byte-for-byte (already starts with 0x05 "vorbis" sync).
      // We emit it as raw bytes via repeated 8-bit writes.
      for &b in Q5_SETUP_BLOB {
          w.write(b as u32, 8);
      }
  }
  ```

  Important details:
  - The blocksize values in id header are stored as their `log2`. Q5 uses 2048 = 1<<11, so the field value is 11. Both blocksize_0 and blocksize_1 are 11 since we don't do block-switching.
  - Bitrates: ffmpeg-libvorbis at -q:a 5 typically writes `bitrate_max=0`, `bitrate_nom=0`, `bitrate_min=-1` (or similar). **Compare against the dumped reference bytes** to determine the exact values; don't guess.
  - The packet type byte (0x01, 0x03, 0x05) and "vorbis" sync are bytewise — write each as 8 bits.

- [ ] **Step 2: Wire `mod headers;` into `src/lib.rs`** with `#[allow(dead_code)]`.

- [ ] **Step 3: `cargo build` clean.**

### Task 7.3: Byte-equality tests

- [ ] **Step 1: Create `tests/headers.rs`:**

  ```rust
  use lewtoff::{Channels, SampleRate};
  // Use re-exports or doc(hidden) pub mod to access write_id_header etc.

  fn header_bytes(constructor: impl FnOnce(&mut lewtoff::headers::BitWriter)) -> Vec<u8> {
      let mut w = lewtoff::bitpack::BitWriter::new();
      constructor(&mut w);
      w.into_bytes()
  }

  #[test]
  fn id_header_mono_44100_matches_ffmpeg() {
      let mut w = /* construct */;
      lewtoff::headers::write_id_header(SampleRate::Hz44100, Channels::Mono, &mut w);
      let actual = w.into_bytes();
      let expected = include_bytes!("vectors/headers/id_mono44.bin");
      assert_eq!(actual.as_slice(), expected.as_slice());
  }

  // ... similar for mono48, stereo44, stereo48
  // ... similar for comment header (4 tests, one per combo — though comment is rate/channel-independent, the test suite validates that)
  // ... similar for setup header (just 1 test — Q5_SETUP_BLOB matches packet 2)
  ```

  You'll need to expose `bitpack::BitWriter` and `headers` to integration tests via `#[doc(hidden)] pub mod ...;` (same pattern as `mdct`).

- [ ] **Step 2: All 9-ish header tests pass byte-identically.** If a test fails, the diff between your constructed bytes and the reference will show exactly which field is wrong — fix in place.

- [ ] **Step 3: Commit** as `Phase 7: id/comment/setup header construction + byte-equality tests`.

### Task 7.4: Push, verify CI

- [ ] All previous tests pass + new header tests
- [ ] `just verify` exits 0
- [ ] `cargo build --target wasm32-unknown-unknown --release` exits 0
- [ ] `git push origin main`
- [ ] CI green

**Phase 7 gate:**
- All header byte-equality tests pass (this is a real byte-equality gate, not a deferred one)
- `Q5_SETUP_BLOB` continues to round-trip identically as packet 2
- CI green

---

## Self-Review

Spec coverage (against README §7 Phase 7):
- "ID and comment are trivial fixed-format" → Tasks 7.1, 7.2 ✓
- "Setup is the embedded Q5 blob" → already in src/setup_blob.rs (Phase 3) ✓
- "Re-verify the blob round-trips through `lewton`'s setup-header decoder" → Phase 3's lewton cross-check covers this ✓
- "emitted three-header packet sequence matches a stripped ffmpeg output" → Task 7.3 byte-equality ✓
