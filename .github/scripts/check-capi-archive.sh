#!/bin/bash
# Check an unpacked C library archive (Linux or macOS) the way a user builds against it:
#
#     .github/scripts/check-capi-archive.sh DIR
#
# - the archive's examples/minimal.c (crates/dsd2pcm-capi/tests/minimal.c) compiles with the flags from DIR's pkg-config file,
#   links against the shared library and runs (CC, default cc; for example "gcc -m32");
# - the shared library's name is the ABI-versioned one: SONAME libdsd2pcm.so.1 on Linux,
#   install name @rpath/libdsd2pcm.1.dylib on macOS, and the program depends on that name;
# - the same program links against the static library with Libs.private and runs;
# - the static library holds no claxon symbols (check-capi-no-claxon.sh; the shared library
#   is stripped in releases, so its symbol table is checked in CI instead).
set -euo pipefail

dir=$(cd "$1" && pwd)
cc=${CC:-cc}
src=$dir/examples/minimal.c
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
abi=$(sed -n 's/^#define DSD2PCM_ABI_VERSION \([0-9]*\).*/\1/p' "$dir/include/dsd2pcm.h")

export PKG_CONFIG_PATH=$dir/lib/pkgconfig
pkg-config --version
pkg-config --modversion dsd2pcm
read -r -a cflags <<< "$(pkg-config --cflags dsd2pcm)"
read -r -a libs <<< "$(pkg-config --libs dsd2pcm)"
read -r -a private <<< "$(pkg-config --libs --static dsd2pcm | sed 's/-ldsd2pcm//')"
echo "cflags: ${cflags[*]}"
echo "libs: ${libs[*]}"

# shellcheck disable=SC2086 # CC may hold flags (gcc -m32)
$cc -std=c99 -Wall -Wextra -Werror "${cflags[@]}" -o "$work/minimal" "$src" "${libs[@]}" -Wl,-rpath,"$dir/lib"
"$work/minimal"

case "$(uname -s)" in
    Linux)
        so=$dir/lib/libdsd2pcm.so.$abi
        soname=$(readelf -d "$so" | sed -n 's/.*(SONAME).*\[\(.*\)\]/\1/p')
        echo "SONAME: $soname"
        [[ "$soname" == "libdsd2pcm.so.$abi" ]] || { echo "::error::SONAME is '$soname', not libdsd2pcm.so.$abi"; exit 1; }
        readelf -d "$work/minimal" | grep -q "NEEDED.*\[libdsd2pcm.so.$abi\]" \
            || { echo "::error::the program does not depend on libdsd2pcm.so.$abi"; exit 1; }
        ;;
    Darwin)
        id=$(otool -D "$dir/lib/libdsd2pcm.$abi.dylib" | tail -n 1)
        echo "install name: $id"
        [[ "$id" == "@rpath/libdsd2pcm.$abi.dylib" ]] || { echo "::error::install name is '$id'"; exit 1; }
        otool -L "$work/minimal" | grep -q "@rpath/libdsd2pcm.$abi.dylib" \
            || { echo "::error::the program does not depend on @rpath/libdsd2pcm.$abi.dylib"; exit 1; }
        ;;
esac

# shellcheck disable=SC2086
$cc -std=c99 -Wall -Wextra -Werror "${cflags[@]}" -o "$work/minimal-static" "$src" "$dir/lib/libdsd2pcm.a" "${private[@]}"
"$work/minimal-static"
"$(dirname "$0")/check-capi-no-claxon.sh" "$dir/lib/libdsd2pcm.a"
echo "C library archive: shared and static programs built and ran; no claxon code"
