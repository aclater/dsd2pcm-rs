#!/bin/bash
# Tests of capi-src-manifests.py: on the repository's manifests it must remove exactly the
# flac feature, claxon, the dev-dependencies and the other workspace members, and keep
# everything else; on manifests it does not know (a new dependency, claxon made required, a
# feature needing flac, a new table, default features on) it must refuse with status 1, so
# a manifest change cannot slip into the source archive unchecked.
#
#     .github/scripts/capi-src-manifests-test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failures=0

# fixture NAME: a copy of the repository's three manifests in $work/NAME
fixture() {
    mkdir -p "$work/$1/crates/dsd2pcm" "$work/$1/crates/dsd2pcm-capi" "$work/$1.out"
    cp "$repo/Cargo.toml" "$work/$1/"
    cp "$repo/crates/dsd2pcm/Cargo.toml" "$work/$1/crates/dsd2pcm/"
    cp "$repo/crates/dsd2pcm-capi/Cargo.toml" "$work/$1/crates/dsd2pcm-capi/"
}
# expect STATUS NAME DESCRIPTION
expect() {
    local got=0
    python3 -I "$here/capi-src-manifests.py" "$work/$2" "$work/$2.out" > "$work/$2.log" 2>&1 || got=$?
    # A refusal is the script's own message, not a traceback (which also exits 1).
    if [[ "$got" -eq "$1" ]] && { [[ "$got" -eq 0 ]] || grep -q '^capi-src-manifests.py: ' "$work/$2.log"; } \
        && ! grep -q '^Traceback' "$work/$2.log"; then
        echo "ok: $3 (status $got: $(tail -n 1 "$work/$2.log"))"
    else
        echo "::error::$3: status $got, expected $1"
        cat "$work/$2.log"
        failures=$((failures + 1))
    fi
}

fixture real
expect 0 real "the repository's manifests"
# What was removed is exactly what should be, and the rest is unchanged.
if python3 -I - "$work/real" "$work/real.out" <<'EOF'; then echo "ok: only flac, claxon, dev-dependencies and members changed"; else echo "::error::the transform changed more or less than it should"; failures=$((failures + 1)); fi
import sys, tomllib
from pathlib import Path
src, out = Path(sys.argv[1]), Path(sys.argv[2])
def load(p):
    return tomllib.loads(p.read_text())
root, root_out = load(src / "Cargo.toml"), load(out / "Cargo.toml")
assert root_out["workspace"]["members"] == ["crates/dsd2pcm", "crates/dsd2pcm-capi"]
assert root_out["workspace"]["package"] == root["workspace"]["package"]
assert root_out["workspace"]["dependencies"] == {"dsd2pcm": root["workspace"]["dependencies"]["dsd2pcm"]}
assert root_out["profile"] == root["profile"]
lib, lib_out = load(src / "crates/dsd2pcm/Cargo.toml"), load(out / "crates/dsd2pcm/Cargo.toml")
assert "claxon" in lib["dependencies"] and "flac" in lib["features"] and "dev-dependencies" in lib
assert "dependencies" not in lib_out and "dev-dependencies" not in lib_out
assert "flac" not in lib_out["features"] and "flac" not in lib_out["features"]["default"]
assert lib_out["package"] == lib["package"]
assert set(lib_out) == {"package", "features", "lints"}, set(lib_out)
capi, capi_out = load(src / "crates/dsd2pcm-capi/Cargo.toml"), load(out / "crates/dsd2pcm-capi/Cargo.toml")
assert "flac" in capi["features"] and "flac" not in capi_out["features"]
del capi["features"]["flac"]
assert capi_out == capi, (capi_out, capi)
lock = tomllib.loads((out / "Cargo.lock").read_text())
v = root["workspace"]["package"]["version"]
assert [(p["name"], p["version"]) for p in lock["package"]] == [("dsd2pcm", v), ("dsd2pcm-capi", v)]
EOF

fixture newdep
sed -i 's/^\[dependencies\]$/&\nlibc = "0.2"/' "$work/newdep/crates/dsd2pcm/Cargo.toml"
grep -q '^libc' "$work/newdep/crates/dsd2pcm/Cargo.toml" || { echo "::error::fixture newdep: no edit"; exit 1; }
expect 1 newdep "a new dependency of dsd2pcm"

fixture required
sed -i 's/, optional = true//' "$work/required/crates/dsd2pcm/Cargo.toml"
grep -q '^claxon = { version = "[^"]*" }$' "$work/required/crates/dsd2pcm/Cargo.toml" || { echo "::error::fixture required: no edit"; exit 1; }
expect 1 required "claxon made a required dependency"

fixture needsflac
sed -i 's/^flac = \["dep:claxon"\]$/&\nwav-and-flac = ["flac"]/' "$work/needsflac/crates/dsd2pcm/Cargo.toml"
grep -q '^wav-and-flac' "$work/needsflac/crates/dsd2pcm/Cargo.toml" || { echo "::error::fixture needsflac: no edit"; exit 1; }
expect 1 needsflac "a feature that needs flac"

fixture patch
printf '\n[patch.crates-io]\nclaxon = { path = "../claxon" }\n' >> "$work/patch/Cargo.toml"
expect 1 patch "a new top-level table in the workspace manifest"

fixture defaults
sed -i 's/, default-features = false }/ }/' "$work/defaults/Cargo.toml"
grep -q 'default-features' "$work/defaults/Cargo.toml" && { echo "::error::fixture defaults: no edit"; exit 1; }
expect 1 defaults "the workspace's dsd2pcm with default features"

fixture capidep
sed -i 's/^dsd2pcm.workspace = true$/&\nlibc = "0.2"/' "$work/capidep/crates/dsd2pcm-capi/Cargo.toml"
grep -q '^libc' "$work/capidep/crates/dsd2pcm-capi/Cargo.toml" || { echo "::error::fixture capidep: no edit"; exit 1; }
expect 1 capidep "a new dependency of dsd2pcm-capi"

[[ "$failures" -eq 0 ]] || { echo "$failures failed"; exit 1; }
echo "all passed"
