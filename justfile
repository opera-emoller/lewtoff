# Common dev commands for lewtoff.
# Install just via `brew install just` or `cargo install just`.

default:
    @just --list

# Full verify: fmt + clippy + tests.
verify: fmt-check clippy test

# One-time dev setup: install dev tools and git hooks. Safe to re-run.
setup: install-tools install-hooks

install-tools:
    cargo install cargo-nextest@0.9.128 --locked

install-hooks:
    git config core.hooksPath .githooks
    @echo "hooks installed: .githooks/pre-commit will run before each commit"
    @echo "bypass with 'git commit --no-verify' (avoid)"

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --all-targets -- -D warnings

# All unit + integration tests. Uses cargo-nextest if available
# (~40% faster wall-clock and ~99% less output on green runs).
test:
    #!/usr/bin/env bash
    if command -v cargo-nextest >/dev/null 2>&1; then
      cargo nextest run --status-level=fail --no-tests=warn
    else
      echo "cargo-nextest not installed; falling back to cargo test"
      echo "install: cargo install cargo-nextest@0.9.128 --locked"
      cargo test
    fi

test-verbose:
    cargo nextest run --no-tests=warn

# Oracle parity: requires ffmpeg with libvorbis 1.3.7 on PATH.
# Encodes the same input via lewtoff and via ffmpeg, byte-diffs the output.
parity:
    cargo nextest run --features oracle --no-tests=warn parity_

# Per-chunk diff helper for parity failures.
# Usage: just parity-diff input.s16le 44100 mono
parity-diff input rate channels:
    cargo run --bin parity-diff -- {{input}} {{rate}} {{channels}}

# Regenerate the embedded Q5 setup-header blob by extracting it from a
# fresh ffmpeg-libvorbis encode of a 1-sample silence file.
regen-setup-blob:
    cargo run -p gen-setup-blob

# Regenerate tests/vectors/headers/*.bin reference files from a fresh
# ffmpeg-libvorbis encode for each supported (rate × channels) combo.
regen-header-vectors:
    cargo run -p gen-header-vectors
    cp src/setup_blob.bin tests/vectors/headers/setup.bin

# Build the table generator and write src/tables/*.rs.
regen-tables:
    cargo run --bin gen-tables

# Regenerate src/tables/trig.rs from a fresh run of tools/gen-tables.
# Must be run on a canonical host (macOS arm64) for byte-identical reproducibility.
regen-trig-table:
    cargo run -p gen-tables
    cargo fmt --all

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

# Headless wasm parity check (uses wasm-pack + a node runtime).
wasm-test:
    wasm-pack test --node

debug-dump-c:
    #!/usr/bin/env bash
    set -e
    TOOL=tools/debug-libvorbis-dump
    /usr/bin/clang -O0 -g -std=c11 \
      -I$TOOL/vendored-libvorbis \
      -I$TOOL/vendored-libvorbis/include \
      -I/opt/homebrew/include \
      $TOOL/harness.c \
      $TOOL/vendored-libvorbis/debug_dump.c \
      $TOOL/vendored-libvorbis/analysis.c \
      $TOOL/vendored-libvorbis/bitrate.c \
      $TOOL/vendored-libvorbis/block.c \
      $TOOL/vendored-libvorbis/codebook.c \
      $TOOL/vendored-libvorbis/envelope.c \
      $TOOL/vendored-libvorbis/floor0.c \
      $TOOL/vendored-libvorbis/floor1.c \
      $TOOL/vendored-libvorbis/info.c \
      $TOOL/vendored-libvorbis/lookup.c \
      $TOOL/vendored-libvorbis/lpc.c \
      $TOOL/vendored-libvorbis/lsp.c \
      $TOOL/vendored-libvorbis/mapping0.c \
      $TOOL/vendored-libvorbis/mdct.c \
      $TOOL/vendored-libvorbis/misc.c \
      $TOOL/vendored-libvorbis/psy.c \
      $TOOL/vendored-libvorbis/registry.c \
      $TOOL/vendored-libvorbis/res0.c \
      $TOOL/vendored-libvorbis/sharedbook.c \
      $TOOL/vendored-libvorbis/smallft.c \
      $TOOL/vendored-libvorbis/synthesis.c \
      $TOOL/vendored-libvorbis/vorbisenc.c \
      $TOOL/vendored-libvorbis/window.c \
      -L/opt/homebrew/lib -logg -lm \
      -o $TOOL/harness
    $TOOL/harness
    echo "dumps in /tmp/lewtoff-debug/c_*"

debug-dump-rust:
    LEWTOFF_DEBUG_DUMP=1 cargo run --bin debug-rust-dump
    echo "dumps in /tmp/lewtoff-debug/r_*"

debug-diff: debug-dump-rust
    cargo run --bin debug-diff

clean:
    cargo clean
