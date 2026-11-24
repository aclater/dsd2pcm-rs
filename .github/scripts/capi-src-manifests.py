"""Write the manifests and lock file of the C library's source archive
(libdsd2pcm-<version>-src.tar.gz, made by make-capi-src.sh).

    python3 -I .github/scripts/capi-src-manifests.py SRC OUT

SRC holds the repository's Cargo.toml, crates/dsd2pcm/Cargo.toml and
crates/dsd2pcm-capi/Cargo.toml (a checkout, or the files git archive took from the release
commit). OUT receives the archive's Cargo.toml, Cargo.lock and the two crates' Cargo.toml.
Prints the version.

The archive must build with `cargo build --offline --locked` and an empty CARGO_HOME, so its
lock file may list only the two crates. Cargo resolves optional and dev-dependencies too,
even when nothing turns them on, and fails offline without a registry index. So:

- the workspace has the two crates only, with the same [workspace.package], the dsd2pcm
  entry of [workspace.dependencies] (default features off) and the same [profile] tables;
- dsd2pcm loses its dev-dependencies (the archive has no tests), the optional claxon and the
  `flac` feature that turns it on (the C library is built without it: docs/c-api.md), and
  declares cfg(feature = "flac") as known, so the sources compile without a warning;
- dsd2pcm-capi loses its `flac` feature, which only forwarded dsd2pcm's.

Every other key is kept as it is. Anything this script does not know (a new dependency, a
new top-level table, a feature that needs `flac`) stops it, so a change to the manifests is
looked at before it reaches the archive. The written files are read back and must parse to
exactly the transformed tables. Standard library only (Python 3.11 or later).
"""

import copy
import json
import sys
import tomllib
from pathlib import Path

CRATES = ("dsd2pcm", "dsd2pcm-capi")
# Keys a crate manifest may have; anything else stops the script.
CRATE_KEYS = {"package", "lib", "features", "dependencies", "dev-dependencies"}
FLAC_CFG = {"level": "warn", "check-cfg": ['cfg(feature, values("flac"))']}


class Refused(Exception):
    pass


def need(cond, msg):
    if not cond:
        raise Refused(msg)


def workspace(root):
    need(set(root) <= {"workspace", "profile"}, f"Cargo.toml: unknown tables {sorted(set(root) - {'workspace', 'profile'})}")
    ws = root["workspace"]
    need(set(ws) <= {"resolver", "members", "package", "dependencies"}, f"[workspace]: unknown keys {sorted(set(ws) - {'resolver', 'members', 'package', 'dependencies'})}")
    for c in CRATES:
        need(f"crates/{c}" in ws.get("members", []), f"[workspace] members: no crates/{c}")
    dep = ws.get("dependencies", {}).get("dsd2pcm")
    need(isinstance(dep, dict) and dep.get("path") == "crates/dsd2pcm", "[workspace.dependencies] dsd2pcm: not the path crates/dsd2pcm")
    need(dep.get("default-features") is False, "[workspace.dependencies] dsd2pcm: default features must be off")
    need(set(dep) <= {"path", "version", "default-features"}, f"[workspace.dependencies] dsd2pcm: unknown keys {sorted(dep)}")
    out = {"workspace": {"resolver": ws["resolver"], "members": [f"crates/{c}" for c in CRATES]}}
    if "package" in ws:
        out["workspace"]["package"] = copy.deepcopy(ws["package"])
    out["workspace"]["dependencies"] = {"dsd2pcm": copy.deepcopy(dep)}
    if "profile" in root:
        out["profile"] = copy.deepcopy(root["profile"])
    return out


def strip_flac(name, m):
    """The manifest without the `flac` feature; features that need it stop the script."""
    feats = m.get("features", {})
    for f, v in feats.items():
        if f in ("flac", "default"):
            continue
        need(not any(x == "flac" or x.endswith("/flac") or x == "dep:claxon" for x in v), f"{name}: feature {f} needs flac")
    feats.pop("flac", None)
    if "default" in feats:
        feats["default"] = [x for x in feats["default"] if x != "flac"]


