#!/bin/bash
# The C library must hold no FLAC decoder (claxon, Apache-2.0 only): it is built without the
# `flac` feature, so its archives contain no third-party code (docs/c-api.md).
#
#     .github/scripts/check-capi-no-claxon.sh LIBRARY...
#
# Each LIBRARY (libdsd2pcm.a, libdsd2pcm.so, .dylib) is listed with nm, every symbol, defined
# or not, and fails the check if a line holds "claxon" (Rust's legacy and v0 manglings both
# keep crate names readable: _ZN6claxon..., _R...6claxon...; nm -A also prefixes each line
# with the archive member, so a member named after claxon counts too). The symbol table is
# read, not the bytes, so a message or path string that mentions claxon does not count.
#
# Exit status: 0 no claxon, 1 claxon symbols found, 2 the check could not be made. It fails
# closed: an nm error, any nm message other than "no symbols" for a member, an archive member
# that nm did not list, or a library with fewer than 100 symbols (stripped: a shared library
# is checked through its full symbol table, so check it before stripping) is status 2, never
# a pass. NM names the nm binary (default nm; GNU nm, or LLVM's, which macOS's nm is).
#
# LLVM's nm reads an object's embedded LLVM bitcode (Rust's standard library ships its
# objects with it) in place of the object's symbol table, and fails on bitcode from a newer
# LLVM than its own: Xcode's nm on Rust's compiler_builtins members gives "Unknown attribute
# kind". --no-llvm-bc makes it read the symbol table, as the linker does. GNU nm has no such
# option and never reads bitcode.
#
# nm and ar read a copy of each library under a fixed relative name, from its directory: nm
# -A prefixes each line with the path as nm got it, and MSYS2 hands a native (MinGW) program
# an absolute path rewritten to Windows form (/d/a/x.a becomes D:/a/x.a), which would then
# not match the path given here. A relative name is passed unchanged, and holds no colon.
# Tests: check-capi-no-claxon-test.sh.
set -euo pipefail
# Pipelines end in `grep PATTERN >/dev/null`, not `grep -q`: grep -q stops at the first
# match, and the writer's SIGPIPE would then fail the pipeline under pipefail.

nm=${NM:-nm}
[[ $# -gt 0 ]] || { echo "usage: $0 LIBRARY..." >&2; exit 2; }
# nm runs from the work directory: a relative NM is made absolute first.
if [[ "$nm" == */* ]]; then
    nm=$(cd "$(dirname "$nm")" && pwd)/$(basename "$nm")
fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
opts=(-A)
if "$nm" --version 2>&1 | grep LLVM > /dev/null; then
    opts+=(--no-llvm-bc)
fi
status=0
fail() { echo "::error::$*"; status=2; }
for lib in "$@"; do
    [[ -f "$lib" ]] || { fail "$lib: no such file"; continue; }
    cp "$lib" "$work/library" || { fail "cannot copy $lib"; continue; }
    rc=0
    (cd "$work" && "$nm" "${opts[@]}" library) > "$work/syms" 2> "$work/err" || rc=$?
    # The one message nm may give: a member without symbols (GNU: "nm: MEMBER: no symbols";
    # LLVM: "LIB:MEMBER: no symbols"). Anything else means a part of the library went unread.
    grep -v ': no symbols$' "$work/err" > "$work/other" || true
    if [[ $rc -ne 0 || -s "$work/other" ]]; then
        head -n 20 "$work/other"
        fail "$nm cannot read $lib (exit status $rc; its messages above, at most 20)"
        continue
    fi
    count=$(grep -c . "$work/syms" || true)
    if [[ $count -lt 100 ]]; then
        fail "$lib has $count symbols: stripped? Check it before stripping."
        continue
    fi
    # An archive: every member (but the archive's own symbol table) must be in nm's listing,
    # with symbols or as "no symbols", so that no member is skipped without a word. nm lists
    # nothing, and says nothing, for an object whose symbol table holds only file and section
    # symbols (GNU nm on Rust's empty crates): such a member is read again on its own and
    # counts as read when nm, without an error, lists no symbol in it.
    if [[ "$(head -c 7 "$lib" | tr -d '\0')" == '!<arch>' ]]; then
        (cd "$work" && ar t library) > "$work/members" || { fail "ar cannot list $lib"; continue; }
        grep -Ev '^(__\.SYMDEF.*|/|//|/SYM64/)$' "$work/members" | sort -u > "$work/want" || true
        {
            awk -v p="library:" 'index($0, p) == 1 { m = substr($0, length(p) + 1); sub(/:.*/, "", m); print m }' "$work/syms"
            sed -n 's/: no symbols$//p' "$work/err" | sed 's/.*: *//; s/.*://'
        } | sort -u > "$work/seen"
        unread=()
        while IFS= read -r m; do
            # ar p of a name that two members share gives both, which nm then cannot read: an
            # error, so the check fails closed.
            rc=0
            (cd "$work" && ar p library "$m" > member && "$nm" "${opts[@]}" member > msyms 2> merr) || rc=$?
            if [[ $rc -ne 0 || -s "$work/msyms" ]] || grep -v ': no symbols$' "$work/merr" > /dev/null; then
                unread+=("$m")
            fi
        done < <(comm -23 "$work/want" "$work/seen")
        if [[ ${#unread[@]} -gt 0 ]]; then
            printf '%s\n' "${unread[@]}" | head -n 20
            fail "$lib: ${#unread[@]} of $(wc -l < "$work/want") archive members are not in nm's listing (above, at most 20)"
            continue
        fi
    fi
    hits=$(grep -i claxon "$work/syms" || true)
    if [[ -n "$hits" ]]; then
        head -n 20 <<< "$hits"
        echo "::error::$lib holds $(wc -l <<< "$hits") claxon symbols (above, at most 20): built with the flac feature?"
        [[ $status -eq 2 ]] || status=1
    else
        echo "$lib: $count symbols, none from claxon"
    fi
done
exit "$status"
