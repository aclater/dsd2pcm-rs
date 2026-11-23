#!/bin/bash
# Check an unpacked C library archive (Linux with glibc or musl, macOS, or Windows with
# MinGW-w64 under MSYS2) the way a user builds against it:
#
#     .github/scripts/check-capi-archive.sh DIR
#
# - the archive's examples/minimal.c (crates/dsd2pcm-capi/tests/minimal.c) compiles with the flags from DIR's pkg-config file,
#   links against the shared library and runs (CC, default cc; for example "gcc -m32");
# - the shared library's name is the ABI-versioned one: SONAME libdsd2pcm.so.1 on Linux,
#   install name @rpath/libdsd2pcm.1.dylib on macOS, and the program depends on that name;
#   on Windows the program imports dsd2pcm.dll;
# - the same program links against the static library with Libs.private and runs;
# - the static library embeds in a shared library of the user's own, built with its symbols
#   hidden as docs/c-api.md says (--exclude-libs; -exported_symbols_list on macOS): this
#   fails if the static library is not position-independent (-z text), and that shared
#   library must export the user's function and nothing of libdsd2pcm.a. A program linked
#   against it runs.
set -euo pipefail

dir=$(cd "$1" && pwd)
cc=${CC:-cc}
src=$dir/examples/minimal.c
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
abi=$(sed -n 's/^#define DSD2PCM_ABI_VERSION \([0-9]*\).*/\1/p' "$dir/include/dsd2pcm.h")
os=$(uname -s)
case "$os" in MINGW* | MSYS* | CYGWIN*) os=Windows ;; esac

export PKG_CONFIG_PATH=$dir/lib/pkgconfig
pkg-config --version
pkg-config --modversion dsd2pcm
read -r -a cflags <<< "$(pkg-config --cflags dsd2pcm)"
read -r -a libs <<< "$(pkg-config --libs dsd2pcm)"
read -r -a private <<< "$(pkg-config --libs --static dsd2pcm | sed 's/-ldsd2pcm//')"
echo "cflags: ${cflags[*]}"
echo "libs: ${libs[*]}"
echo "static: ${private[*]}"

# Windows has no rpath: a program finds a DLL in its own folder.
rpath=() erpath=()
if [[ "$os" != Windows ]]; then
    rpath=("-Wl,-rpath,$dir/lib") erpath=("-Wl,-rpath,$work")
fi
# shellcheck disable=SC2086 # CC may hold flags (gcc -m32)
$cc -std=c99 -Wall -Wextra -Werror "${cflags[@]}" -o "$work/minimal" "$src" "${libs[@]}" "${rpath[@]}"

case "$os" in
    Linux)
        so=$dir/lib/libdsd2pcm.so.$abi
        soname=$(readelf -d "$so" | sed -n 's/.*(SONAME).*\[\(.*\)\]/\1/p')
        echo "SONAME: $soname"
        [[ "$soname" == "libdsd2pcm.so.$abi" ]] || { echo "::error::SONAME is '$soname', not libdsd2pcm.so.$abi"; exit 1; }
        readelf -d "$work/minimal" | grep "NEEDED.*\[libdsd2pcm.so.$abi\]" > /dev/null \
            || { echo "::error::the program does not depend on libdsd2pcm.so.$abi"; exit 1; }
        readelf -d "$so" | grep NEEDED
        # x86-64: every object of the static library carries the CET shadow-stack property
        # (.github/scripts/clib-shstk.py), so a program built with -fcf-protection keeps it
        # when it links the library. The shared library is not marked: it holds zig's
        # libunwind, which does not unwind the shadow stack. Never IBT: Rust's standard
        # library has no ENDBR64.
        if readelf -h "$so" | grep 'Machine:.*X86-64' > /dev/null; then
            a=$dir/lib/libdsd2pcm.a
            members=$(ar t "$a" | wc -l)
            marked=$(readelf -n "$a" | grep -c 'x86 feature: SHSTK$' || true)
            echo "static library members marked SHSTK: $marked of $members"
            [[ "$marked" -eq "$members" ]] || { echo "::error::$((members - marked)) of $members objects in libdsd2pcm.a are not marked SHSTK (and only SHSTK)"; exit 1; }
            # A relocatable link merges the property over the objects it takes from the
            # archive, as a program link does, without the system's start files. The program
            # is compiled first, so that -nostdlib (link only) leaves the C headers alone.
            # shellcheck disable=SC2086
            $cc -std=c99 -fcf-protection=full -c "${cflags[@]}" -o "$work/minimal-cf.o" "$src"
            # shellcheck disable=SC2086
            $cc -nostdlib -r -o "$work/minimal-cet.o" "$work/minimal-cf.o" "$a"
            [[ "$(readelf -n "$work/minimal-cet.o" | grep 'x86 feature:')" == *'x86 feature: SHSTK' ]] \
                || { echo "::error::a program object linked with libdsd2pcm.a is not marked SHSTK"; exit 1; }
            # The shared library must stay unmarked while it carries LLVM's libunwind, which
            # does not unwind the shadow stack: a panic caught at the C boundary would crash a
            # process running with shadow stacks (docs/design.md, "Intel CET").
            if grep -q 'libunwind: ' "$so"; then
                if readelf -n "$so" | grep 'x86 feature:.*SHSTK' > /dev/null; then
                    echo "::error::$so is marked SHSTK but holds libunwind, which does not unwind the shadow stack"; exit 1
                fi
                echo "shared library: holds libunwind, not marked SHSTK"
            fi
        fi
        ;;
    Darwin)
        id=$(otool -D "$dir/lib/libdsd2pcm.$abi.dylib" | tail -n 1)
        echo "install name: $id"
        [[ "$id" == "@rpath/libdsd2pcm.$abi.dylib" ]] || { echo "::error::install name is '$id'"; exit 1; }
        otool -L "$work/minimal" | grep "@rpath/libdsd2pcm.$abi.dylib" > /dev/null \
            || { echo "::error::the program does not depend on @rpath/libdsd2pcm.$abi.dylib"; exit 1; }
        ;;
    Windows)
        objdump -p "$dir/bin/dsd2pcm.dll" | grep 'DLL Name'
        objdump -p "$work/minimal.exe" | grep 'DLL Name: dsd2pcm.dll' > /dev/null \
            || { echo "::error::the program does not import dsd2pcm.dll"; exit 1; }
        cp "$dir/bin/dsd2pcm.dll" "$work/"
        ;;
