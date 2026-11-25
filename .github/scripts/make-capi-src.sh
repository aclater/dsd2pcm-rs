#!/bin/bash
# The C library's source archive, libdsd2pcm-<version>-src.tar.gz (prefix
# libdsd2pcm-<version>/), from a commit: dsd2pcm and dsd2pcm-capi as a workspace of their
# own that builds with `cargo build --offline --locked`, with nothing from a registry
# (docs/c-api.md, "Offline source build"). It holds:
#   - crates/dsd2pcm and crates/dsd2pcm-capi: build.rs and src/ only (no tests, benches,
#     examples or fuzz targets), byte for byte from the commit;
#   - Cargo.toml (both crates' and the workspace's) and Cargo.lock, written by
#     capi-src-manifests.py from the commit's manifests: without the `flac` feature, its
#     optional claxon and the dev-dependencies, which Cargo would otherwise look up;
#   - include/dsd2pcm.h, examples/minimal.c, LICENSE-MIT, LICENSE-APACHE, NOTICE and a
#     README.md.
# Deterministic: the commit's files (git archive), entries sorted by name, every time the
# commit's (SOURCE_DATE_EPOCH), owner 0, fixed modes, ustar, `gzip -n -9`. The same commit
# gives the same bytes. Needs GNU tar, git and Python 3.11 or later.
#
#     .github/scripts/make-capi-src.sh OUT_DIR [COMMIT]
#
# Writes OUT_DIR/libdsd2pcm-<version>-src.tar.gz and its .sha256 file, and prints the
# archive's name. COMMIT defaults to HEAD.
set -euo pipefail

[[ $# -ge 1 && $# -le 2 ]] || { echo "usage: $0 OUT_DIR [COMMIT]" >&2; exit 2; }
out=$1
commit=$(git rev-parse --verify "${2:-HEAD}^{commit}")
here=$(cd "$(dirname "$0")" && pwd)
tar --version 2>/dev/null | grep -q 'GNU tar' || { echo "make-capi-src.sh: needs GNU tar" >&2; exit 2; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir "$work/git" "$work/manifests"
git archive --format=tar "$commit" -- Cargo.toml LICENSE-MIT LICENSE-APACHE NOTICE \
    crates/dsd2pcm/Cargo.toml crates/dsd2pcm/src \
    crates/dsd2pcm-capi/Cargo.toml crates/dsd2pcm-capi/build.rs crates/dsd2pcm-capi/src \
    crates/dsd2pcm-capi/include/dsd2pcm.h crates/dsd2pcm-capi/tests/minimal.c |
    tar -x -C "$work/git"
version=$(python3 -I "$here/capi-src-manifests.py" "$work/git" "$work/manifests")
name="libdsd2pcm-$version"
d="$work/$name"
mkdir -p "$d/crates" "$d/include" "$d/examples"
for c in dsd2pcm dsd2pcm-capi; do
    mkdir "$d/crates/$c"
    cp -R "$work/git/crates/$c/src" "$d/crates/$c/src"
    cp "$work/manifests/crates/$c/Cargo.toml" "$d/crates/$c/Cargo.toml"
done
cp "$work/git/crates/dsd2pcm-capi/build.rs" "$d/crates/dsd2pcm-capi/"
cp "$work/manifests/Cargo.toml" "$work/manifests/Cargo.lock" "$d/"
cp "$work/git/LICENSE-MIT" "$work/git/LICENSE-APACHE" "$work/git/NOTICE" "$d/"
cp "$work/git/crates/dsd2pcm-capi/include/dsd2pcm.h" "$d/include/"
cp "$work/git/crates/dsd2pcm-capi/tests/minimal.c" "$d/examples/"
cat > "$d/README.md" <<EOF
# libdsd2pcm $version: source of the C library

The source of dsd2pcm's C library (ABI version 1) at version $version, for building it with
no network: the crates \`dsd2pcm\` and \`dsd2pcm-capi\` of
https://github.com/dsd-toolbox/dsd2pcm-rs as a workspace of their own, with no dependencies
outside it. Commit $commit.

Only the manifests differ from that commit. This workspace has the two crates only; their
tests, benchmarks and examples are left out; and \`dsd2pcm\` has no \`flac\` feature, no
claxon and no dev-dependencies, so \`Cargo.lock\` lists the two crates alone. The C library is
built without \`flac\` in every case, so it is the same library.
\`.github/scripts/capi-src-manifests.py\` in the repository writes the manifests.

Build it with Rust $(sed -n 's/^rust-version = "\(.*\)"$/\1/p' "$d/Cargo.toml") or later, offline:

    cargo build --offline --locked --release -p dsd2pcm-capi

This gives \`target/release/libdsd2pcm.a\` and \`libdsd2pcm.so\` (\`libdsd2pcm.dylib\` on macOS,
\`dsd2pcm.dll\` on Windows; the Linux SONAME is \`libdsd2pcm.so.1\`). The header is
\`include/dsd2pcm.h\`. To build and run the example on Linux:

    cc -std=c11 -I include examples/minimal.c target/release/libdsd2pcm.a -lpthread -ldl -lm -o minimal
    ./minimal

It prints \`dsd2pcm $version: 2352 samples per channel, first byte 0\`. The C interface is
documented in docs/c-api.md in the repository. Licence: MIT OR Apache-2.0 (LICENSE-MIT,
LICENSE-APACHE, NOTICE).
EOF

epoch=$(git log -1 --format=%ct "$commit")
mkdir -p "$out"
tar -C "$work" --sort=name --format=ustar --mtime="@$epoch" --owner=0 --group=0 \
    --numeric-owner --mode='u=rwX,go=rX' -cf - "$name" | gzip -n -9 > "$out/$name-src.tar.gz"
(cd "$out" && sha256sum "$name-src.tar.gz" > "$name-src.tar.gz.sha256")
echo "$name-src.tar.gz"
