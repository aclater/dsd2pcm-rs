#!/bin/bash
# Tests of check-capi-no-claxon.sh on small archives built here (a temporary directory) with
# the platform's C compiler, ar and nm: every case must give its exit status (0 no claxon,
# 1 claxon found, 2 the check could not be made), so the check can neither pass on a planted
# claxon symbol nor on an archive it could not read.
#
#     .github/scripts/check-capi-no-claxon-test.sh
#
# CC names the C compiler (default cc), NM the nm (default nm), as for the check.
set -euo pipefail

check=$(cd "$(dirname "$0")" && pwd)/check-capi-no-claxon.sh
cc=${CC:-cc}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failures=0

# obj NAME [EXTRA_SYMBOL]: NAME.o with 120 functions (the check wants at least 100 symbols)
obj() {
    local i
    {
        for i in $(seq 1 120); do echo "int fn_$1_$i(void) { return $i; }"; done
        [[ -z "${2:-}" ]] || echo "int $2(void) { return 0; }"
    } > "$work/$1.c"
    # shellcheck disable=SC2086 # CC may hold flags (gcc -m32)
    $cc -c -o "$work/$1.o" "$work/$1.c"
}
# lib NAME MEMBER...: libNAME.a from the members, in that order, with ar; it must hold them
# all (Apple's ar leaves out a file that is not an object, without a word)
lib() {
    local name=$1
    shift
    rm -f "$work/lib$name.a"
    (cd "$work" && ar rc "lib$name.a" "$@")
    [[ "$(ar t "$work/lib$name.a" | grep -v '^__\.SYMDEF' | tr '\n' ' ')" == "$* " ]] \
        || { echo "::error::ar did not put $* in lib$name.a"; exit 1; }
}
# rawlib NAME MEMBER...: libNAME.a written here byte for byte (the common ar format, names of
# at most 15 characters), so that it can hold a member that is not an object
rawlib() {
    local name=$1 m size
    shift
    {
        printf '!<arch>\n'
        for m in "$@"; do
            size=$(wc -c < "$work/$m" | tr -d ' ')
            printf '%-16s%-12s%-6s%-6s%-8s%-10s`\n' "$m" 0 0 0 644 "$size"
            cat "$work/$m"
            if [[ $((size % 2)) -eq 1 ]]; then printf '\n'; fi
        done
    } > "$work/lib$name.a"
}
# expect STATUS NAME REGEX LIBRARY [VAR=VALUE...]: the check exits STATUS, its output matches REGEX
expect() {
    local want=$1 name=$2 text=$3 library=$4 got=0
    shift 4
    env "$@" "$check" "$library" > "$work/out" 2>&1 || got=$?
    if [[ $got == "$want" ]] && grep -E -- "$text" "$work/out" > /dev/null; then
        echo "ok: $name"
    else
        echo "FAIL: $name: exit status $got, expected $want and /$text/"
        sed 's/^/    /' "$work/out" | head -n 20
        failures=$((failures + 1))
    fi
}

obj a
obj b
obj planted _ZN6claxon5frame6decode17h0123456789abcdefE
cp "$work/b.o" "$work/claxon.o"
: > "$work/empty.c"
# shellcheck disable=SC2086
$cc -c -o "$work/empty.o" "$work/empty.c"
printf 'not an object file\n' > "$work/junk.o"
printf 'int few(void) { return 0; }\n' > "$work/few.c"
# shellcheck disable=SC2086
$cc -c -o "$work/few.o" "$work/few.c"

lib clean a.o empty.o b.o
lib planted a.o empty.o planted.o
lib member a.o claxon.o
rawlib junk a.o junk.o b.o
rawlib raw a.o empty.o b.o
lib few few.o

# An nm that drops every line of member b.o from the archive's listing, without a word.
cat > "$work/nm-drops" << 'EOF'
#!/bin/bash
nm "$@" | grep -v ':b\.o:'
EOF
# An nm that exits 1 after a full listing.
cat > "$work/nm-fails" << 'EOF'
#!/bin/bash
nm "$@"
exit 1
EOF
# An nm that lists everything but says something else besides.
cat > "$work/nm-warns" << 'EOF'
#!/bin/bash
nm "$@"
echo "nm: warning: something went unread" >&2
EOF
# An nm that gets absolute paths rewritten to Windows form, as MSYS2 does for a native (MinGW)
# program: its -A prefixes and messages then name D:/tmp/x.a where it was given /tmp/x.a.
cat > "$work/nm-msys" << 'EOF'
#!/bin/bash
rc=0
err=$(mktemp)
out=$(nm "$@" 2> "$err") || rc=$?
msg=$(cat "$err")
rm -f "$err"
for a in "$@"; do
    if [[ "$a" == /* ]]; then
        out=${out//"$a:"/"D:$a:"}
        msg=${msg//"$a:"/"D:$a:"}
    fi
done
[[ -z "$out" ]] || printf '%s\n' "$out"
[[ -z "$msg" ]] || printf '%s\n' "$msg" >&2
exit "$rc"
EOF
chmod +x "$work"/nm-*

expect 0 "a clean archive passes (a member without symbols is not an error)" "none from claxon" "$work/libclean.a"
expect 1 "a planted claxon symbol is found" "claxon symbols" "$work/libplanted.a"
expect 1 "a member named after claxon is found" "claxon symbols" "$work/libmember.a"
# GNU nm reports the member it cannot read and exits 1; LLVM's nm (23.1 on Linux) skips it without
# a word, which the check finds as a member missing from the listing.
expect 0 "a hand-written archive of objects passes (the format rawlib writes is read)" "none from claxon" "$work/libraw.a"
expect 2 "a member nm cannot read fails the check" "cannot read|not in nm's listing" "$work/libjunk.a"
expect 2 "a library with few symbols (stripped) fails the check" "stripped\\?" "$work/libfew.a"
expect 2 "a missing library fails the check" "no such file" "$work/libnone.a"
expect 2 "a member left out of nm's listing fails the check" "not in nm's listing" "$work/libclean.a" NM="$work/nm-drops"
expect 2 "an nm error exit fails the check" "cannot read" "$work/libclean.a" NM="$work/nm-fails"
expect 2 "an nm message other than \"no symbols\" fails the check" "cannot read" "$work/libclean.a" NM="$work/nm-warns"
expect 2 "an error outranks a planted symbol" "cannot read" "$work/libplanted.a" NM="$work/nm-fails"
# MSYS2's rewrite of absolute paths (the Windows-gnu job: 275 of 279 members "not in nm's
# listing" when the check matched nm's prefixes against the path it was given).
expect 0 "an nm given Windows-form paths (MSYS2) passes a clean archive" "none from claxon" "$work/libclean.a" NM="$work/nm-msys"
expect 1 "an nm given Windows-form paths finds a planted claxon symbol" "claxon symbols" "$work/libplanted.a" NM="$work/nm-msys"
expect 2 "an nm given Windows-form paths still fails on a member it cannot read" "cannot read|not in nm's listing" "$work/libjunk.a" NM="$work/nm-msys"

if [[ $failures -ne 0 ]]; then
    echo "::error::$failures tests of check-capi-no-claxon.sh failed"
    exit 1
fi
echo "check-capi-no-claxon.sh: all tests passed"
