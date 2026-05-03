# Phase 8 — Ogg Page Framing Implementation Plan

> No worktrees. Work on `main`.

**Goal:** Wire Vorbis packets into Ogg pages with the correct stream serial number, page sequence, granule positions, and CRC. Use the `ogg` crate (already a runtime dep) and verify byte-equality of the header-only ogg stream against ffmpeg.

**Verification:** Real byte-equality gate, free given the existing `tools/gen-header-vectors` from Phase 7. Add a test that uses our `OggWriter`-equivalent to emit just the 3 header packets and assert the resulting ogg byte stream matches the first N bytes of ffmpeg's full output (where N = end of the second ogg page, typically). Audio packets and final page come in Phase 9.

**Architecture:**
- `src/ogg_pages.rs` — wraps `ogg::PacketWriter` with a lewtoff-specific API:
  - `OggStreamWriter::new(serial: u32) -> Self`
  - `write_packet(&mut self, packet_bytes: &[u8], granule: u64, end_of_stream: bool, force_page_flush: bool)`
  - `into_bytes(self) -> Vec<u8>`
- The `serial` is a stream serial number (random 32-bit). For deterministic output, **we must match ffmpeg's serial number choice** — ffmpeg picks a deterministic serial based on... actually no, ffmpeg uses a random serial by default. This is a problem.

**Serial number wrinkle:** ffmpeg writes a random serial to each ogg stream. So our parity test can't compare the serial number byte-by-byte with ffmpeg. Solutions:
1. Force ffmpeg to use a fixed serial (`-stream_serial`? not sure if it exposes this).
2. Have our parity tests skip the serial bytes when comparing.
3. Have `gen-header-vectors` and `gen-setup-blob` extract the serial number from ffmpeg's output and emit it as a side-channel; our test uses the same serial.

Solution 3 is cleanest. The serial is in bytes 14-17 (LE u32) of every page. Extract it once, write to a `serial.txt` (or a header in the bin files), and use it in our test.

**Source of truth:**
- Vorbis I §A.2 (page format)
- The `ogg` crate's `writing` module
- `~/Documents/src/libvorbis/lib/codec_internal.h` (granule-position semantics)

---

## Tasks

### Task 8.1: Extract reference ogg-header bytes

- [ ] **Step 1: Modify `tools/gen-header-vectors`** to also dump the FULL ogg bytes (just for the headers section — packets 0, 1, 2 — typically takes 2 ogg pages). Output: `tests/vectors/ogg/headers_<combo>.ogg` (4 files, one per combo).
- [ ] **Step 2: Also dump the serial number** for each combo: `tests/vectors/ogg/serial_<combo>.txt` containing the 4-byte LE serial as a hex string. Or: just an `&[u8; 4]` in a `.bin`.
- [ ] **Step 3: Run, commit.**

### Task 8.2: Implement OggStreamWriter

- [ ] **Step 1: Create `src/ogg_pages.rs`** wrapping `ogg::PacketWriter` (or `BasePacketWriter` if `PacketWriter` doesn't fit). Read `ogg` crate's docs to find the right API.
- [ ] **Step 2: API surface:**
  ```rust
  pub(crate) struct OggStreamWriter { /* ... */ }
  impl OggStreamWriter {
      pub fn new(serial: u32) -> Self;
      pub fn write_packet(&mut self, bytes: &[u8], granule: u64, eos: bool, force_flush: bool);
      pub fn into_bytes(self) -> Vec<u8>;
  }
  ```
- [ ] **Step 3: Match ffmpeg's page-flush behavior.** ffmpeg flushes a page after the id header (forces page boundary after packet 0). Comment + setup go in page 1. Audio packets aggregate. Final page is forced.

### Task 8.3: Byte-equality test for the headers-only stream

- [ ] **Step 1: Add `tests/ogg_headers.rs`:**
  ```rust
  #[test]
  fn ogg_headers_mono44_match_ffmpeg() {
      let serial = u32::from_le_bytes(*include_bytes!("vectors/ogg/serial_mono44.bin"));
      let mut w = OggStreamWriter::new(serial);
      
      // packet 0: id header
      let mut bw = BitWriter::new();
      headers::write_id_header(SampleRate::Hz44100, Channels::Mono, &mut bw);
      w.write_packet(&bw.into_bytes(), 0, false, true);  // force page break
      
      // packet 1: comment header
      let mut bw = BitWriter::new();
      headers::write_comment_header(&mut bw);
      w.write_packet(&bw.into_bytes(), 0, false, false);
      
      // packet 2: setup header
      let mut bw = BitWriter::new();
      headers::write_setup_header(&mut bw);
      w.write_packet(&bw.into_bytes(), 0, false, true);  // force page break to seal page 1
      
      let actual = w.into_bytes();
      let expected = include_bytes!("vectors/ogg/headers_mono44.ogg");
      assert_eq!(actual.as_slice(), expected.as_slice(), "ogg header bytes diverged");
  }
  // ... 3 more for the other combos
  ```
- [ ] **Step 2: All 4 ogg-header tests pass byte-identically.**

### Task 8.4: Push, verify CI

- [ ] All previous + new tests pass
- [ ] `just verify` exits 0
- [ ] `cargo build --target wasm32-unknown-unknown --release` exits 0
- [ ] `git push origin main`
- [ ] CI green

**Phase 8 gate:**
- 4 ogg-header byte-equality tests pass (real gate)
- `OggStreamWriter` is ready for Phase 9 to feed audio packets through it
- CI green
