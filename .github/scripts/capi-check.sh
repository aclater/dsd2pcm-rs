#!/bin/bash
# The C library (crates/dsd2pcm-capi): its committed header matches what cbindgen generates,
# the library builds, and C programs compile and run against it.
#
#     .github/scripts/capi-check.sh [CRATE_DIR]
#
# - The header: the one .h file under CRATE_DIR (outside target/), regenerated with cbindgen
#   (and CRATE_DIR/cbindgen.toml when present) and compared byte for byte. cbindgen must be on
#   the PATH (the workflow installs a pinned version).
# - The header alone compiles as C99 and C11 with every warning an error.
# - Every CRATE_DIR/tests/*.c is compiled with the same flags, linked against the static
#   library and run; a non-zero exit fails the check. Without any, a program that only
#   includes the header is linked instead.
# - With ASAN_CC set (clang), every CRATE_DIR/tests/*.c again, built with AddressSanitizer,
#   LeakSanitizer and UndefinedBehaviorSanitizer and run with detect_leaks=1: any leak,
#   memory error or undefined behaviour fails the check. leakloop.c runs
#   LEAKLOOP_ITERATIONS times (default 100).
# Does nothing (exit 0) when CRATE_DIR does not exist.
set -euo pipefail

dir=${1:-crates/dsd2pcm-capi}
if [[ ! -d "$dir" ]]; then
    echo "$dir does not exist: no C library to check"
    exit 0
fi
cc=${CC:-cc}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

mapfile -t headers < <(find "$dir" -path "$dir/target" -prune -o -name '*.h' -print | LC_ALL=C sort)
if [[ ${#headers[@]} -ne 1 ]]; then
    echo "::error::expected exactly one committed header under $dir, found ${#headers[@]}: ${headers[*]}"
    exit 1
fi
header=${headers[0]}
incdir=$(dirname "$header")
echo "header: $header"

config=()
[[ -f "$dir/cbindgen.toml" ]] && config=(--config "$dir/cbindgen.toml")
cbindgen --version
cbindgen --quiet "${config[@]}" --crate "$(basename "$dir")" --output "$work/generated.h" "$dir"
if ! diff -u "$header" "$work/generated.h"; then
    echo "::error file=$header::the committed header differs from cbindgen's output (diff above): regenerate it"
    exit 1
fi
echo "header matches cbindgen"

flags=(-Wall -Wextra -Werror -pedantic -I "$incdir")
for std in c99 c11; do
    printf '#include "%s"\n' "$(basename "$header")" > "$work/include-only.c"
    "$cc" -std="$std" "${flags[@]}" -fsyntax-only "$work/include-only.c"
done
echo "header compiles as C99 and C11"

pkg=$(basename "$dir")
cargo build --locked --release -p "$pkg"
lib=$(cargo metadata --format-version 1 --no-deps \
    | python3 -I -c 'import json, sys
pkg = sys.argv[1]
for p in json.load(sys.stdin)["packages"]:
    if p["name"] == pkg:
        print(next(t["name"] for t in p["targets"] if "staticlib" in t["crate_types"]))' "$pkg")
target_dir=$(cargo metadata --format-version 1 --no-deps | python3 -I -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
archive="$target_dir/release/lib$lib.a"
ls -l "$archive"

mapfile -t programs < <(find "$dir/tests" -maxdepth 1 -name '*.c' 2>/dev/null | LC_ALL=C sort)
if [[ ${#programs[@]} -eq 0 ]]; then
    printf '#include "%s"\nint main(void) { return 0; }\n' "$(basename "$header")" > "$work/smoke.c"
    programs=("$work/smoke.c")
fi
for src in "${programs[@]}"; do
    exe="$work/$(basename "$src" .c)"
    "$cc" -std=c99 "${flags[@]}" -o "$exe" "$src" "$archive" -lpthread -ldl -lm
    echo "run: $src"
    "$exe"
done
echo "C smoke test passed (${#programs[@]} program(s))"

if [[ -n "${ASAN_CC:-}" ]]; then
    "$ASAN_CC" --version | head -n 1
    for src in "${programs[@]}"; do
        exe="$work/asan-$(basename "$src" .c)"
        "$ASAN_CC" -std=c99 "${flags[@]}" -O1 -g -fno-omit-frame-pointer \
            -fsanitize=address,undefined -fno-sanitize-recover=all \
            -o "$exe" "$src" "$archive" -lpthread -ldl -lm
        args=()
        [[ $(basename "$src") == leakloop.c ]] && args=("${LEAKLOOP_ITERATIONS:-100}")
        echo "run (ASan, LSan, UBSan): $src ${args[*]}"
        ASAN_OPTIONS=detect_leaks=1:halt_on_error=1 UBSAN_OPTIONS=print_stacktrace=1 \
            "$exe" "${args[@]}"
    done
    echo "C tests passed under the sanitizers, no leaks"
fi
