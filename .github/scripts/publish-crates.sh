#!/bin/bash
# Publish the workspace's crates to crates.io, in dependency order:
#
#     dsd2pcm  ->  dsd2pcm-capi (only when its Cargo.toml allows publishing)  ->  dsd2pcm-cli
#
#     .github/scripts/publish-crates.sh [--dry-run | --pending]
#
# Crates whose Cargo.toml says `publish = false` are skipped; a publishable crate missing from
# the order above stops the script, so a new crate is never published by accident. A version
# that crates.io already has is skipped, so a run that failed half-way can be re-run.
# --dry-run packages and verifies every crate (cargo publish --workspace --dry-run, which
# builds each against the others' packages, so it needs nothing on crates.io) and uploads
# nothing. --pending only prints the crates crates.io does not have yet at this version and,
# under GitHub Actions, writes pending=true or false to $GITHUB_OUTPUT. Without either,
# CARGO_REGISTRY_TOKEN must hold a token (the release workflow's comes from trusted
# publishing).
set -euo pipefail

order=(dsd2pcm dsd2pcm-capi dsd2pcm-cli)

mode=${1:-publish}

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

if [[ "$mode" == --dry-run ]]; then
    args=()
    for name in "${list[@]}"; do args+=(-p "$name"); done
    cargo publish --dry-run --locked "${args[@]}"
    exit 0
fi

on_crates_io() {
    local code
    code=$(curl -sS -o /dev/null -w '%{http_code}' --proto =https --tlsv1.2 \
        -A "dsd2pcm-rs release workflow (https://github.com/aclater/dsd2pcm-rs)" \
        "https://crates.io/api/v1/crates/$1/$2")
    case "$code" in
        200) return 0 ;;
        404) return 1 ;;
        *) echo "::error::crates.io answered $code for $1 $2"; exit 1 ;;
    esac
}

pending=()
for name in "${list[@]}"; do
    if on_crates_io "$name" "${version[$name]}"; then
        echo "$name ${version[$name]} is already on crates.io: skipped"
    else
        pending+=("$name")
    fi
done
echo "to publish: ${pending[*]:-nothing}"
if [[ "$mode" == --pending ]]; then
    [[ -n "${GITHUB_OUTPUT:-}" ]] && echo "pending=$([[ ${#pending[@]} -gt 0 ]] && echo true || echo false)" >> "$GITHUB_OUTPUT"
    exit 0
fi
[[ "$mode" == publish ]] || { echo "usage: publish-crates.sh [--dry-run | --pending]" >&2; exit 2; }

for name in "${pending[@]}"; do
    echo "::group::cargo publish -p $name (${version[$name]})"
    # --no-verify: preflight already ran the verifying --dry-run on this commit, and a
    # verify build here would run every build script with the publish token in the env.
    cargo publish --locked --no-verify -p "$name"
    echo "::endgroup::"
done
