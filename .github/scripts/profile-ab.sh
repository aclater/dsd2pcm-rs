#!/bin/bash
# The default release profile against one a consumer may build us with as a dependency
# (codegen units 16, no LTO, overflow checks on): the threads example built both ways, timed
# in interleaved rounds on this machine, one thread. Fails if the outputs differ or if the
# consumer build's median time is more than TOLERANCE times the default's. Prints every round
# and the spread.
#
#     .github/scripts/profile-ab.sh [ROUNDS] [TOLERANCE]    (defaults 7 and 1.05)
#
# The consumer profile once cost 25 %: overflow checks in the inner loops (docs/design.md,
# "Speed").
set -euo pipefail

rounds=${1:-7}
tol=${2:-1.05}
dir=${RUNNER_TEMP:-${TMPDIR:-/tmp}}/profile-ab
mkdir -p "$dir"
python3 -I .github/scripts/make-test-dsf.py "$dir/test.dsf"

cargo build --release --locked --example threads --target-dir "$dir/default"
CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 CARGO_PROFILE_RELEASE_LTO=false \
    CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true \
    cargo build --release --locked --example threads --target-dir "$dir/consumer"

# One thread, converters warm, 176.4 kHz standard, the 2-second file 150 times over.
run() {
    "$dir/$1/release/examples/threads" "$dir/test.dsf" ours warm 1 176400 standard 150
}

: >"$dir/times"
for ((r = 1; r <= rounds; r++)); do
    for p in default consumer; do
        line=$(run "$p")
        echo "round $r $p: $line"
        echo "$p ${line#*: }" >>"$dir/times"
    done
done

python3 -I - "$dir/times" "$tol" <<'PY'
import statistics, sys
t, h = {}, {}
for line in open(sys.argv[1]):
    p, secs, _s, *rest = line.split()
    t.setdefault(p, []).append(float(secs.rstrip(',')))
    h.setdefault(p, set()).add(rest[-1])
if h["default"] != h["consumer"] or len(h["default"]) != 1:
    sys.exit(f"output differs: {h}")
md, mc = statistics.median(t["default"]), statistics.median(t["consumer"])
ratio = mc / md
for p in ("default", "consumer"):
    v = t[p]
    print(f"{p}: median {statistics.median(v):.4f} s, range {min(v):.4f} to {max(v):.4f} s"
          f" ({(max(v) - min(v)) / statistics.median(v) * 100:.1f} %)")
print(f"consumer / default: {ratio:.3f} (tolerance {sys.argv[2]})")
if ratio > float(sys.argv[2]):
    sys.exit("the consumer profile is slower than the tolerance allows")
PY
