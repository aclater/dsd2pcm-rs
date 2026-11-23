#!/bin/bash
# The workspace's tests once per SIMD level this CPU has, each forced with DSD2PCM_SIMD, so
# the golden hashes and the identity tests run on every kernel. Output must be the same at
# every level (AGENTS.md, "Output stability").
#
#     .github/scripts/test-simd-levels.sh [CARGO TEST ARGS...]
#
# On x86_64 (from `rustc -vV`, or the --target in the arguments) the levels are generic, avx2
# and avx2fma; on 32-bit x86 also sse2. On Linux a level the CPU lacks fails the job instead of being
# skipped: DSD2PCM_SIMD only lowers the level, so a missing feature would run a lower one and
# pass without testing it. Elsewhere (aarch64) there is one level, generic.
set -euo pipefail

target=$(rustc -vV | sed -n 's/^host: //p')
prev=
for a in "$@"; do
    [[ "$prev" == --target ]] && target=$a
    [[ "$a" == --target=* ]] && target=${a#--target=}
    prev=$a
done

case "$target" in
    x86_64-*) levels=(generic avx2 avx2fma) ;;
    i686-* | i586-*) levels=(generic sse2 avx2 avx2fma) ;;
    *) levels=(generic) ;;
esac

if [[ ${#levels[@]} -gt 1 && -r /proc/cpuinfo ]]; then
    for f in sse2 avx2 fma; do
        grep -qw "$f" /proc/cpuinfo || { echo "::error::this CPU has no $f: cannot test every SIMD level"; exit 1; }
    done
fi

for level in "${levels[@]}"; do
    echo "::group::cargo test, DSD2PCM_SIMD=$level ($target)"
    DSD2PCM_SIMD=$level cargo test --workspace --locked "$@"
    echo "::endgroup::"
done
