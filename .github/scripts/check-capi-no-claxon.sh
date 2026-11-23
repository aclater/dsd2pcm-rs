#!/bin/bash
# The C library must hold no FLAC decoder (claxon, Apache-2.0 only): it is built without the
# `flac` feature, so its archives contain no third-party code (docs/c-api.md).
#
#     .github/scripts/check-capi-no-claxon.sh LIBRARY...
#
# Each LIBRARY (libdsd2pcm.a, libdsd2pcm.so, .dylib) is listed with nm, every symbol, defined
# or not, and fails the check if a symbol name holds "claxon" (Rust's legacy and v0 manglings
# both keep crate names readable: _ZN6claxon..., _R...6claxon...). The symbol table is read,
# not the bytes, so a file name or message that mentions claxon does not count, and a
# library without symbols (stripped) fails rather than passing unchecked: a shared library
# is checked through its full symbol table, so check it before stripping. NM names the nm
# binary (default nm; llvm-nm on macOS works too).
set -euo pipefail

nm=${NM:-nm}
[[ $# -gt 0 ]] || { echo "usage: $0 LIBRARY..." >&2; exit 2; }
status=0
for lib in "$@"; do
    syms=$("$nm" -A "$lib" 2>/dev/null) || { echo "::error::$nm cannot read $lib"; exit 1; }
    count=$(printf '%s\n' "$syms" | grep -c . || true)
    if [[ $count -lt 100 ]]; then
        echo "::error::$lib has $count symbols: stripped? Check it before stripping."
        status=1
        continue
    fi
    hits=$(printf '%s\n' "$syms" | grep -i claxon || true)
    if [[ -n "$hits" ]]; then
        printf '%s\n' "$hits" | head -n 20
        echo "::error::$lib holds $(printf '%s\n' "$hits" | wc -l) claxon symbols (above, at most 20): built with the flac feature?"
        status=1
    else
        echo "$lib: $count symbols, none from claxon"
    fi
done
exit "$status"
