#!/bin/bash
# The core crate's tests under Miri (docs/testing.md), at the generic kernel level: the
# library's unit tests and the integration tests that convert little audio. Run from the
# repository root with a nightly that has Miri:
#
#     RUSTUP_TOOLCHAIN=nightly-2026-11-01 .github/scripts/miri-core.sh
#
# Tests too slow under Miri (it interprets every operation, about a thousand times slower
# than native code) are left out: in the integration tests with
# #[cfg_attr(miri, ignore = "...")] beside the test, and the library's kernel proofs and
# one integration test below with --skip. All of them run natively on every pull request,
# at every SIMD level.
#
# With an argument, only that part runs, so CI can run the parts as separate jobs (a whole
# run took 47 minutes on one core at 0.2.0; the CI limit for a job is 45):
#
#     .github/scripts/miri-core.sh [all | lib | containers | flac-frames]
#
# Each part prints how long it took ("miri-core: PART took N s"), for sizing the jobs.
set -euo pipefail

part=${1:-all}
case "$part" in
    all | lib | containers | flac-frames) ;;
    *) echo "usage: $0 [all | lib | containers | flac-frames]" >&2; exit 2 ;;
esac
want() { [[ $part == all || $part == "$1" ]]; }
timed() {
    local name=$1 t0=$SECONDS
    shift
    "$@"
    echo "miri-core: $name took $((SECONDS - t0)) s"
}

export DSD2PCM_SIMD=generic
# Miri isolates the program from the host environment; let the level through.
export MIRIFLAGS="${MIRIFLAGS:--Zmiri-env-forward=DSD2PCM_SIMD}"

# Unit tests of the library. Skipped, each 5 minutes or more under Miri, all arithmetic on
# long inputs or every table with no unsafe code or I/O (skipped here rather than marked
# in the sources, so the kernels' files stay as they are):
# - decimator::tests::lanes_match_ordered: every lane layout against the ordered kernels;
# - decimator::tests::s1_chunks_match_bytes: every stage-1 chunking of long inputs;
# - decimator::tests::sums_are_exact: the exactness proofs over every table;
# - decimator::tests::plans_match_reference and precise_plans_match_the_design: every
#   plan against the integer reference, over an hour each;
# - filters::pysacd::tests::floats_match_the_reference_at_every_simd_level and
#   render_matches_the_stream: seconds of audio through pysacd's filter.
if want lib; then
    timed lib cargo miri test --locked -p dsd2pcm --lib -- \
        --skip decimator::tests::lanes_match_ordered \
        --skip decimator::tests::s1_chunks_match_bytes \
        --skip decimator::tests::sums_are_exact \
        --skip decimator::tests::plans_match_reference \
        --skip decimator::tests::precise_plans_match_the_design \
        --skip filters::pysacd::tests::floats_match_the_reference_at_every_simd_level \
        --skip filters::pysacd::tests::render_matches_the_stream
fi

# Integration tests: the containers' parsers and writers, FLAC, the frames API's argument
# checks. Not run: identity, pysacd, progress and frames_alloc, which convert seconds of
# audio in every test, and soak. Skipped: output_counts_past_32_bits_do_not_panic, which
# converts a whole 1 MiB block (3 seconds of DSD64) with two filters before the error it
# checks for, over 50 minutes.
if want containers; then
    timed containers cargo miri test --locked -p dsd2pcm --test containers -- \
        --skip output_counts_past_32_bits_do_not_panic
fi
if want flac-frames; then
    for t in flac frames; do
        timed "$t" cargo miri test --locked -p dsd2pcm --test "$t"
    done
fi
