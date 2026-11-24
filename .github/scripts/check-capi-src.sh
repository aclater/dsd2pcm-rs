#!/bin/bash
# Checks the C library's source archive that make-capi-src.sh made from COMMIT:
#   1. making it again gives the same bytes (reproducible);
#   2. it holds exactly the expected files, and its sources are the commit's byte for byte;
#   3. its Cargo.lock lists dsd2pcm and dsd2pcm-capi only;
#   4. unpacked in a fresh folder, `cargo build --offline --locked --release -p dsd2pcm-capi`
#      builds it with an empty CARGO_HOME, without a network (in a network namespace of its
#      own, `unshare -rn`, where the system allows it; with REQUIRE_NO_NETWORK=1 it must);
#   5. its examples/minimal.c builds against the static library and the shared library,
#      and both print the expected line.
#
#     .github/scripts/check-capi-src.sh ARCHIVE [COMMIT]
#
# CC names the C compiler (default cc). COMMIT defaults to HEAD.
set -euo pipefail

[[ $# -ge 1 && $# -le 2 ]] || { echo "usage: $0 ARCHIVE [COMMIT]" >&2; exit 2; }
archive=$(realpath "$1")
commit=$(git rev-parse --verify "${2:-HEAD}^{commit}")
here=$(cd "$(dirname "$0")" && pwd)
cc=${CC:-cc}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fail() { echo "::error::$*" >&2; exit 1; }

name=$(basename "$archive" -src.tar.gz)
version=${name#libdsd2pcm-}
[[ "$name" == libdsd2pcm-* && "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]] || fail "$archive: not libdsd2pcm-<version>-src.tar.gz"

echo "== reproducible"
"$here/make-capi-src.sh" "$work/again" "$commit" > /dev/null
cmp "$archive" "$work/again/$name-src.tar.gz" || fail "a second make-capi-src.sh run gave different bytes"
sha256sum "$archive"

echo "== contents"
tar -tzf "$archive" | sort > "$work/list"
grep -v "^$name/" "$work/list" && fail "entries outside $name/"
# Every source file of the two crates' src/, and nothing else under crates/ but the
# manifests and build.rs.
{
    printf '%s\n' Cargo.toml Cargo.lock README.md LICENSE-MIT LICENSE-APACHE NOTICE \
        include/dsd2pcm.h examples/minimal.c crates/dsd2pcm/Cargo.toml \
        crates/dsd2pcm-capi/Cargo.toml crates/dsd2pcm-capi/build.rs
    git ls-tree -r --name-only "$commit" -- crates/dsd2pcm/src crates/dsd2pcm-capi/src
} | sed "s|^|$name/|" | sort > "$work/want"
grep -v '/$' "$work/list" > "$work/files"
diff -u "$work/want" "$work/files" || fail "the archive's files are not the expected ones"
echo "$(wc -l < "$work/files") files"
mkdir "$work/x"
tar -C "$work/x" -xzf "$archive"
src="$work/x/$name"
while read -r f; do
    git cat-file blob "$commit:$f" | cmp -s - "$src/$f" || fail "$f differs from the commit's"
done < <(git ls-tree -r --name-only "$commit" -- crates/dsd2pcm/src crates/dsd2pcm-capi/src LICENSE-MIT LICENSE-APACHE NOTICE crates/dsd2pcm-capi/build.rs)
git cat-file blob "$commit:crates/dsd2pcm-capi/include/dsd2pcm.h" | cmp -s - "$src/include/dsd2pcm.h" || fail "include/dsd2pcm.h differs"
git cat-file blob "$commit:crates/dsd2pcm-capi/tests/minimal.c" | cmp -s - "$src/examples/minimal.c" || fail "examples/minimal.c differs"

echo "== Cargo.lock"
cat "$src/Cargo.lock"
python3 -I - "$src/Cargo.lock" "$version" <<'EOF' || fail "Cargo.lock lists other packages"
import sys, tomllib
lock = tomllib.loads(open(sys.argv[1]).read())
pkgs = sorted((p["name"], p["version"], "source" in p) for p in lock["package"])
want = [("dsd2pcm", sys.argv[2], False), ("dsd2pcm-capi", sys.argv[2], False)]
sys.exit(0 if pkgs == want else 1)
EOF

echo "== offline build, empty CARGO_HOME"
export CARGO_HOME="$work/cargo-home"
mkdir "$CARGO_HOME"
build=(cargo build --offline --locked --release -p dsd2pcm-capi)
if unshare -rn true 2> /dev/null; then
    echo "no network: unshare -rn"
    (cd "$src" && unshare -rn "${build[@]}")
else
    [[ "${REQUIRE_NO_NETWORK:-0}" != 1 ]] || fail "unshare -rn is not allowed here: cannot build without a network"
    echo "unshare -rn is not allowed here: offline by --offline and the empty CARGO_HOME only"
    (cd "$src" && "${build[@]}")
fi
# Cargo leaves its lock files there (.package-cache, .global-cache); nothing else.
[[ ! -e "$CARGO_HOME/registry" && ! -e "$CARGO_HOME/git" ]] || { ls -AR "$CARGO_HOME"; fail "the build fetched into CARGO_HOME"; }
rel="$src/target/release"

echo "== examples/minimal.c"
want_line="dsd2pcm $version: 2352 samples per channel, first byte 0"
case "$(uname -s)" in
    Linux) sys_libs=(-lpthread -ldl -lm) ;;
    *) sys_libs=() ;;
esac
# shellcheck disable=SC2086 # CC may hold flags
$cc -std=c11 -Wall -Wextra -Werror -I "$src/include" "$src/examples/minimal.c" "$rel/libdsd2pcm.a" "${sys_libs[@]}" -o "$work/minimal-static"
got=$("$work/minimal-static")
[[ "$got" == "$want_line" ]] || fail "static: printed '$got'"
echo "static: $got"
if [[ -f "$rel/libdsd2pcm.so" ]]; then
    ln -sf libdsd2pcm.so "$rel/libdsd2pcm.so.1"
    # shellcheck disable=SC2086
    $cc -std=c11 -Wall -Wextra -Werror -I "$src/include" "$src/examples/minimal.c" -L "$rel" -ldsd2pcm -o "$work/minimal-shared"
    got=$(LD_LIBRARY_PATH="$rel" "$work/minimal-shared")
    [[ "$got" == "$want_line" ]] || fail "shared: printed '$got'"
    echo "shared: $got"
fi
echo "ok: $name-src.tar.gz"
