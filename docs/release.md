[dsd2pcm-rs](../README.md) › [Documentation](README.md) › Releasing

# Releasing

A release is one annotated tag, `v` plus the workspace version (for example `v0.1.2`), on a
commit on `main`. Pushing the tag runs `.github/workflows/release.yml`, which builds, tests,
publishes to crates.io and creates the GitHub release. Everything else on this page is done by
the maintainer, who holds the tag, crates.io and repository-admin rights.

## What a release publishes

crates.io, in this order (each crate after the one it depends on):

1. `dsd2pcm`, the library.
2. `dsd2pcm-capi`, the C library, only when its `Cargo.toml` allows publishing. It has
   `publish = false`: the C library ships as GitHub release assets (below), not on
   crates.io.
3. `dsd2pcm-cli`, the command-line tool (`cargo install dsd2pcm-cli` installs `dsd2pcm`).

`.github/scripts/publish-crates.sh` holds this order. A crate that can be published but is not
in the list stops the release, so a new crate is never published by accident. A crate whose
version is already on crates.io is skipped, so a failed run can be re-run.

The GitHub release, with these assets:

| Asset | Platform |
|---|---|
| `dsd2pcm-0.1.2-src.tar.gz` | (from 0.1.3) The source: the tagged commit's whole tree (workspace, `fuzz/`, `docs/`) |
| `dsd2pcm-0.1.2-x86_64-linux.tar.gz` | Linux x86_64, glibc 2.17 or later |
| `dsd2pcm-0.1.2-aarch64-linux.tar.gz` | Linux aarch64, glibc 2.17 or later |
| `dsd2pcm-0.1.2-i686-linux.tar.gz` | Linux 32-bit x86 (Pentium 4 and later), glibc 2.17 or later |
| `dsd2pcm-0.1.2-x86_64-linux-musl.tar.gz` | (from 0.1.3) Linux x86_64, statically linked (musl): any Linux |
| `dsd2pcm-0.1.2-aarch64-linux-musl.tar.gz` | (from 0.1.3) Linux aarch64, statically linked (musl): any Linux |
| `dsd2pcm-0.1.2-aarch64-macos.tar.gz` | macOS 11 or later, Apple silicon; signed (Developer ID) and notarised |
| `dsd2pcm-0.1.2-x86_64-macos.tar.gz` | (from 0.1.3) macOS 11 or later, Intel; signed (Developer ID) and notarised |
| `dsd2pcm-0.1.2-x86_64-windows.zip` | Windows 10 and 11, x86_64; C runtime linked in; not code-signed |
| `libdsd2pcm-0.1.2-x86_64-linux.tar.gz` | C library, Linux x86_64, glibc 2.17 or later; static library marked for the CET shadow stack |
| `libdsd2pcm-0.1.2-aarch64-linux.tar.gz` | C library, Linux aarch64, glibc 2.17 or later |
| `libdsd2pcm-0.1.2-i686-linux.tar.gz` | C library, Linux 32-bit x86, glibc 2.17 or later |
| `libdsd2pcm-0.1.2-x86_64-linux-musl.tar.gz` | (from 0.1.3) C library, Linux x86_64, musl 1.2.3 or later (Alpine 3.17 or later); static library marked for the CET shadow stack |
| `libdsd2pcm-0.1.2-aarch64-linux-musl.tar.gz` | (from 0.1.3) C library, Linux aarch64, musl 1.2.3 or later |
| `libdsd2pcm-0.1.2-aarch64-macos.tar.gz` | C library, macOS 11 or later, Apple silicon; `libdsd2pcm.1.dylib` signed and notarised |
| `libdsd2pcm-0.1.2-x86_64-macos.tar.gz` | (from 0.1.3) C library, macOS 11 or later, Intel; `libdsd2pcm.1.dylib` signed and notarised |
| `libdsd2pcm-0.1.2-x86_64-windows.zip` | C library, Windows 10 and 11, x86_64, for MSVC |
| `libdsd2pcm-0.1.2-x86_64-windows-gnu.zip` | (from 0.1.3) C library, Windows 10 and 11, x86_64, for MinGW-w64 gcc (msvcrt) |

