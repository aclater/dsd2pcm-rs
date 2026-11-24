#!/bin/bash
# Tests of check-private-names.sh in throwaway repositories (a temporary directory), with a
# dummy PRIVATE_NAMES regex: every case must give its exit status (0 clean, 1 found, 2 the
# check failed), so the check can neither pass on a hit nor on an error.
#
#     .github/scripts/check-private-names-test.sh
set -euo pipefail

check=$(cd "$(dirname "$0")" && pwd)/check-private-names.sh
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failures=0

repo() { # a fresh repository with one clean commit
    rm -rf "$work/r"
    git init -q "$work/r"
    git -C "$work/r" config user.name tester
    git -C "$work/r" config user.email tester@example.com
    echo clean > "$work/r/a.txt"
    commit "a clean file"
}
commit() {
    git -C "$work/r" add -A
    git -C "$work/r" commit -q --allow-empty -m "$1"
}
expect() { # expect STATUS NAME [VAR=VALUE...]
    local want=$1 name=$2 got=0
    shift 2
    (cd "$work/r" && env "$@" "$check") > "$work/out" 2>&1 || got=$?
    if [[ $got == "$want" ]]; then
        echo "ok: $name ($got)"
    else
        echo "FAIL: $name: exit $got, expected $want"
        sed 's/^/    /' "$work/out"
        failures=$((failures + 1))
    fi
}

repo
expect 0 "clean tree, no PRIVATE_NAMES"
expect 0 "clean tree, a dummy PRIVATE_NAMES" PRIVATE_NAMES='dummyhost[0-9]+'
for hit in 'nas.home.arpa' '10.1.2.3' '172.20.0.5' '192.168.7.9'; do
    repo
    echo "see $hit" > "$work/r/b.txt"
    commit "b"
    expect 1 "built-in pattern in a file: $hit"
done
repo
echo 'public 172.32.0.1 and 11.0.0.1, version 10.0.19041' > "$work/r/b.txt"
commit "b"
expect 0 "public addresses and a version number"

repo
echo 'DummyHost42 was here' > "$work/r/b.txt"
commit "b"
expect 1 "PRIVATE_NAMES in a file" PRIVATE_NAMES='dummyhost[0-9]+'
expect 0 "the same file without PRIVATE_NAMES" PRIVATE_NAMES=

# A hit among thousands of files, past the first (xargs would split such a list).
repo
mkdir "$work/r/many"
for i in $(seq 1 3000); do echo "file $i" > "$work/r/many/f$i.txt"; done
echo 'dummyhost7' > "$work/r/many/f2999.txt"
echo 'dummyhost8' > "$work/r/many/f3.txt"
commit "many"
expect 1 "hits in a long file list" PRIVATE_NAMES='dummyhost[0-9]+'

# A tracked symbolic link to a directory, and a name starting with '-', next to a hit.
repo
mkdir "$work/r/d"
echo inside > "$work/r/d/x.txt"
ln -s d "$work/r/link"
echo 'see 192.168.0.1' > "$work/r/-n.txt"
commit "link"
expect 1 "a hit next to a directory symlink and a dash name"
repo
mkdir "$work/r/d"
ln -s d "$work/r/link"
commit "link"
expect 0 "a directory symlink alone"

# PRIVATE_NAMES that would match everything or nothing: refused, never a pass.
repo
expect 2 "an invalid PRIVATE_NAMES regex" PRIVATE_NAMES='dummy(host'
expect 2 "a trailing '|' in PRIVATE_NAMES" PRIVATE_NAMES='dummyhost|'
expect 2 "a leading '|' in PRIVATE_NAMES" PRIVATE_NAMES='|dummyhost'
expect 2 "PRIVATE_NAMES of spaces" PRIVATE_NAMES=' '

# Commit identities.
repo
base=$(git -C "$work/r" rev-parse HEAD)
git -C "$work/r" -c user.email=me@192.168.1.5 commit -q --allow-empty -m id
expect 1 "a private address in an identity"
expect 1 "the same, BASE_SHA before it" BASE_SHA="$base"
expect 0 "the same, BASE_SHA at it" BASE_SHA="$(git -C "$work/r" rev-parse HEAD)"
git -C "$work/r" -c user.name=DummyHost3 commit -q --allow-empty -m id2
expect 1 "PRIVATE_NAMES in an identity" BASE_SHA="$(git -C "$work/r" rev-parse HEAD~1)" PRIVATE_NAMES='dummyhost[0-9]+'
expect 2 "an invalid regex with identities only" BASE_SHA="$(git -C "$work/r" rev-parse HEAD)" PRIVATE_NAMES='a['

if [[ $failures -gt 0 ]]; then
    echo "::error::$failures check-private-names test(s) failed"
    exit 1
fi
echo "check-private-names: all tests passed"
