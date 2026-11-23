#!/bin/bash
# Publish the workspace's crates to crates.io, in dependency order:
#
#     dsd2pcm  ->  dsd2pcm-capi (only when its Cargo.toml allows publishing)  ->  dsd2pcm-cli
#
#     .github/scripts/publish-crates.sh [--dry-run]
#
# Crates whose Cargo.toml says `publish = false` are skipped; a publishable crate missing from
# the order above stops the script, so a new crate is never published by accident. A version
# that crates.io already has is skipped, so a run that failed half-way can be re-run.
# --dry-run packages and verifies every crate (cargo publish --workspace --dry-run, which
# builds each against the others' packages, so it needs nothing on crates.io) and uploads
# nothing. Without it, CARGO_REGISTRY_TOKEN must hold a token (the release workflow's comes
# from trusted publishing).
set -euo pipefail

order=(dsd2pcm dsd2pcm-capi dsd2pcm-cli)

dry=false
[[ "${1:-}" == --dry-run ]] && dry=true

meta=$(cargo metadata --format-version 1 --no-deps --locked)
mapfile -t publishable < <(python3 -I -c 'import json, sys
for p in json.load(sys.stdin)["packages"]:
    if p["publish"] is None or "crates-io" in p["publish"]:
        print(p["name"], p["version"])' <<< "$meta")
declare -A version
for line in "${publishable[@]}"; do
    version[${line% *}]=${line#* }
done
for name in "${!version[@]}"; do
    [[ " ${order[*]} " == *" $name "* ]] \
        || { echo "::error::$name is publishable but not in publish-crates.sh's order: add it, or set publish = false"; exit 1; }
done
list=()
for name in "${order[@]}"; do
    [[ -n "${version[$name]:-}" ]] && list+=("$name")
done
echo "crates, in order: ${list[*]}"

if $dry; then
    args=()
    for name in "${list[@]}"; do args+=(-p "$name"); done
    cargo publish --dry-run --locked "${args[@]}"
    exit 0
fi

for name in "${list[@]}"; do
    v=${version[$name]}
    code=$(curl -sS -o /dev/null -w '%{http_code}' --proto =https --tlsv1.2 \
        -A "dsd2pcm-rs release workflow (https://github.com/aclater/dsd2pcm-rs)" \
        "https://crates.io/api/v1/crates/$name/$v")
    if [[ "$code" == 200 ]]; then
        echo "$name $v is already on crates.io: skipped"
        continue
    fi
    echo "::group::cargo publish -p $name ($v)"
    cargo publish --locked -p "$name"
    echo "::endgroup::"
done
