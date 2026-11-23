#!/bin/bash
# Every cargo-fuzz target in fuzz/ for SECONDS each (default 60), one after another, from the
# committed seeds in fuzz/corpus/<target> when present. Needs a nightly toolchain as the
# default and cargo-fuzz on the PATH. A crash, leak or timeout fails the run; its input stays
# in fuzz/artifacts/<target>/ (the workflow uploads it).
#
#     .github/scripts/fuzz-smoke.sh [SECONDS]
#
# Does nothing (exit 0) when fuzz/ does not exist.
set -euo pipefail

seconds=${1:-60}
if [[ ! -f fuzz/Cargo.toml ]]; then
    echo "fuzz/ does not exist: nothing to fuzz"
    exit 0
fi
cd fuzz
# The fuzz targets: the binaries whose source is fuzz_targets/<name>.rs (fuzz/ may hold
# helper binaries too, such as a seed generator).
targets=()
while read -r t; do
    [[ -f "fuzz_targets/$t.rs" ]] && targets+=("$t")
done < <(cargo fuzz list)
[[ ${#targets[@]} -gt 0 ]] || { echo "::error::cargo fuzz list found no targets"; exit 1; }
cargo fuzz build -O
status=0
for t in "${targets[@]}"; do
    echo "::group::$t, $seconds s"
    mkdir -p "corpus/$t"
    # A copy of the seeds: the run adds its finds to the first directory, not to the repository's.
    work=$(mktemp -d)
    cp -r "corpus/$t/." "$work/"
    if cargo fuzz run -O "$t" "$work" -- -max_total_time="$seconds" -timeout=10 -rss_limit_mb=2048 -print_final_stats=1; then
        echo "$t: no failures in $seconds s"
    else
        echo "::error::fuzz target $t failed (input in fuzz/artifacts/$t/)"
        status=1
    fi
    rm -rf "$work"
    echo "::endgroup::"
done
exit $status
