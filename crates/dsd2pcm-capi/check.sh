#!/bin/sh
# SPDX-License-Identifier: MIT OR Apache-2.0
# Checks of the C interface, run from the repository root:
#   crates/dsd2pcm-capi/check.sh
# 1. include/dsd2pcm.h is what cbindgen generates from the sources (CBINDGEN names the
#    cbindgen binary; default: cbindgen on PATH).
# 2. tests/c/smoke.c builds against the header and the static library with -Wall -Wextra
#    -Werror under gcc and clang (CCS lists the compilers), and passes; so do the examples
#    of the README and docs/c-api.md (tests/c/minimal.c, tests/c/convert.c).
# 3. The same under clang with AddressSanitizer and UndefinedBehaviorSanitizer, unless
#    ASAN=0. The sanitizer runtime is clang's shared one (-shared-libasan), preloaded.
set -eu

crate=crates/dsd2pcm-capi
cbindgen=${CBINDGEN:-cbindgen}
ccs=${CCS:-gcc clang}
target=${CARGO_TARGET_DIR:-target}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

echo "== header"
"$cbindgen" --quiet --config "$crate/cbindgen.toml" --crate dsd2pcm-capi \
    --output "$work/dsd2pcm.h"
if ! cmp -s "$work/dsd2pcm.h" "$crate/include/dsd2pcm.h"; then
    diff -u "$crate/include/dsd2pcm.h" "$work/dsd2pcm.h" || true
    echo "include/dsd2pcm.h is out of date: regenerate it with cbindgen (see cbindgen.toml)" >&2
    exit 1
fi

echo "== static library"
libs=$(cargo rustc --release -q -p dsd2pcm-capi --lib --crate-type staticlib \
    -- --print native-static-libs 2>&1 | sed -n 's/.*native-static-libs: //p' | tail -n 1)
lib="$target/release/libdsd2pcm.a"
test -f "$lib"

for cc in $ccs; do
    echo "== $cc"
    # shellcheck disable=SC2086
    "$cc" -std=c11 -O2 -Wall -Wextra -Werror -I "$crate/include" \
        "$crate/tests/c/smoke.c" "$lib" $libs -o "$work/smoke-$cc"
    "$work/smoke-$cc" "$work"
    for ex in minimal convert; do
        # shellcheck disable=SC2086
        "$cc" -std=c11 -O2 -Wall -Wextra -Werror -I "$crate/include" \
            "$crate/tests/c/$ex.c" "$lib" $libs -o "$work/$ex-$cc"
    done
    "$work/minimal-$cc"
    "$work/convert-$cc" "$work/smoke.dsf" "$work/convert.wav"
done

if [ "${ASAN:-1}" != 0 ]; then
    echo "== clang AddressSanitizer + UndefinedBehaviorSanitizer"
    rt=$(clang -print-file-name=libclang_rt.asan.so)
    if [ ! -f "$rt" ]; then
        rt=$(find "$(clang -print-resource-dir)/lib" -name 'libclang_rt.asan*.so' | head -n 1)
    fi
    # shellcheck disable=SC2086
    clang -std=c11 -O1 -g -fno-omit-frame-pointer -Wall -Wextra -Werror \
        -fsanitize=address,undefined -fno-sanitize-recover=all -shared-libasan \
        -I "$crate/include" "$crate/tests/c/smoke.c" "$lib" $libs -o "$work/smoke-asan"
    env LD_PRELOAD="$rt" ASAN_OPTIONS=detect_leaks=1:abort_on_error=1 \
        "$work/smoke-asan" "$work"
fi
echo "C interface checks passed"