A `dsd2pcm-` archive holds `dsd2pcm` (or `dsd2pcm.exe`), `README.md`, `LICENSE-MIT` and
`LICENSE-APACHE`. A `libdsd2pcm-` archive holds `include/dsd2pcm.h`, the shared library
under its ABI-versioned name (`libdsd2pcm.so.1` with a `libdsd2pcm.so` link, SONAME
`libdsd2pcm.so.1`; `libdsd2pcm.1.dylib` with a `libdsd2pcm.dylib` link, install name
`@rpath/libdsd2pcm.1.dylib`; `bin/dsd2pcm.dll` with the import library
`lib/dsd2pcm.dll.lib`, or `lib/libdsd2pcm.dll.a` for MinGW-w64), the static library,
`lib/pkgconfig/dsd2pcm.pc` (not for MSVC), `examples/minimal.c`, a README taken from
[docs/c-api.md](c-api.md#binary-releases) and the licences. The C library holds no
third-party code: it is built without the `flac` feature, and
`.github/scripts/check-capi-no-claxon.sh` checks the static library of every archive but the
MSVC one for claxon symbols with `nm` (in `check-capi-archive.sh`, after its own tests on
Linux and macOS). The command-line tool
has eight platforms and the C library nine: the MinGW-w64 build is the C library only (the MSVC
`dsd2pcm.exe` serves Windows). `dsd2pcm-0.1.2-src.tar.gz` is
`git archive --format=tar --prefix=dsd2pcm-0.1.2/ v0.1.2 | gzip -n -9`: the files carry the
commit's time, so the same command on the tag gives the same tar (and, with GNU gzip, the
same bytes); the workflow checks that the C library builds from it (`cargo build --release
--locked -p dsd2pcm-capi` in the unpacked folder). Every archive has a `.sha256` file and a
build provenance attestation (`.intoto.jsonl`, also stored with GitHub). `SHA256SUMS` lists every archive:
55 files in all (18 archives, 18 `.sha256`, 18 `.intoto.jsonl`, `SHA256SUMS`).

## How the workflow runs

1. **version and tag**: every crate has the workspace version (in `Cargo.toml` and
   `Cargo.lock`), crates that use `dsd2pcm` pin it exactly (`=0.1.2`, through
   `dsd2pcm.workspace = true`), the tag is `v` plus that version, it is an annotated tag on
   the run's commit, and that commit is on `main` (`git merge-base --is-ancestor` against a
   fresh fetch of `main`). Then `cargo publish --dry-run` packages and verifies every crate.
