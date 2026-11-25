"""Fail when the release version disagrees between the manifests, the lock files, the
changelog and the docs.

    python3 -I .github/scripts/check-doc-versions.py [REPO_ROOT]

The version is `[workspace.package] version` in Cargo.toml. It must also be:

- the exact pin of `dsd2pcm` under `[workspace.dependencies]` (`=X.Y.Z`);
- the version of the workspace crates in Cargo.lock and of `dsd2pcm` in fuzz/Cargo.lock;
- the newest version section of CHANGELOG.md (only `## Unreleased` may come before it);
- every release reference in README.md, SECURITY.md and docs/: release download URLs and
  tags (`vX.Y.Z`), archive names (`dsd2pcm-X.Y.Z-...`, `libdsd2pcm-X.Y.Z-...`), the C
  example's output (`dsd2pcm X.Y.Z:`), and in docs/release.md the example versions
  (`vX.Y.Z`, `"X.Y.Z"`, `=X.Y.Z`);
- the supported series in SECURITY.md (`X.Y.x`);
- the `version:` of CITATION.cff.

Between a version bump and its tag the docs already name the new version, whose release
does not exist yet: this check only compares text, and the Quick start check
(readme-quick-start.sh) skips while the tag is missing. Mentions of older versions as
history (for example "0.1.0 sizes" of the C ABI, or CHANGELOG sections) are not release
references and are not checked. Standard library only (Python 3.11 or later).
"""

import re
import sys
import tomllib
from pathlib import Path

V = r"(\d+\.\d+\.\d+)"
PLATFORMS = r"(?:x86_64-linux|aarch64-linux|i686-linux|aarch64-macos|x86_64-windows)"

# Release references in any doc.
DOC_PATTERNS = [
    re.compile(r"releases/download/v" + V),
    re.compile(r"releases/tag/v" + V),
    re.compile(r"refs/tags/v" + V),
    re.compile(r"\b(?:lib)?dsd2pcm-" + V + r"-" + PLATFORMS),
    re.compile(r"\bdsd2pcm " + V + r":"),
    re.compile(r"gh release (?:download|view) v" + V),
    re.compile(r"git (?:tag -a|push origin) v" + V),
]
# docs/release.md: every tag-like and pin-like version is an example of the current one.
RELEASE_MD_PATTERNS = [
    re.compile(r"(?<![\w.])v" + V + r"\b"),
    re.compile(r'"=?' + V + r'"'),
    re.compile(r"`=" + V + r"`"),
]
# Files that must hold at least one release reference, so a reworded doc cannot slip out
# of the check unnoticed.
MUST_MATCH = ["README.md", "docs/c-api.md", "docs/release.md"]


def main() -> int:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    errors = []

    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    version = manifest["workspace"]["package"]["version"]
    print(f"workspace version: {version}")

    pin = manifest["workspace"]["dependencies"]["dsd2pcm"].get("version")
    if pin != f"={version}":
        errors.append(f"Cargo.toml: [workspace.dependencies] dsd2pcm is {pin!r}, not '={version}'")

    members = set()
    for member in manifest["workspace"]["members"]:
        crate = tomllib.loads((root / member / "Cargo.toml").read_text(encoding="utf-8"))
        members.add(crate["package"]["name"])

    def lock_versions(path):
        lock = tomllib.loads((root / path).read_text(encoding="utf-8"))
        return {p["name"]: p["version"] for p in lock.get("package", []) if "source" not in p}

    for name, ver in sorted(lock_versions("Cargo.lock").items()):
        if name in members and ver != version:
            errors.append(f"Cargo.lock: {name} {ver}, not {version} (run cargo update --workspace)")
    if (root / "fuzz/Cargo.lock").exists():
        ver = lock_versions("fuzz/Cargo.lock").get("dsd2pcm")
        if ver is not None and ver != version:
            errors.append(f"fuzz/Cargo.lock: dsd2pcm {ver}, not {version} (cargo update --manifest-path fuzz/Cargo.toml -p dsd2pcm)")

    # CHANGELOG.md: the first section heading after any "Unreleased" one.
    newest = None
    for line in (root / "CHANGELOG.md").read_text(encoding="utf-8").splitlines():
        m = re.match(r"^## +\[?([^\]\s]+)\]?", line)
        if not m:
            continue
        if m.group(1).lower() == "unreleased":
            continue
        newest = m.group(1)
        break
    if newest != version:
        errors.append(f"CHANGELOG.md: newest version section is {newest!r}, not {version}")

    docs = [root / "README.md", root / "SECURITY.md"] + sorted((root / "docs").rglob("*.md"))
    found = {}
    for path in docs:
        if not path.exists():
            continue
        rel = path.relative_to(root).as_posix()
        patterns = DOC_PATTERNS + (RELEASE_MD_PATTERNS if rel == "docs/release.md" else [])
        for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            for pat in patterns:
                for m in pat.finditer(line):
                    found[rel] = found.get(rel, 0) + 1
                    if m.group(1) != version:
                        errors.append(f"{rel}:{n}: {m.group(0)!r} names {m.group(1)}, not {version}")
    for rel in MUST_MATCH:
        if not found.get(rel):
            errors.append(f"{rel}: no release reference found; update the patterns in {Path(__file__).name}")

    citation = root / "CITATION.cff"
    if citation.exists():
        m = re.search(r'^version:\s*"?([^"\s]+)"?\s*$', citation.read_text(encoding="utf-8"), re.M)
        if m is None:
            errors.append("CITATION.cff: no version")
        elif m.group(1) != version:
            errors.append(f"CITATION.cff: version {m.group(1)}, not {version}")

    security = root / "SECURITY.md"
    if security.exists():
        series = ".".join(version.split(".")[:2]) + ".x"
        if series not in security.read_text(encoding="utf-8"):
            errors.append(f"SECURITY.md: the supported versions do not name {series}")

    for e in errors:
        print(f"::error::{e}")
    if errors:
        print(f"{len(errors)} version mismatches")
        return 1
    print("versions agree: " + ", ".join(f"{k} ({v})" for k, v in sorted(found.items())))
    return 0


if __name__ == "__main__":
    sys.exit(main())
