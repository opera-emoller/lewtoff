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
    cargo run --bin gen-setup-blob

# Build the table generator and write src/tables/*.rs.
regen-tables:
    cargo run --bin gen-tables

# Regenerate src/tables/trig.rs from a fresh run of tools/gen-tables.
# Must be run on a canonical host (macOS arm64) for byte-identical reproducibility.
regen-trig-table:
    cargo run -p gen-tables
    cargo fmt --all

# Headless wasm parity check (uses wasm-pack + a node runtime).
wasm-test:
    wasm-pack test --node

clean:
    cargo clean
