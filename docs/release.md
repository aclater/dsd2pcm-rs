# Releasing

A release is one annotated tag, `v` plus the workspace version (for example `v0.1.2`), on a
commit on `main`. Pushing the tag runs `.github/workflows/release.yml`, which builds, tests,
publishes to crates.io and creates the GitHub release. Everything else on this page is done by
a person (the Orchestrator, who holds the tag, crates.io and repository-admin rights).

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
| `dsd2pcm-0.1.2-x86_64-linux.tar.gz` | Linux x86_64, glibc 2.17 or later |
| `dsd2pcm-0.1.2-aarch64-linux.tar.gz` | Linux aarch64, glibc 2.17 or later |
| `dsd2pcm-0.1.2-i686-linux.tar.gz` | Linux 32-bit x86 (Pentium 4 and later), glibc 2.17 or later |
| `dsd2pcm-0.1.2-aarch64-macos.tar.gz` | macOS 11 or later, Apple silicon; signed (Developer ID) and notarised |
| `dsd2pcm-0.1.2-x86_64-windows.zip` | Windows 10 and 11, x86_64; C runtime linked in; not code-signed |
| `libdsd2pcm-0.1.2-x86_64-linux.tar.gz` | C library, Linux x86_64, glibc 2.17 or later |
| `libdsd2pcm-0.1.2-aarch64-linux.tar.gz` | C library, Linux aarch64, glibc 2.17 or later |
| `libdsd2pcm-0.1.2-i686-linux.tar.gz` | C library, Linux 32-bit x86, glibc 2.17 or later |
| `libdsd2pcm-0.1.2-aarch64-macos.tar.gz` | C library, macOS 11 or later, Apple silicon; `libdsd2pcm.1.dylib` signed and notarised |
| `libdsd2pcm-0.1.2-x86_64-windows.zip` | C library, Windows 10 and 11, x86_64 |

A `dsd2pcm-` archive holds `dsd2pcm` (or `dsd2pcm.exe`), `README.md`, `LICENSE-MIT` and
`LICENSE-APACHE`. A `libdsd2pcm-` archive holds `include/dsd2pcm.h`, the shared library
under its ABI-versioned name (`libdsd2pcm.so.1` with a `libdsd2pcm.so` link, SONAME
`libdsd2pcm.so.1`; `libdsd2pcm.1.dylib` with a `libdsd2pcm.dylib` link, install name
`@rpath/libdsd2pcm.1.dylib`; `bin/dsd2pcm.dll` with the import library
`lib/dsd2pcm.dll.lib`), the static library, `lib/pkgconfig/dsd2pcm.pc` (not on Windows),
`examples/minimal.c`, a README taken from [docs/c-api.md](c-api.md#binary-releases) and the
licences. Every archive has a `.sha256` file and a build provenance attestation
(`.intoto.jsonl`, also stored with GitHub). `SHA256SUMS` lists every archive: 31 files in
all.

## How the workflow runs

1. **version and tag**: every crate has the workspace version (in `Cargo.toml` and
   `Cargo.lock`), crates that use `dsd2pcm` pin it exactly (`=0.1.2`, through
   `dsd2pcm.workspace = true`), the tag is `v` plus that version, it is an annotated tag on
   the run's commit, and that commit is on `main` (`git merge-base --is-ancestor` against a
   fresh fetch of `main`). Then `cargo publish --dry-run` packages and verifies every crate.
2. **binary (five platforms)**: build and test on each platform's own runner (Linux with
   cargo-zigbuild against glibc 2.17's symbols), pack the tool and the C library, convert
   a generated test file with the packed tool, and build and run
   `crates/dsd2pcm-capi/tests/minimal.c` against the packed C library, shared and static
   (through its pkg-config file on Linux and macOS, with MSVC on Windows), checking the
   SONAME (Linux) or install name (macOS).
   On macOS this is three jobs. `binary (aarch64-macos)` builds and tests, with no secrets.
   `sign and notarise (aarch64-macos)` runs on a tag in the environment `release` (so it
   waits for approval too): it signs `dsd2pcm` and `libdsd2pcm.1.dylib` with the Developer ID
   certificate (`codesign --options runtime --timestamp`, in a temporary keychain deleted
   afterwards), sends a zip of both to Apple's notary service (`notarytool submit --wait`),
   checks the signatures and the notarisation, and packs the archives. The notary
   submission and its log are kept as the workflow artifact `macos-notarisation`.
   `check (aarch64-macos)` unpacks the archives on a fresh runner and checks the signatures
   and notarisation again (`.github/scripts/check-macos-signature.sh`: strict signature,
   Developer ID, hardened runtime, timestamp, `codesign --check-notarization`, `spctl`) before
   the smoke tests. Bare Mach-O files cannot be stapled: macOS looks the ticket up online.
   A tag without the signing secrets fails here, before anything is published; a dry run
   skips signing with a notice.
