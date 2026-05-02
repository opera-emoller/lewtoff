# lewtoff

A pure-Rust Ogg Vorbis **encoder**, byte-identical to `libvorbis` 1.3.7 at quality Q5. Named after [lewton](https://github.com/RustAudio/lewton) — the pure-Rust Vorbis *decoder*. lewtoff is the encoder counterpart that lewton (and Mozilla bug 1446654) explicitly considered "infeasible." The spike in this directory shows it isn't.

This document is everything you need to start the project in a fresh repo. It's written for an LLM picking up cold — it includes findings, design, repo conventions, oracle setup, implementation plan, and ready-to-paste templates. Read it top-to-bottom.

---

## 1. Mission

Produce a Rust crate `lewtoff` whose encoder output is **byte-for-byte identical** to `ffmpeg -c:a libvorbis -q:a 5` (libvorbis 1.3.7) for the constrained input space below. Byte-identical bytes mean ±0 PCM after decode — so the test gate is a `cmp` against the oracle, not a fidelity threshold.

## 2. Constrained input space (everything else is out of scope)

| Constraint | Value |
|---|---|
| Sample format | Interleaved `i16` PCM |
| Sample rates | **44100 Hz or 48000 Hz only** |
| Channels | **Mono (1) or Stereo (2) only** |
| Quality | **Q5 only** (≈160 kbps stereo, ≈80 kbps mono) |
| Block-switching | **Long blocks only** (mode 0, n=2048) |
| Coupling | **None** (left/right, no mid/side) |
| Floor type | **Floor 1 only** (no floor 0 / LSP) |
| Comments | Fixed vendor string `lewtoff <version>`, zero user comments |

Public API:
```rust
pub fn encode(samples: &[i16], rate: SampleRate, channels: Channels) -> Vec<u8>;
pub enum SampleRate { Hz44100, Hz48000 }
pub enum Channels { Mono, Stereo }
```
One function. Two enums. No `Result` (input space is closed by construction). No streaming. No `Write` trait.

## 3. Spike findings (already validated — do not redo unless changing platform)

### 3.1 What we proved

On **macOS arm64** (Apple Silicon, clang + rustc 1.95.0):

| Question | Verdict | Evidence |
|---|---|---|
| Does Rust f32 arithmetic produce identical results to C f32 arithmetic for libvorbis's `mdct_forward`? | **YES — 0 ulp diff across all 1024 output samples** | `spike/c/harness.c` + `spike/rust/src/main.rs` |
| Does Rust `f64::cos`/`f64::sin` match Apple libm bit-for-bit on the trig-table input domain? | **YES — 0 ulp diff across all 2560 trig entries** | `spike/rust/src/bin/libm_trig.rs` |

### 3.2 What this means

Bit-exact replication of libvorbis encoder output in pure Rust **is achievable on the dev platform**. There were no FMA-contraction surprises, no rounding-mode issues, no libm divergence. Rust's `f64::cos` on Apple Silicon resolves into the same `libsystem_m` libvorbis links against.

The spike took ~330 LOC of ported butterfly arithmetic plus ~50 LOC of trig generation, both ported on first try. Extrapolating to the full encoder path (`floor1.c` + `res0.c` + `psy.c` + `codebook.c` + `lookup.c` + Q5 setup tables = ~5000 LOC of dense C), expect the project to take ~8–12k LOC of Rust including tests.

### 3.3 What the spike does NOT cover (real risks for the new repo)

1. **Cross-platform libm.** Apple libm `cos` ≠ glibc `cos` ≠ MSVC `cos` ≠ Rust `libm` crate (used on `wasm32-unknown-unknown`). Each target produces ulp-level drift in transcendentals. **Mitigation**: bake a precomputed trig/window/bark table at build time from a small Rust generator that runs on the build host. At runtime, no `cos`/`sin`/`log`/`exp` calls — only table lookups + arithmetic. This makes the encoder fully deterministic across all targets including `wasm32-unknown-unknown`. Validate this once on Linux x86_64 and once on `wasm32-unknown-unknown` before going far.
2. **Larger float kernels not yet spiked.** MDCT is the simplest. The psymodel (`psy.c`) uses `exp`/`log`/`pow` — these have wider ulp tolerances than `cos`/`sin` and are more likely to drift between libm implementations. Same mitigation (precomputed tables) applies, but verify before committing.
3. **Compiler version drift.** rustc 1.95.0 + clang on macOS match today. Pin both, and if either version moves, re-run the spike.

### 3.4 How to reproduce the spike (sanity check on a new machine)

The full spike code is in `spike/`. To run:

```bash
# Clone libvorbis once
git clone --depth=1 https://github.com/xiph/vorbis.git ~/Documents/src/libvorbis

# Build + run C harness (writes input.bin, trig.bin, bitrev.bin, scale.bin, c_output.bin)
cd spike/c
clang -O2 -ffp-contract=off -o harness harness.c \
  ~/Documents/src/libvorbis/lib/mdct.c \
  -I~/Documents/src/libvorbis/lib \
  -I~/Documents/src/libvorbis/include \
  -I/opt/homebrew/include \
  -lm
./harness

# Build + run Rust port (reads C-generated tables, writes rust_output.bin, diffs)
cd ../rust
cargo build --release
./target/release/mdct-spike      # → "VERDICT: BIT-EXACT match"
./target/release/libm_trig       # → "VERDICT: trig table BIT-EXACT"
```

Expected output ends with `VERDICT: BIT-EXACT match` (mdct-spike) and `VERDICT: trig table BIT-EXACT` (libm_trig). If either diverges, **stop** — investigate before doing anything else.

---

## 4. Architecture

### 4.1 Module breakdown

```
crates/lewtoff/src/
├── lib.rs              -- public API (encode + enums) only
├── tables/             -- generated tables, no runtime transcendentals
│   ├── mod.rs
│   ├── trig.rs         -- include!() of build-script-generated tables
│   ├── window.rs       -- vorbis window: y = sin(0.5*PI*sin^2(x))
│   └── bark.rs         -- bark scale + masking-curve tables
├── bitpack.rs          -- LSB-first bit writer (~50 LOC)
├── mdct.rs             -- forward MDCT 2048 (port of libvorbis lib/mdct.c)
├── floor1.rs           -- floor 1 fitter + encoder (port of lib/floor1.c)
├── residue.rs          -- residue 2 (stereo) + residue 0 (mono) (port of lib/res0.c)
├── codebook.rs         -- VQ encode + Huffman pack (port of lib/codebook.c)
├── psy.rs              -- psymodel (port of lib/psy.c) — masking thresholds
├── headers.rs          -- id, comment, setup header packets
├── setup_blob.rs       -- the Q5 setup tables, transcribed from lib/modes/setup_44.h
├── ogg_pages.rs        -- ogg page framing + CRC (or use the `ogg` crate)
└── encode.rs           -- end-to-end orchestration

build.rs                -- generates src/tables/{trig,window,bark}.rs at build time
```

Why a build script generates the tables: see §3.3.1. The build script runs on the host (Apple/Linux/Windows libm), so its output is stable per build host — but importantly, the generated tables get checked into the binary, so the **runtime** path is host-independent.

Alternative: pre-compute once with a `tools/gen-tables` binary, commit the generated `.rs` files. Either works — the build-script form keeps the tables in sync with the source if a constant changes; the committed form makes the build hermetic. **Recommendation: build script** for now, with the table values reproducible from a small `tools/gen-tables` binary that runs `cargo run -p gen-tables` and writes a known-bytes blob the build script verifies against. Belt-and-braces.

### 4.2 Dependencies

Runtime: **one** — the [`ogg` crate](https://crates.io/crates/ogg) (RustAudio, same author as lewton) for page framing + CRC. Nothing else. Bitpacker is hand-written.

Dev: `lewton` (decoder roundtrip), `proptest` or `arbitrary` + `arbitrary-fuzz` for fuzz, nothing else.

Build script: nothing.

## 5. Repo layout & conventions (steal these from `assetcompiler`)

The new repo should adopt these conventions verbatim. Templates are in `templates/`.

### 5.1 `Cargo.toml` (root, single-crate repo)

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

[dependencies]
ogg = "0.9"

[dev-dependencies]
lewton = "0.10"

[profile.release]
debug = "line-tables-only"

[lints.rust]
unsafe_code = "forbid"
```

### 5.2 `rust-toolchain.toml`

```toml
[toolchain]
channel = "1.95.0"
components = ["rustfmt", "clippy"]
```

### 5.3 `justfile`

```just
default:
    @just --list

# Full verify: fmt + clippy + tests + (if on macOS, oracle parity).
verify: fmt-check clippy test

# One-time dev setup.
setup: install-tools install-hooks

install-tools:
    cargo install cargo-nextest@0.9.128 --locked

install-hooks:
    git config core.hooksPath .githooks
    @echo "hooks installed: .githooks/pre-commit will run before each commit"

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --all-targets -- -D warnings

# All unit + integration tests. Uses cargo-nextest if available.
test:
    #!/usr/bin/env bash
    if command -v cargo-nextest >/dev/null 2>&1; then
      cargo nextest run --status-level=fail
    else
      echo "cargo-nextest not installed; falling back to cargo test"
      echo "install: cargo install cargo-nextest@0.9.128 --locked"
      cargo test
    fi

test-verbose:
    cargo nextest run

# Oracle parity test — requires ffmpeg with libvorbis built in.
# Encodes the same input via lewtoff and via ffmpeg, byte-diffs the output.
parity:
    cargo nextest run --features oracle parity_

# Regenerate the embedded Q5 setup-header blob by extracting it from a
# fresh ffmpeg-libvorbis encode of a 1-sample silence file.
regen-setup-blob:
    cargo run --bin gen-setup-blob

# Build the table generator and write src/tables/*.rs.
regen-tables:
    cargo run --bin gen-tables

clean:
    cargo clean
```

### 5.4 `.gitignore`

```
/target
*.log
.DS_Store
/oracle-cache
/tests/fixtures/oracle-out/*.ogg
!/tests/fixtures/oracle-out/.keep
```

### 5.5 `.githooks/pre-commit`

```bash
#!/usr/bin/env bash
set -eu
staged_rs=$(git diff --cached --name-only --diff-filter=ACMR | grep -E '\.rs$' || true)
[ -z "$staged_rs" ] && exit 0

if ! cargo fmt --all -- --check >/dev/null 2>&1; then
    echo "error: cargo fmt --check failed. Run: cargo fmt --all"
    exit 1
fi

if ! cargo clippy --all-targets -- -D warnings 2>&1; then
    echo ""
    echo "error: cargo clippy failed."
    exit 1
fi
```
Make it executable: `chmod +x .githooks/pre-commit`.

### 5.6 `.github/workflows/ci.yml` (Ubuntu + Linux libm)

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
      - name: Build for wasm32
        run: cargo build --target wasm32-unknown-unknown --release
```

### 5.7 nextest config

`.config/nextest.toml`:
```toml
[profile.default]
status-level = "fail"
final-status-level = "flaky"
slow-timeout = { period = "30s", terminate-after = 2 }
```

### 5.8 Why these specific tools

- **just** (vs Makefile): readable recipes, no tab/space pitfalls, lets contributors run `just <recipe>` without learning the project. Install via `brew install just` or `cargo install just`.
- **cargo-nextest** (vs `cargo test`): ~40% faster wall-clock and ~99% less output on green runs. Pinned (`@0.9.128 --locked`) so the dev experience doesn't drift.
- **rust-toolchain.toml** (vs floating stable): pins float-determinism guarantees. If rustc bumps libm calls or adds FMA, you want to control when that happens.
- **pre-commit hook** (vs CI-only): catches fmt/clippy locally so CI doesn't fail on trivial things. Bypassable with `--no-verify` if needed.
- **`unsafe_code = "forbid"`**: lewtoff has zero need for unsafe. Pure float math in safe Rust is the whole pitch.

---

## 6. Oracle setup (the most important section)

Oracle parity is the entire correctness story. Get this right.

### 6.1 What we're matching

`ffmpeg -c:a libvorbis -q:a 5` invocation — byte-identical output, every byte of the Ogg container and Vorbis stream.

### 6.2 Why ffmpeg (not raw libvorbis CLI)

GameMaker's pipeline uses ffmpeg-bundled libvorbis (see binary at `~/Documents/src/GameMaker/Zeus/compiler/bin/ffmpeg/macos/libvorbisenc.2.0.12.dylib`). ffmpeg is also installable in CI via `apt-get install ffmpeg` on Ubuntu. The libvorbis CLI tools (`oggenc`) exist but are less ubiquitously installable. Use ffmpeg for portability.

### 6.3 Pinning the oracle version

Different libvorbis versions produce different bytes (you've already seen this — the AssetCompiler memory entry "AUDO blob 41 OGG decoder LSB drift (library-version, may be unfixable)"). Pin to **libvorbis 1.3.7** (the latest stable, what ffmpeg ships).

In CI, `apt-get install ffmpeg` on `ubuntu-latest` installs ffmpeg with libvorbis 1.3.7 (Ubuntu 24.04 ships this). Verify in the workflow:

```yaml
- name: Verify libvorbis version
  run: |
    ffmpeg -version | grep -q "libvorbis 1.3" || { echo "Wrong libvorbis version"; exit 1; }
```

If the version drifts, the parity tests will fail loudly. That's the desired behavior — silent drift is the failure mode.

### 6.4 The parity test pattern

```rust
// tests/parity.rs
//
// Run with: cargo test --features oracle parity_
//
// Each test:
//   1. Generates deterministic i16 PCM input
//   2. Encodes with lewtoff::encode → lewtoff_bytes
//   3. Encodes with ffmpeg invocation → oracle_bytes
//   4. Asserts lewtoff_bytes == oracle_bytes (byte-equality)

#[cfg(feature = "oracle")]
fn ffmpeg_encode_q5(samples: &[i16], rate: u32, channels: u16) -> Vec<u8> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("ffmpeg")
        .args([
            "-loglevel", "error",
            "-y",
            "-f", "s16le",
            "-ar", &rate.to_string(),
            "-ac", &channels.to_string(),
            "-i", "pipe:0",
            "-c:a", "libvorbis",
            "-q:a", "5",
            "-f", "ogg",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn ffmpeg");

    let mut stdin = child.stdin.take().unwrap();
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for s in samples { bytes.extend_from_slice(&s.to_le_bytes()); }
    stdin.write_all(&bytes).unwrap();
    drop(stdin);

    let out = child.wait_with_output().expect("wait ffmpeg");
    assert!(out.status.success(), "ffmpeg failed: {:?}", out.status);
    out.stdout
}

#[cfg(feature = "oracle")]
#[test]
fn parity_silence_44100_mono() {
    let samples = vec![0i16; 44100 * 2]; // 2s silence
    let lewtoff_bytes = lewtoff::encode(&samples, lewtoff::SampleRate::Hz44100, lewtoff::Channels::Mono);
    let oracle_bytes = ffmpeg_encode_q5(&samples, 44100, 1);
    assert_eq!(
        lewtoff_bytes, oracle_bytes,
        "byte mismatch: lewtoff={} bytes, oracle={} bytes",
        lewtoff_bytes.len(), oracle_bytes.len(),
    );
}

#[cfg(feature = "oracle")]
#[test]
fn parity_sine_1khz_44100_mono() { /* ... */ }

#[cfg(feature = "oracle")]
#[test]
fn parity_white_noise_seeded_44100_stereo() { /* ... */ }
```

The `oracle` feature gates these tests so contributors without ffmpeg can still run the bulk of the suite. CI runs them; pre-commit doesn't.

### 6.5 Per-chunk diff helper

When parity tests fail, you need to know **where** in the bytestream they diverge — at which Ogg page, which Vorbis packet. Add a small diff binary:

```bash
cargo run --bin parity-diff -- input.s16le 44100 mono
```
that prints something like:
```
Page 0 (id header): match (28 bytes)
Page 1 (comment): match (43 bytes)
Page 1 (setup): match (10247 bytes)
Page 2 (audio): DIVERGE at byte 12 of packet 0
  lewtoff: 0x4d 0x2a ...
  oracle:  0x4d 0x2b ...
```

This is the parity equivalent of AssetCompiler's `harness diff` recipe.

### 6.6 Listening corpus (qualitative gate)

While parity is the quantitative gate, also keep a `tests/corpus/` directory with ~10 short SFX samples (impacts, voice grunts, ambient loops). Decode them with `lewton`, compare to the original. PSNR > 40 dB is a reasonable transparency target. This catches "the bytes match but the audio sounds wrong" failure mode (which shouldn't happen if oracle parity holds, but it's belt-and-braces).

---

## 7. Implementation plan (port order)

The libvorbis encoder path is a tall stack. Port bottom-up so each layer can be verified before depending on it. Each step has a verification gate — **don't move past a step until its gate is green**.

### Phase 0 — Repo bootstrap (1 day)

1. Create the repo with the templates from §5.
2. Copy `spike/` from this handoff into `examples/mdct-spike/` (or delete it; you've already seen it pass). Keep the harness.c around — you'll use the same pattern for every kernel below.
3. Wire CI. Confirm `cargo nextest run` passes on an empty crate.
4. **Gate**: `just verify` is green; CI is green.

### Phase 1 — bitpacker (~50 LOC, half a day)

Vorbis uses LSB-first bit packing within bytes (distinct from typical formats — Vorbis is bizarrely little-endian at the bit level). Port `lib/bitwise.c::oggpack_write` to a tiny Rust struct.

- **Gate**: round-trip property test — write N bits, read them back, assert equality. Port libvorbis's `bitwise.c` test cases verbatim.

### Phase 2 — Forward MDCT (already proven by spike, ~330 LOC)

Copy from `spike/rust/src/main.rs`. Verification harness: same as spike — generate trig table at build time, test against a libvorbis-generated reference vector.

- **Gate**: byte-identical to libvorbis on macOS arm64 AND ubuntu-latest x86_64 AND wasm32-unknown-unknown (use `wasm-pack test --node`).

### Phase 3 — Codebooks (~600 LOC port from `lib/codebook.c` + `lib/sharedbook.c`)

The Q5 codebooks are ~30 static codebooks burned into the setup header. Port the codebook *encode* path: `vorbis_book_encode`, `vorbis_book_errorv`, the VQ search.

The codebook *data* (Huffman trees, VQ entries) come from the Q5 setup header. Don't construct them — extract once from a reference ffmpeg-encoded file and embed as `&'static [u8]`. The `gen-setup-blob` binary does this.

- **Gate**: encoded individual VQ vectors match libvorbis byte-for-byte for a battery of inputs.

### Phase 4 — Floor 1 fitter + encoder (~1500 LOC port from `lib/floor1.c`)

The trickiest port. `floor1.c` does:
1. Compute log-magnitude spectrum
2. Bark-scale weighting
3. Greedy line-fitting heuristic to choose post values
4. Encode posts via codebooks

The fitter is heuristic — slight code differences cascade. Port literally, line-by-line. If you're tempted to "improve" anything, don't. The goal is byte-identical output, not better quality.

- **Gate**: encoded floor1 packet matches libvorbis byte-for-byte for the same input spectrum.

### Phase 5 — Psymodel (~3000 LOC port from `lib/psy.c`)

The masking model: for each bark band, compute the masking threshold below which residue can be quantized more aggressively. Lots of `exp`/`log`/`pow` in the C — replace with table lookups (precomputed in `tables/bark.rs`).

This is the most likely place for libm divergence. Port carefully and verify against the C reference at intermediate stages.

- **Gate**: produced masking thresholds match libvorbis bit-for-bit per bark band.

### Phase 6 — Residue 0/2 encoder (~800 LOC from `lib/res0.c`)

Partition the residue, classify each partition, encode via codebooks.

- **Gate**: encoded residue packet matches libvorbis byte-for-byte.

### Phase 7 — Headers (id, comment, setup) (~300 LOC)

ID and comment are trivial fixed-format. Setup is the embedded Q5 blob from `gen-setup-blob`. Re-verify the blob round-trips through `lewton`'s setup-header decoder.

- **Gate**: emitted three-header packet sequence matches a stripped ffmpeg output.

### Phase 8 — Ogg page framing (use the `ogg` crate)

Wire packets into pages. `ogg::PacketWriter` does the work; just feed it our packets with correct granule positions.

- **Gate**: full ogg stream byte-identical to ffmpeg for a battery of inputs (silence, sine, noise, real SFX).

### Phase 9 — End-to-end orchestration

`encode()` wires phases 1–8 together. Add the high-level invariants (block alignment, eos granule), expose the public API.

- **Gate**: every parity test in §6.4 passes on Linux + macOS + wasm32.

### Phase 10 — Listening corpus + fuzz

- Add `tests/corpus/` with real SFX content
- Add `tests/fuzz_target.rs`: random i16 → encode → lewton-decode must never panic, must roundtrip without error
- Run for ~1B iterations under `cargo fuzz`

---

## 8. Risks & open questions

| Risk | Likelihood | Mitigation |
|---|---|---|
| libm divergence on wasm32 (Rust uses pure-Rust `libm` crate, not bit-identical to system libm) | high | Precompute all transcendentals at build time; verify wasm32 parity in CI from phase 1 onward |
| psymodel `exp`/`log` divergence between glibc and Apple libm | medium | Same: precompute masking-curve tables |
| libvorbis 1.3.x version drift (Ubuntu apt vs upstream) | medium | Pin via CI assertion; test fixture file with known-good oracle bytes committed |
| Floor 1 fitter port has non-trivial control-flow bugs that produce different post sets | high | Literal line-by-line port; test against C reference at every intermediate stage |
| aoTuV is already merged into 1.3.4+, so a "1.3.7 port" implicitly includes aoTuV behavior | (informational) | No mitigation needed — confirms we get aoTuV-tuned quality "for free" by porting 1.3.7 |
| Setup-header blob format depends on libvorbis internal mode tables — extracting from ffmpeg output may include things our limited input space doesn't need | medium | The blob is opaque to lewtoff; extract it once and embed verbatim |

---

## 9. Why "fresh repo" is the right call

- **License clarity**: lewtoff is BSD-style derived (libvorbis is BSD; aoTuV is BSD). Keeping it in a separate repo with its own LICENSE makes the lineage obvious.
- **Audience separation**: lewtoff is useful to anyone needing pure-Rust Ogg Vorbis encoding. AssetCompiler is GameMaker-specific. Coupling them buries lewtoff.
- **Release cadence**: lewtoff will be stable once parity is achieved; AssetCompiler iterates fast.
- **Consumer cost**: zero. AssetCompiler adds `lewtoff = "0.1"` to its `Cargo.toml`. Pre-publish dev: `[patch.crates-io] lewtoff = { path = "../lewtoff" }`. Same workflow as any other Rust dep.

---

## 10. Reading list (in order)

1. **Vorbis I specification** — https://xiph.org/vorbis/doc/Vorbis_I_spec.html. Mandatory. Read sections 4–9 cover-to-cover.
2. **libvorbis source** — `git clone https://github.com/xiph/vorbis.git`. Especially `lib/mdct.c`, `lib/floor1.c`, `lib/res0.c`, `lib/psy.c`, `lib/codebook.c`, `lib/modes/setup_44.h`.
3. **lewton source** — https://github.com/RustAudio/lewton. The decoder. Read it as a model for how Vorbis structures map to Rust.
4. **ogg crate** — https://docs.rs/ogg. Container framing.
5. **Mozilla bug 1446654** — https://bugzilla.mozilla.org/show_bug.cgi?id=1446654. Useful context on why this hasn't been done before.

---

## 11. What's in this handoff

```
lewtoff-handoff/
├── README.md                  ← you are here
├── spike/
│   ├── c/
│   │   └── harness.c          ← C side that exercises libvorbis mdct_forward
│   └── rust/
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs        ← Rust port of mdct_forward + diff
│           └── bin/
│               └── libm_trig.rs ← libm comparison probe
└── templates/                 ← ready-to-paste files (see §5)
    ├── Cargo.toml
    ├── rust-toolchain.toml
    ├── justfile
    ├── .gitignore
    ├── pre-commit
    ├── ci.yml
    └── nextest.toml
```

Everything you need is in this directory. Good luck.
