#!/usr/bin/env bash
# Builds tools/oracle-encoder/oracle-encoder from main.c plus the vendored
# libvorbis source in tools/debug-libvorbis-dump/vendored-libvorbis/, with
# the same pinned compile flags the parity tests assume:
#
#     -O0 -ffp-contract=off -std=c99
#
# This makes the produced binary's output bit-deterministic across hosts:
# no FMA contraction, no aggressive inlining that might reorder f32 ops.
#
# Run from anywhere (the script resolves relative paths). Requires:
#   - clang or gcc
#   - libogg headers + library (homebrew on macOS, libogg-dev on Debian/Ubuntu)
#
# CI uses this script; locally you can re-run it after pulling new commits.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VEND="$SCRIPT_DIR/../debug-libvorbis-dump/vendored-libvorbis"

if [ ! -d "$VEND" ]; then
    echo "build.sh: cannot find vendored libvorbis at $VEND" >&2
    exit 1
fi

CC="${CC:-cc}"
CFLAGS="${CFLAGS:--O0 -ffp-contract=off -std=c99 -Wno-implicit-function-declaration}"

# Find libogg. On macOS with homebrew it lives in /opt/homebrew/opt/libogg
# (Apple Silicon) or /usr/local/opt/libogg (Intel). On Linux distros it's
# in the default include/library path once libogg-dev is installed.
OGG_INC=""
OGG_LIB=""
for prefix in /opt/homebrew/opt/libogg /usr/local/opt/libogg; do
    if [ -d "$prefix/include/ogg" ]; then
        OGG_INC="-I$prefix/include"
        OGG_LIB="-L$prefix/lib"
        break
    fi
done

SRCS=(
    "$SCRIPT_DIR/main.c"
    "$VEND/analysis.c"
    "$VEND/bitrate.c"
    "$VEND/block.c"
    "$VEND/codebook.c"
    "$VEND/debug_dump.c"
    "$VEND/envelope.c"
    "$VEND/floor0.c"
    "$VEND/floor1.c"
    "$VEND/info.c"
    "$VEND/lookup.c"
    "$VEND/lpc.c"
    "$VEND/lsp.c"
    "$VEND/mapping0.c"
    "$VEND/mdct.c"
    "$VEND/psy.c"
    "$VEND/registry.c"
    "$VEND/res0.c"
    "$VEND/sharedbook.c"
    "$VEND/smallft.c"
    "$VEND/synthesis.c"
    "$VEND/vorbisenc.c"
    "$VEND/window.c"
)

OUT="$SCRIPT_DIR/oracle-encoder"
echo "build.sh: $CC $CFLAGS -> $OUT"
$CC $CFLAGS \
    -I"$VEND/include" \
    -I"$VEND" \
    $OGG_INC \
    -o "$OUT" \
    "${SRCS[@]}" \
    $OGG_LIB -logg -lm

echo "build.sh: built $OUT"
