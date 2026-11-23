#!/bin/bash
# The rounds of .github/workflows/bench.yml, after the examples are built in release:
#
#     .github/scripts/bench-rounds.sh LOGDIR [ROUNDS] [SECONDS]
#
# Writes a 2-channel and a 6-channel test DSF (make-test-dsf.py --music), then runs
# examples/bench.rs and examples/sacdfs_path.rs (all modes) on each, ROUNDS times (default 5),
# timing scarletbook-pcm first in odd rounds and dsd2pcm first in even ones. Each run checks
# that both crates give the same bytes and fails the job if not. Logs go to LOGDIR.
set -euo pipefail

logdir=$1
rounds=${2:-5}
secs=${3:-60}
ex=target/release/examples
mkdir -p "$logdir"

{
    echo "machine: $(uname -sm)"
    if [ "$(uname -s)" = Darwin ]; then
        echo "cpu: $(sysctl -n machdep.cpu.brand_string), $(sysctl -n hw.ncpu) cpus"
    else
        echo "cpu: $(grep -m1 -E '^(model name|Model)' /proc/cpuinfo | cut -d: -f2- | sed 's/^ *//') $(nproc) cpus"
        grep -m1 -E '^CPU part' /proc/cpuinfo || true
    fi
    rustc --version
} | tee "$logdir/runner.txt"

for ch in 2 6; do
    python3 -I .github/scripts/make-test-dsf.py --music "$secs" "$ch" "$logdir/../test-${ch}ch.dsf"
done
(cd "$logdir/.." && shasum -a 256 test-2ch.dsf test-6ch.dsf 2>/dev/null || sha256sum test-2ch.dsf test-6ch.dsf) | tee -a "$logdir/runner.txt"

for r in $(seq 1 "$rounds"); do
    if [ $((r % 2)) -eq 0 ]; then export BENCH_OURS_FIRST=1; else export BENCH_OURS_FIRST=0; fi
    for ch in 2 6; do
        echo "== round $r, ${ch} ch, BENCH_OURS_FIRST=$BENCH_OURS_FIRST"
        "$ex/bench" "$logdir/../test-${ch}ch.dsf" "$secs" | tee "$logdir/bench-${ch}ch-$r.log"
        "$ex/sacdfs_path" "$logdir/../test-${ch}ch.dsf" "$secs" all | tee "$logdir/path-${ch}ch-$r.log"
    done
done
