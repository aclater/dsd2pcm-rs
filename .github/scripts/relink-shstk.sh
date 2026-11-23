#!/bin/bash
# x86-64 Linux with glibc: link libdsd2pcm.so again, from the static library whose objects
# .github/scripts/clib-shstk.py marked SHSTK, so that the shared library carries the CET
# shadow-stack property too (docs/c-api.md, "Intel CET"). For .github/workflows/release.yml:
#
#     .github/scripts/relink-shstk.sh LIBDIR ZIG_TARGET "STATIC_LIBS"
#
# LIBDIR holds the marked libdsd2pcm.a and gets the new libdsd2pcm.so; ZIG_TARGET is zig's
# target with the glibc version (x86_64-linux-gnu.2.17); STATIC_LIBS is rustc's
# native-static-libs list. The library exports dsd2pcm_* only, as the cdylib does, with the
# SONAME libdsd2pcm.so.1.
#
# The linker ANDs the property over its input objects. Besides the marked archive, zig
# links glibc 2.17's libc_nonshared.a, whose stat64 and fstat64 wrappers (one tail call each)
# are not marked; nothing else is. `zig cc` refuses -z shstk, its ld.lld does not: the link
# is zig's own command line (zig cc -v) run again with -z shstk, and -z cet-report lists every
# unmarked input, which must be those two wrappers and nothing else. The new library keeps
# the hardening of the one cargo built (full RELRO, BIND_NOW, no executable stack, no text
# relocations): it is linked with those flags and compared with it before it replaces it.
set -euo pipefail

libdir=$1 zig_target=$2 static_libs=$3
[[ "$static_libs" =~ ^[-[:alnum:]_.:/\ ]+$ ]] || { echo "::error::unexpected static library list: '$static_libs'"; exit 1; }
read -r -a libs <<< "$static_libs"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
printf '{\n  global: dsd2pcm_*;\n  local: *;\n};\n' > "$work/exports.map"

args=(-target "$zig_target" -shared -s -o "$work/libdsd2pcm.so"
    "-Wl,--whole-archive" "$libdir/libdsd2pcm.a" "-Wl,--no-whole-archive"
    "-Wl,-soname,libdsd2pcm.so.1" "-Wl,--version-script=$work/exports.map" "-Wl,--gc-sections"
    "-Wl,-z,relro,-z,now,-z,noexecstack,-z,text" "${libs[@]}")
cmd=$(zig cc -v "${args[@]}" 2>&1 | grep '^ld\.lld ' | tail -n 1)
[[ -n "$cmd" && "$cmd" != *[\"\'\\]* ]] || { echo "::error::no plain ld.lld command line from zig cc -v"; exit 1; }
read -r -a lld <<< "$cmd"
zig "${lld[@]}" -z shstk -z cet-report=warning 2> "$work/report.txt" \
    || { cat "$work/report.txt"; echo "::error::the ld.lld link failed"; exit 1; }
unmarked=$(grep 'does not have GNU_PROPERTY_X86_FEATURE_1_SHSTK' "$work/report.txt" || true)
echo "inputs without SHSTK:"
echo "${unmarked:-  none}"
other=$(grep -Ev 'libc_nonshared\.a\([^)]*/f?stat64-[0-9.]+\.o\)' <<< "$unmarked" || true)
[[ -z "$other" ]] || { echo "::error::inputs without the SHSTK property besides glibc's stat64 wrappers: $other"; exit 1; }
readelf -n "$work/libdsd2pcm.so" | grep -q 'x86 feature: SHSTK$' || { echo "::error::the relinked library is not marked SHSTK"; exit 1; }
n=$(nm -D --defined-only "$work/libdsd2pcm.so" | grep -cv ' dsd2pcm_' || true)
[[ "$n" -eq 0 ]] || { echo "::error::the relinked library exports $n symbols besides dsd2pcm_*"; exit 1; }
# The hardening of cargo's library, for comparison: BIND_NOW, GNU_RELRO, the stack's flags
# and TEXTREL.
hardening() {
    readelf -d "$1" | grep -Eo 'BIND_NOW|TEXTREL|Flags: .*NOW' | sed 's/Flags: .*NOW/NOW/' | sort -u | tr '\n' ' '
    readelf -lW "$1" | awk '$1 == "GNU_RELRO" {print "RELRO"} $1 == "GNU_STACK" {print "STACK=" $(NF-1)}' | tr '\n' ' '
}
old=$(hardening "$libdir/libdsd2pcm.so") new=$(hardening "$work/libdsd2pcm.so")
echo "hardening, cargo's library: $old"
echo "hardening, relinked:        $new"
[[ "$old" == "$new" ]] || { echo "::error::the relinked library's hardening differs from cargo's"; exit 1; }
[[ "$new" == *NOW* && "$new" == *RELRO* && "$new" == *"STACK=RW "* && "$new" != *TEXTREL* ]] \
    || { echo "::error::the relinked library lacks full RELRO, BIND_NOW or a non-executable stack"; exit 1; }
install -m 0755 "$work/libdsd2pcm.so" "$libdir/libdsd2pcm.so"
echo "$libdir/libdsd2pcm.so: relinked, x86 feature SHSTK, $(nm -D --defined-only "$libdir/libdsd2pcm.so" | wc -l) exports"