2. **binary (nine platforms)**: build and test on each platform's own runner (Linux with
   cargo-zigbuild against glibc 2.17's symbols or zig's musl), pack the tool and the C
   library, convert a generated test file with the tool, and build and run
   `crates/dsd2pcm-capi/tests/minimal.c` against the packed C library, shared and static
   (through its pkg-config file, with MSVC for the MSVC Windows build), checking the
   SONAME (Linux) or install name (macOS). The C library is also linked into a shared
   library of the test's own with its symbols hidden, as [c-api.md](c-api.md#embedding-the-static-library)
   advises, which fails if the static library is not position-independent
   (`.github/scripts/check-capi-archive.sh`). Where each runs:

   | Platform | Build and test | C library check |
   |---|---|---|
   | x86_64-linux, i686-linux, x86_64-linux-musl | `ubuntu-24.04` | the runner (`gcc -m32` for i686); musl in Alpine 3.17 and 3.24 containers, where the static tool converts the test file too |
   | aarch64-linux, aarch64-linux-musl | `ubuntu-24.04-arm` | the runner; musl in Alpine 3.17 and 3.24 containers, where the static tool converts the test file too |
   | aarch64-macos | `macos-15` | `macos-15` |
   | x86_64-macos | `macos-15-intel` (Intel, native) | `macos-15-intel` |
   | x86_64-windows | `windows-2025`, MSVC | MSVC `cl` |
   | x86_64-windows-gnu | `windows-2025`, MSYS2 MINGW64 gcc | MSYS2 MINGW64 gcc and pkgconf |

   GitHub keeps `macos-15-intel` until August 2027; after that the x86_64 macOS build moves
   to an arm64 runner (cross-compiled, tests under Rosetta).
   On x86_64 Linux, every object of the static library is marked for the CET shadow stack
   (`.github/scripts/clib-shstk.py`, from scarletbook-rs); the archive check requires the
   mark on every object and on a program object linked with it
   ([c-api.md](c-api.md#intel-cet)).
   On macOS this is three jobs. `binary (aarch64-macos)` and `binary (x86_64-macos)` build
   and test, with no secrets. `sign and notarise (macos)` runs on a tag in the environment
   `release` (so it waits for approval too): it signs `dsd2pcm` and `libdsd2pcm.1.dylib` of
   both architectures with the Developer ID certificate (`codesign --options runtime
   --timestamp`, in a temporary keychain deleted afterwards), sends one zip of the four
   files to Apple's notary service (`notarytool submit --wait`), checks the signatures and
   the notarisation, and packs the four archives. The notary submission and its log are kept
   as the workflow artifact `macos-notarisation`. `check (aarch64-macos)` and
   `check (x86_64-macos)` unpack the archives on a fresh runner of that architecture and check the signatures
   and notarisation again (`.github/scripts/check-macos-signature.sh`: strict signature,
   Developer ID, hardened runtime, timestamp, `codesign --check-notarization`, `spctl`) before
   the smoke tests. Bare Mach-O files cannot be stapled: macOS looks the ticket up online.
   A tag without the signing secrets fails here, before anything is published; a dry run
   skips signing with a notice.
3. **same output everywhere**: all eighteen archives (with the source archive, job
   `source archive`) are there and match their `.sha256`,
   and all nine binaries made byte-identical output from the test file.
4. **publish to crates.io** (tag only): in the GitHub environment `release`, so it waits for
   a required reviewer's approval. Trusted publishing: the job's OIDC token is exchanged for
   a crates.io token valid for 30 minutes and revoked when the job ends. No crates.io token
   is stored in the repository.
5. **GitHub release** (tag only, after the crates): checksums, attestation, release.

A pull request that changes the release files, or a manual run (Actions, release, "Run
workflow"), is a dry run: steps 1 to 3, nothing published.

Permissions: every workflow reads only (`contents: read`). Only `sign and notarise
(macos)` sees the six `MACOS_*` secrets, and only in its signing step. The crates job adds
`id-token: write` (for crates.io). The release job adds `contents: write` (the release),
`id-token: write` and `attestations: write` (the attestation). Every action is pinned by
commit SHA; checkouts keep no credentials.

## One-time setup on GitHub (maintainer)

1. **Environment `release`** (Settings, Environments, New environment, `release`):
   - Required reviewers: the maintainer (and other reviewers if wanted). Leave "Prevent
     self-review" off if one person both pushes tags and approves.
   - Deployment branches and tags: "Selected branches and tags", add the tag rule `v*`.
   - Secrets for macOS signing and notarisation (crates.io's trusted publishing needs none):

     | Secret | Content |
     |---|---|
     | `MACOS_CERT_P12_BASE64` | the Developer ID Application certificate and its private key, as a .p12 file, base64 |
     | `MACOS_CERT_PASSWORD` | the .p12 file's password |
     | `MACOS_SIGN_IDENTITY` | the certificate's name, `Developer ID Application: <name> (<team ID>)` |
     | `MACOS_NOTARY_KEY_ID` | the App Store Connect API key's ID |
     | `MACOS_NOTARY_ISSUER` | the API key's issuer ID |
     | `MACOS_NOTARY_KEY_P8_BASE64` | the API key's .p8 file, base64 |

     The API key needs the Developer role (enough for `notarytool`). Environment secrets
     reach only jobs that name the environment, and only on `v*` tags (the deployment rule
     above); pull requests and manual runs never see them. Never add these as repository
     secrets: a manual run from any branch could then read them.
   - Repository variable (Settings, Secrets and variables, Actions, Variables) `MACOS_TEAM_ID`:
     the Apple team ID of the Developer ID certificate (the 10 characters in brackets in
     `MACOS_SIGN_IDENTITY`). The signature checks accept only that team. It is not secret.
2. **Tag ruleset** (Settings, Rules, Rulesets, New tag ruleset): target `v*`; restrict
   creations to the maintainer (bypass list), restrict updates, restrict deletions.
   Release tags are not signed: this ruleset and the build provenance attestations are what
   make a tag trustworthy.
3. **Branch ruleset for `main`**: pull requests required, branches up to date, with these required
   checks: `fmt + clippy + docs`, `tests (x86_64-linux)`, `tests (aarch64-linux)`,
   `tests (i686-linux)`, `tests (aarch64-macos)`, `tests (x86_64-windows)`, `C library`,
   `msrv`, `cargo-deny`, `zizmor`, `analyze (rust)`, `analyze (actions)`. `fuzz` is not
   required (a fuzzer can find an old bug in any run).
4. **Actions settings**: workflow permissions "Read repository contents" (the default for new
   repositories); "Allow GitHub Actions to create and approve pull requests" off.
5. **Code scanning** (CodeQL, Scorecard) needs the repository to be public.

## First publish to crates.io (maintainer, first release)

crates.io can configure trusted publishing only for a crate that already exists ("Your crate
must already be published to crates.io (initial publish requires an API token)",
crates.io/docs/trusted-publishing). So version 0.1.0 is published by hand with a token, and
the tag's workflow then finds both crates on crates.io and skips them (it asks crates.io for
no token when there is nothing to publish).

1. On crates.io, Account Settings, API Tokens: a new token with the scopes `publish-new` and
   `publish-update`, for the crates `dsd2pcm` and `dsd2pcm-cli`, expiring in one day.
2. From a clean clone of the release commit:

   ```sh
   git clone https://github.com/aclater/dsd2pcm-rs.git
   cd dsd2pcm-rs
   cargo publish --dry-run --locked -p dsd2pcm -p dsd2pcm-cli
   cargo login
   cargo publish --locked -p dsd2pcm
   cargo publish --locked -p dsd2pcm-cli
   cargo logout
   ```

   `cargo login` reads the token from the terminal. Publish `dsd2pcm` first: `dsd2pcm-cli`
   depends on it.
3. For each of `dsd2pcm` and `dsd2pcm-cli`, on crates.io, the crate's Settings, Trusted
   Publishing, Add, GitHub:
   - Repository owner: `aclater`
   - Repository name: `dsd2pcm-rs`
   - Workflow filename: `release.yml`
   - Environment: `release`
4. On the same page, tick "Require trusted publishing for all new versions" for both crates.
5. Revoke the token (Account Settings, API Tokens).
6. Tag the same commit (next section). The workflow builds and creates the GitHub release;
   its crates.io job reports both crates as already published.

A later new crate (for example `dsd2pcm-capi` when it becomes publishable) needs the same
steps 1 to 5 for that crate alone.

## Each release (maintainer)

1. On `main`, through a pull request: set `version` under `[workspace.package]` and the
   `version` of `dsd2pcm` under `[workspace.dependencies]` in `Cargo.toml` to the new
   version (`"0.1.2"` and `"=0.1.2"` for 0.1.2), run `cargo update --workspace` and
   `cargo update --manifest-path fuzz/Cargo.toml -p dsd2pcm`, turn `## Unreleased` in
   CHANGELOG.md into the version and date (`## 0.1.2 - 2026-11-23` for 0.1.2), and update the version in README and docs (the
   `versions` check of `docs.yml` lists every place). CI must be green on that commit; the
   Quick start check skips with a notice until the tag is released.
2. Optional: a dry run of the release workflow on `main` (Actions, release, Run workflow).
3. Tag the release commit, the `main` commit that CI and a security review approved, and
   push the tag:

   ```sh
   git fetch origin
   git tag -a v0.1.2 -m "dsd2pcm 0.1.2" origin/main
   git push origin v0.1.2
   ```

   Check first that `git log -1 origin/main` is the approved commit.
4. Approve the `release` environment twice when a job waits for it (Actions, the run, Review
   deployments): `sign and notarise (macos)` early in the run, then
   `publish to crates.io` once every build is checked. Notarisation usually takes a few
   minutes.
5. Check: the crates on crates.io, the GitHub release with its 55 files (eighteen
   archives, their `.sha256` and `.intoto.jsonl` files, and `SHA256SUMS`), and an asset:

   ```sh
   gh release download v0.1.2 -R aclater/dsd2pcm-rs -p dsd2pcm-0.1.2-x86_64-linux.tar.gz
   gh attestation verify dsd2pcm-0.1.2-x86_64-linux.tar.gz -R aclater/dsd2pcm-rs --signer-workflow aclater/dsd2pcm-rs/.github/workflows/release.yml --source-ref refs/tags/v0.1.2 --deny-self-hosted-runners
   ```

   From 0.1.3 on, also run the source archive's commands in [c-api.md](c-api.md) (download,
   `sha256sum -c`, `gh attestation verify`, unpack, build) against the new tag.
6. Release notes: the workflow writes the same notes for every release. Put the version's
   CHANGELOG section on top of them, from a clone of the release commit:

   ```sh
   awk '/^## 0\.1\.2 /{f=1; next} /^## /{f=0} f' CHANGELOG.md > notes.md
   gh release view v0.1.2 -R aclater/dsd2pcm-rs --json body -q .body >> notes.md
   gh release edit v0.1.2 -R aclater/dsd2pcm-rs --notes-file notes.md
   ```

7. Yank: when the release fixes security issues, yank the affected earlier versions of both
   crates on crates.io, after the checks in step 5 passed. For 0.1.2, which fixes 0.1.0 and
   0.1.1, with a crates.io token that has the `yank` scope for both crates (Account
   Settings, API Tokens; revoke it afterwards):

   ```sh
   cargo login
   cargo yank --version 0.1.0 dsd2pcm
   cargo yank --version 0.1.1 dsd2pcm
   cargo yank --version 0.1.0 dsd2pcm-cli
   cargo yank --version 0.1.1 dsd2pcm-cli
   cargo logout
   ```

   A yanked version stays downloadable for lock files that already name it; new
   dependency resolution skips it. The CHANGELOG section of the release says which versions
   are yanked.

If a run fails after the crates job, re-run the failed jobs: published crates are skipped.
A tag is never moved or re-created (the ruleset forbids it); a broken release gets a new
patch version.

## See also

- [C interface](c-api.md#binary-releases): what the C library archives hold
- [Development](development.md): CI and the checks before a release
- [Security policy](../SECURITY.md): supported versions

Part of the [DSD Toolbox](https://github.com/aclater/dsd-toolbox).
