#!/bin/sh
# SPDX-License-Identifier: MIT OR Apache-2.0
# Regenerates src/filters/pysacd_taps.rs and tests/pysacd-ref/sha256.txt from pysacd's
# filter (pysacd/dsp.c and dsp.h from pysacd commit 94b2c53, computed in double: see
# PROVENANCE.md) built with -O2 -ffp-contract=off (no fused multiply-add, which would change
# the bits), no SIMD dispatch, no -march. Needs a C compiler and cargo fmt.
set -eu
cd "$(dirname "$0")"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
${CC:-gcc} -O2 -ffp-contract=off -std=gnu17 -Wall -I. -o "$tmp/ref" pysacd_ref.c -lm
"$tmp/ref" taps > ../../src/filters/pysacd_taps.rs
cargo fmt -p dsd2pcm
out="$tmp/sha256.txt"
: > "$out"
hash() { sha256sum < "$1" | cut -d' ' -f1; }
case_() { # kind ch rate gain tc0
  "$tmp/ref" run "$tmp/$1_$2.dsd" "$2" "$3" "$4" "$5" "$tmp/o.i32" "$tmp/o.f64"
  echo "$(hash "$tmp/o.i32")  i32_$1_$2ch_$3_$4dB_tc$5" >> "$out"
  if [ "$4" = 0 ] && [ "$5" = 0 ]; then
    echo "$(hash "$tmp/o.f64")  f64_$1_$2ch_$3" >> "$out"
  fi
}
for kind in silence ff music random; do
  for ch in 2 6; do
    "$tmp/ref" gen "$kind" "$ch" 5 "$tmp/${kind}_$ch.dsd"
    for rate in 88200 176400; do
      for gain in 0 -3 6; do
        case_ "$kind" "$ch" "$rate" "$gain" 0
      done
    done
  done
done
# Dither seeds away from frame 0, and wrapping at 2^32.
case_ music 2 88200 0 1000
case_ music 6 176400 -3 4294967293
cp "$out" sha256.txt