esac
"$work/minimal"

# shellcheck disable=SC2086
$cc -std=c99 -Wall -Wextra -Werror "${cflags[@]}" -o "$work/minimal-static" "$src" "$dir/lib/libdsd2pcm.a" "${private[@]}"
"$work/minimal-static"

# Embedded: minimal.c's main renamed minimal_main, in a shared library with libdsd2pcm.a.
echo "global symbols defined in libdsd2pcm.a: $(nm -g --defined-only "$dir/lib/libdsd2pcm.a" 2> /dev/null | grep -c ' [A-Z] ')"
printf 'int minimal_main(void);\nint main(void) { return minimal_main(); }\n' > "$work/main.c"
case "$os" in
    Linux)
        # shellcheck disable=SC2086
        $cc -std=c99 -fPIC -shared -Dmain=minimal_main "${cflags[@]}" -o "$work/libembed.so" "$src" \
            "$dir/lib/libdsd2pcm.a" -Wl,--exclude-libs,libdsd2pcm.a -Wl,-z,text -Wl,-z,defs "${private[@]}"
        exports=$(nm -D --defined-only "$work/libembed.so" | awk '{print $NF}')
        lib=$work/libembed.so
        ;;
    Darwin)
        echo _minimal_main > "$work/exports.txt"
        # shellcheck disable=SC2086
        $cc -std=c99 -dynamiclib -Dmain=minimal_main "${cflags[@]}" -o "$work/libembed.dylib" "$src" \
            "$dir/lib/libdsd2pcm.a" -Wl,-exported_symbols_list,"$work/exports.txt" "${private[@]}"
        exports=$(nm -gU "$work/libembed.dylib" | awk '{print $NF}' | sed 's/^_//')
        lib=$work/libembed.dylib
        ;;
    Windows)
        # shellcheck disable=SC2086
        $cc -std=c99 -shared -static-libgcc -Dmain=minimal_main "${cflags[@]}" -o "$work/embed.dll" "$src" \
            "$dir/lib/libdsd2pcm.a" -Wl,--exclude-libs,ALL -Wl,--out-implib,"$work/libembed.dll.a" "${private[@]}"
        exports=$(objdump -p "$work/embed.dll" | awk '/\[Ordinal\/Name Pointer\] Table/ {on = 1; next} on && !NF {on = 0} on && /^[[:space:]]*\[/ {print $NF}')
        lib=$work/embed.dll
        # -static-libgcc and --exclude-libs,ALL (docs/c-api.md): no MinGW runtime DLL, which
        # MSYS2's PATH would hide, and none of the static libgcc's unwinder symbols exported.
        if objdump -p "$lib" | grep -Ei 'DLL Name: (libgcc|libwinpthread|libstdc)'; then
            echo "::error::$lib needs a MinGW runtime DLL"; exit 1
        fi
        ;;
esac
echo "exported by the embedding library: $(tr '\n' ' ' <<< "$exports")"
extra=$(grep -Ev '^(minimal_main|_init|_fini|_edata|_end|__bss_start)$' <<< "$exports" || true)
[[ -z "$extra" ]] || { echo "::error::$lib exports libdsd2pcm.a's symbols: $(head -n 5 <<< "$extra" | tr '\n' ' ')"; exit 1; }
grep -qx minimal_main <<< "$exports" || { echo "::error::$lib does not export minimal_main"; exit 1; }
# shellcheck disable=SC2086
$cc -o "$work/embedded" "$work/main.c" -L"$work" -lembed "${erpath[@]}"
"$work/embedded"
echo "C library archive: shared, static and embedded programs built and ran"