def crate(name, m):
    need(set(m) <= CRATE_KEYS, f"{name}: unknown tables {sorted(set(m) - CRATE_KEYS)}")
    m = copy.deepcopy(m)
    m.pop("dev-dependencies", None)
    strip_flac(name, m)
    deps = m.get("dependencies", {})
    if name == "dsd2pcm":
        claxon = deps.pop("claxon", None)
        need(claxon is None or (isinstance(claxon, dict) and claxon.get("optional") is True), "dsd2pcm: claxon is not optional")
        need(not deps, f"dsd2pcm: dependencies the archive cannot build offline: {sorted(deps)}")
        m.pop("dependencies", None)
        m["lints"] = {"rust": {"unexpected_cfgs": copy.deepcopy(FLAC_CFG)}}
    else:
        need(deps == {"dsd2pcm": {"workspace": True}}, f"{name}: dependencies other than the workspace's dsd2pcm: {sorted(deps)}")
    for f, v in m.get("features", {}).items():
        need(all(not x.startswith("dep:") for x in v), f"{name}: feature {f} turns on a dependency")
    return m


# A minimal TOML writer for what Cargo manifests hold here: strings, booleans, integers,
# arrays of those, and tables. A table under the top level (or under workspace, profile and
# lints) gets a [header]; deeper tables are written inline.
SECTION_PARENTS = {(), ("workspace",), ("profile",), ("lints",)}


def value(v):
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, str):
        return json.dumps(v)  # ASCII, with \" \\ \uXXXX escapes: a valid TOML basic string
    if isinstance(v, list):
        return "[" + ", ".join(value(x) for x in v) + "]"
    if isinstance(v, dict):
        return "{ " + ", ".join(f"{key(k)} = {value(x)}" for k, x in v.items()) + " }" if v else "{}"
    raise Refused(f"cannot write {type(v).__name__} {v!r}")


def key(k):
    return k if k and all(c.isalnum() or c in "-_" for c in k) else json.dumps(k)


def write_table(path, t, lines):
    subs = []
    body = []
    for k, v in t.items():
        if isinstance(v, dict) and path in SECTION_PARENTS:
            subs.append((k, v))
        else:
            body.append(f"{key(k)} = {value(v)}")
    if path and (body or not subs):
        lines.append("")
        lines.append("[" + ".".join(key(p) for p in path) + "]")
    lines.extend(body)
    for k, v in subs:
        write_table(path + (k,), v, lines)


def dump(doc, header):
    lines = [f"# {h}" if h else "#" for h in header]
    write_table((), doc, lines)
    return "\n".join(lines) + "\n"


LOCK = """# This file is automatically @generated by Cargo.
# It is not intended for manual editing.
version = 4

[[package]]
name = "dsd2pcm"
version = "{v}"

[[package]]
name = "dsd2pcm-capi"
version = "{v}"
dependencies = [
 "dsd2pcm",
]
"""


def main(argv):
    if len(argv) != 3:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    src, out = Path(argv[1]), Path(argv[2])
    root = tomllib.loads((src / "Cargo.toml").read_text())
    version = root["workspace"]["package"]["version"]
    header = [
        f"Generated for the C library's source archive (libdsd2pcm-{version}-src.tar.gz) by",
        ".github/scripts/capi-src-manifests.py from the repository's manifest: see README.md.",
    ]
    docs = {Path("Cargo.toml"): workspace(root)}
    for c in CRATES:
        m = tomllib.loads((src / "crates" / c / "Cargo.toml").read_text())
        need(m["package"]["name"] == c, f"crates/{c}/Cargo.toml: package {m['package']['name']}")
        docs[Path("crates") / c / "Cargo.toml"] = crate(c, m)
    for rel, doc in docs.items():
        text = dump(doc, header)
        need(tomllib.loads(text) == doc, f"{rel}: the written file does not read back as written")
        (out / rel).parent.mkdir(parents=True, exist_ok=True)
        (out / rel).write_text(text)
    (out / "Cargo.lock").write_text(LOCK.format(v=version))
    print(version)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv))
    except Refused as e:
        print(f"capi-src-manifests.py: {e}", file=sys.stderr)
        sys.exit(1)