3. **same output everywhere**: all ten archives are there and match their `.sha256`, and
   all five binaries made byte-identical output from the test file.
4. **publish to crates.io** (tag only): in the GitHub environment `release`, so it waits for
   a required reviewer's approval. Trusted publishing: the job's OIDC token is exchanged for
   a crates.io token valid for 30 minutes and revoked when the job ends. No crates.io token
   is stored in the repository.
5. **GitHub release** (tag only, after the crates): checksums, attestation, release.

A pull request that changes the release files, or a manual run (Actions, release, "Run
workflow"), is a dry run: steps 1 to 3, nothing published.

Permissions: every workflow reads only (`contents: read`). Only `sign and notarise
(aarch64-macos)` sees the six `MACOS_*` secrets, and only in its signing step. The crates job adds
`id-token: write` (for crates.io). The release job adds `contents: write` (the release),
`id-token: write` and `attestations: write` (the attestation). Every action is pinned by
commit SHA; checkouts keep no credentials.

## One-time setup on GitHub (Orchestrator)

1. **Environment `release`** (Settings, Environments, New environment, `release`):
   - Required reviewers: the Orchestrator (and the owner if wanted). Leave "Prevent
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
   creations to the Orchestrator (bypass list), restrict updates, restrict deletions.
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

## First publish to crates.io (Orchestrator, launch morning)

crates.io can configure trusted publishing only for a crate that already exists ("Your crate
must already be published to crates.io (initial publish requires an API token)",
crates.io/docs/trusted-publishing). So version 0.1.0 is published by hand with a token, and
the tag's workflow then finds both crates on crates.io and skips them (it asks crates.io for
no token when there is nothing to publish).

1. On crates.io, Account Settings, API Tokens: a new token with the scopes `publish-new` and
   `publish-update`, for the crates `dsd2pcm` and `dsd2pcm-cli`, expiring in one day.
2. From a clean clone of the release commit:

   ```
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

## Each release (Orchestrator)

1. On `main`, through a pull request: set `version` under `[workspace.package]` and the
   `version` of `dsd2pcm` under `[workspace.dependencies]` in `Cargo.toml` to the new
   version (`"0.1.2"` and `"=0.1.2"` for 0.1.2), run `cargo update --workspace` and
   `cargo update --manifest-path fuzz/Cargo.toml -p dsd2pcm`, give the `## Unreleased`
   section of CHANGELOG.md the version, and update the version in README and docs (the
   `versions` check of `docs.yml` lists every place). CI must be green on that commit; the
   Quick start check skips with a notice until the tag is released.
2. Optional: a dry run of the release workflow on `main` (Actions, release, Run workflow).
3. Tag the release commit, the `main` commit that CI and the Security Auditor approved, and
   push the tag:

   ```
   git fetch origin
   git tag -a v0.1.2 -m "dsd2pcm 0.1.2" origin/main
   git push origin v0.1.2
   ```

   Check first that `git log -1 origin/main` is the approved commit.
4. Approve the `release` environment twice when a job waits for it (Actions, the run, Review
   deployments): `sign and notarise (aarch64-macos)` early in the run, then
   `publish to crates.io` once every build is checked. Notarisation usually takes a few
   minutes.
5. Check: the crates on crates.io, the GitHub release with its 31 files (ten
   archives, their `.sha256` and `.intoto.jsonl` files, and `SHA256SUMS`), and an asset:

   ```
   gh release download v0.1.2 -R aclater/dsd2pcm-rs -p dsd2pcm-0.1.2-x86_64-linux.tar.gz
   gh attestation verify dsd2pcm-0.1.2-x86_64-linux.tar.gz -R aclater/dsd2pcm-rs --signer-workflow aclater/dsd2pcm-rs/.github/workflows/release.yml --source-ref refs/tags/v0.1.2 --deny-self-hosted-runners
   ```

If a run fails after the crates job, re-run the failed jobs: published crates are skipped.
A tag is never moved or re-created (the ruleset forbids it); a broken release gets a new
patch version.
