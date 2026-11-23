#!/bin/bash
# Lay out the C library's release archive directory (.github/workflows/release.yml):
#
#     .github/scripts/stage-capi.sh VERSION NAME LIBDIR OUTDIR "STATIC_LIBS"
#
# VERSION is the release version, NAME the platform (x86_64-linux, aarch64-macos, ...),
# LIBDIR the cargo output directory holding the built dsd2pcm-capi libraries, OUTDIR where
# OUTDIR/libdsd2pcm-VERSION-NAME/ is made, and STATIC_LIBS the system libraries the static
# library needs (rustc's --print native-static-libs). Prints the directory's name.
#
#   include/dsd2pcm.h
#   lib/libdsd2pcm.so.1 + lib/libdsd2pcm.so -> libdsd2pcm.so.1        Linux (SONAME libdsd2pcm.so.1)
#   lib/libdsd2pcm.1.dylib + lib/libdsd2pcm.dylib -> libdsd2pcm.1.dylib  macOS (install name @rpath/libdsd2pcm.1.dylib)
#   bin/dsd2pcm.dll, lib/dsd2pcm.dll.lib (import library)              Windows
#   lib/libdsd2pcm.a (lib/dsd2pcm.lib on Windows)                      the static library
#   lib/pkgconfig/dsd2pcm.pc                                           Linux and macOS: prefix-relative
#   examples/minimal.c, README.md (docs/c-api.md, "Binary releases"), LICENSE-MIT, LICENSE-APACHE
set -euo pipefail

version=$1 name=$2 libdir=$3 outdir=$4 static_libs=$5
[[ -n "$static_libs" && "$static_libs" =~ ^[-[:alnum:]_.:/\ ]+$ ]] \
    || { echo "::error::unexpected static library list: '$static_libs'"; exit 1; }
abi=$(sed -n 's/^#define DSD2PCM_ABI_VERSION \([0-9]*\).*/\1/p' crates/dsd2pcm-capi/include/dsd2pcm.h)
[[ "$abi" =~ ^[0-9]+$ ]] || { echo "::error::no DSD2PCM_ABI_VERSION in the header"; exit 1; }

pkg=libdsd2pcm-$version-$name
dir=$outdir/$pkg
rm -rf "$dir"
mkdir -p "$dir/include" "$dir/lib"
cp crates/dsd2pcm-capi/include/dsd2pcm.h "$dir/include/"
cp LICENSE-MIT LICENSE-APACHE "$dir/"
mkdir -p "$dir/examples"
cp crates/dsd2pcm-capi/tests/minimal.c "$dir/examples/"

pc=true
case "$(uname -s)" in
    Linux)
        cp "$libdir/libdsd2pcm.so" "$dir/lib/libdsd2pcm.so.$abi"
        ln -s "libdsd2pcm.so.$abi" "$dir/lib/libdsd2pcm.so"
        cp "$libdir/libdsd2pcm.a" "$dir/lib/"
        chmod 0755 "$dir/lib/libdsd2pcm.so.$abi"
        ;;
    Darwin)
        cp "$libdir/libdsd2pcm.dylib" "$dir/lib/libdsd2pcm.$abi.dylib"
        ln -s "libdsd2pcm.$abi.dylib" "$dir/lib/libdsd2pcm.dylib"
        cp "$libdir/libdsd2pcm.a" "$dir/lib/"
        chmod 0755 "$dir/lib/libdsd2pcm.$abi.dylib"
        ;;
    MINGW* | MSYS* | CYGWIN*)
        mkdir -p "$dir/bin"
        cp "$libdir/dsd2pcm.dll" "$dir/bin/"
        cp "$libdir/dsd2pcm.dll.lib" "$libdir/dsd2pcm.lib" "$dir/lib/"
        # MSVC links by file name, not -l flags: no pkg-config file.
        pc=false
        ;;
    *) echo "::error::unknown system $(uname -s)"; exit 1 ;;
esac

if $pc; then
    mkdir -p "$dir/lib/pkgconfig"
    cat > "$dir/lib/pkgconfig/dsd2pcm.pc" <<EOF
prefix=\${pcfiledir}/../..
libdir=\${prefix}/lib
includedir=\${prefix}/include

Name: dsd2pcm
Description: DSD to PCM conversion (C interface, ABI $abi)
URL: https://github.com/aclater/dsd2pcm-rs
Version: $version
Libs: -L\${libdir} -ldsd2pcm
Libs.private: $static_libs
Cflags: -I\${includedir}
EOF
fi

# The README: docs/c-api.md's "Binary releases" section, with a title and the licence.
{
    echo "# libdsd2pcm $version ($name)"
    echo
    echo "The C library of dsd2pcm-rs (https://github.com/aclater/dsd2pcm-rs), ABI version $abi."
    echo "The interface is documented in include/dsd2pcm.h and in docs/c-api.md in the repository."
    echo
    awk '/^## Binary releases/ {on = 1; next} on && /^## / {exit} on' docs/c-api.md
    echo
    echo "Licence: MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE)."
} > "$dir/README.md"
grep -q 'pkg-config' "$dir/README.md" || { echo "::error::docs/c-api.md has no \"Binary releases\" section"; exit 1; }

echo "$pkg"
