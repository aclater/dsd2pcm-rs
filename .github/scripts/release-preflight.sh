#!/bin/bash
# Checks before a release builds anything (.github/workflows/release.yml, job "preflight"):
#
# - every crate of the workspace has the workspace's version, in Cargo.toml and Cargo.lock,
#   and the crates that depend on dsd2pcm pin exactly that version;
# - on a tag (REF_TYPE=tag): the tag is vVERSION; it is an annotated tag whose signature
#   GitHub verified (a key on a maintainer's account); and its commit is on main.
#
# Environment: REF_TYPE, REF_NAME (github.ref_type, github.ref_name), and on a tag GH_REPO,
# GH_TOKEN (read access) and TAG_SHA (the commit). Prints the version and, under GitHub
# Actions, writes version=... to $GITHUB_OUTPUT. Needs python3 (tomllib) and, on a tag, gh
# and git.
set -euo pipefail

fail() { echo "::error::$*"; exit 1; }

version=$(python3 -I - <<'EOF'
import json, subprocess, sys, tomllib

with open("Cargo.toml", "rb") as f:
    version = tomllib.load(f)["workspace"]["package"]["version"]
meta = json.loads(subprocess.run(
    ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
    check=True, capture_output=True, text=True).stdout)
with open("Cargo.lock", "rb") as f:
    locked = {p["name"]: p["version"] for p in tomllib.load(f)["package"] if "source" not in p}
bad = []
for p in meta["packages"]:
    if p["version"] != version:
        bad.append(f'{p["name"]} is {p["version"]}, the workspace {version}')
    if locked.get(p["name"]) != version:
        bad.append(f'Cargo.lock has {p["name"]} {locked.get(p["name"])}')
    for d in p["dependencies"]:
        if d["name"] == "dsd2pcm" and d["kind"] is None and d["req"] != f"={version}":
            bad.append(f'{p["name"]} depends on dsd2pcm {d["req"]}, not ={version}')
for b in bad:
    print(f"::error::{b}", file=sys.stderr)
if bad:
    sys.exit(1)
print(version)
EOF
)
echo "version: $version"

if [[ "${REF_TYPE:-}" == tag ]]; then
    [[ "$REF_NAME" == "v$version" ]] || fail "tag $REF_NAME does not match the workspace version $version (expected v$version)"

    # Annotated and signed: GitHub's verification of the tag object.
    ref=$(gh api "repos/$GH_REPO/git/ref/tags/$REF_NAME" --jq '.object.type + " " + .object.sha')
    [[ "${ref%% *}" == tag ]] || fail "$REF_NAME is a lightweight tag: release tags are annotated and signed (git tag -s)"
    verified=$(gh api "repos/$GH_REPO/git/tags/${ref#* }" --jq '.verification.verified, .verification.reason, .object.sha')
    read -r ok reason commit <<< "$(echo "$verified" | tr '\n' ' ')"
    [[ "$ok" == true ]] || fail "the signature of tag $REF_NAME is not verified by GitHub ($reason): see docs/release.md, \"Tag signing\""
    [[ "$commit" == "$TAG_SHA" ]] || fail "tag $REF_NAME points at $commit, the run at $TAG_SHA"
    echo "tag $REF_NAME: annotated, signature verified ($reason)"

    # On main: never a commit only on a pull request's branch, or one never pushed.
    # The workflow checks out with full history (fetch-depth: 0), which includes origin/main.
    git rev-parse --verify --quiet refs/remotes/origin/main > /dev/null \
        || git fetch --quiet --no-tags origin "+refs/heads/main:refs/remotes/origin/main"
    git merge-base --is-ancestor "$TAG_SHA" refs/remotes/origin/main \
        || fail "tag $REF_NAME ($TAG_SHA) is not on main"
    echo "tag $REF_NAME: on main"
else
    echo "::notice::dry run of $version from ${REF_NAME:-a local tree}: nothing is published"
fi
[[ -n "${GITHUB_OUTPUT:-}" ]] && echo "version=$version" >> "$GITHUB_OUTPUT"
exit 0
