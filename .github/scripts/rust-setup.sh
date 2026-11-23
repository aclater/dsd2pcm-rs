#!/bin/bash
# Rust for a CI job that runs in a UBI or Fedora container (`container:` in the workflow).
# Run as root after the checkout:
#
#     .github/scripts/rust-setup.sh TOOLCHAIN [COMPONENTS] [TARGETS]
#
# rustup-init is pinned to one version and checked against its SHA-256, then TOOLCHAIN (with
# the comma-separated COMPONENTS and TARGETS) is installed under $HOME and put on the PATH of
# the job's later steps. The packages (git, gcc) come from the job's first step.
set -euo pipefail

toolchain=$1
components=${2:-}
targets=${3:-}

rustup_version=1.29.3
case "$(uname -m)" in
    x86_64) rustup_sha256=b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55 ;;
    aarch64) rustup_sha256=9f608c142e0e38a78957d0661ba1dfaf0b6fb37184cfaa4b9a0aa57af6184282 ;;
    *) echo "no pinned rustup-init for $(uname -m)" >&2; exit 1 ;;
esac

git config --system --add safe.directory '*'
dir=$(mktemp -d)
init=$dir/rustup-init  # rustup-init acts on the name it is run as
curl -sSfL --proto =https --tlsv1.2 -o "$init" \
    "https://static.rust-lang.org/rustup/archive/$rustup_version/$(uname -m)-unknown-linux-gnu/rustup-init"
echo "$rustup_sha256  $init" | sha256sum -c
chmod 0755 "$init"
args=(-y --no-modify-path --profile minimal --default-toolchain "$toolchain")
[[ -n "$components" ]] && args+=(--component "$components")
[[ -n "$targets" ]] && args+=(--target "$targets")
"$init" "${args[@]}"
rm -rf "$dir"
echo "$HOME/.cargo/bin" >> "${GITHUB_PATH:-/dev/null}"
"$HOME/.cargo/bin/rustc" -vV
