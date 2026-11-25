[dsd2pcm-rs](../README.md) › [Documentation](README.md) › Release test plan

# Release test plan

Every release runs this whole plan on its release candidate: the commit that will be tagged
(for a batch release, the head of the batch branch). The results go in a copy of the
[results table](#results-table) with the candidate's SHA. Any FAIL stops the tag until it
is fixed; after a fix, the items the fix can affect run again on the new head.

Nothing here is timed, so no item needs an idle machine. The local items run from the
repository root through `.github/scripts/release-test.sh ITEM`, which caps each heavy step's
memory (`RTP_MEM`, default 8G), runs it at nice 19 with `RTP_JOBS` build jobs (default 8),
and prints `PASS ITEM` or `FAIL ITEM` as its last line.

| Item | What | Where | Release length |
|---|---|---|---|
| 1 | Test suite on every platform and SIMD level | local (x86_64, i686, aarch64 under qemu); macOS and Windows from GitHub CI | full suite |
| 2 | Golden hashes and identity with scarletbook-pcm 0.7.0 at every level | inside item 1 | full suite |
| 3 | Fuzzing, every target | local | 30 minutes per target |
| 4 | Leak soak | local | 1 hour |
| 5 | Miri, C interface and core crate | local | complete |
| 6 | Real-file corpus | local | 15 to 20 files |
| 7 | C interface under ASan, LSan and UBSan | local | 1000 leak-loop iterations |
| 8 | README Quick start in a fresh container | local, then again after the release | once |
| 9 | Docs checks and the docs' commands | local | every command |
| 10 | File-safety regressions | local | once |
| 11 | Quality comparison with the previous release | local | the whole suite |

## Setup

A Linux x86_64 machine whose CPU has SSE2, AVX2 and FMA, about 16 threads, 32 GB of
memory, 60 GB of disk, podman (rootless is fine) and a systemd user session
(`systemd-run --user`). Then:

<!-- docs-run: skip="installs packages with sudo on the release test machine" -->
```sh
sudo dnf install gcc clang compiler-rt python3 podman file
rustup toolchain install stable
rustup target add i686-unknown-linux-gnu aarch64-unknown-linux-gnu
rustup toolchain install nightly-2026-11-01 --profile minimal --component miri,rust-src,llvm-tools-preview
cargo install cargo-fuzz
cargo install --locked cbindgen --version 0.29.4
```

lychee 0.24.2 from its GitHub release (the version and SHA-256 pinned in
`.github/workflows/docs.yml`), on the `PATH` or named by `LYCHEE`. Clone the candidate and
record its SHA:

<!-- docs-run: skip="clones the release candidate; BRANCH is the candidate branch, named at release time" -->
```sh
git clone https://github.com/dsd-toolbox/dsd2pcm-rs.git
cd dsd2pcm-rs
git checkout BRANCH
git rev-parse HEAD
```

The containers (items 1, 8) use `registry.fedoraproject.org/fedora:45`, pinned by digest in
the script (`FEDORA_IMAGE` names another image). They see the host's rustup home and only
the `registry/`, `git/` and `bin/` directories of its cargo home, all read-only, so a cargo
configuration or credentials file there stays out of the container. The script runs
`cargo fetch --locked` on the host first, and cargo runs offline inside.

## 1 and 2. Test suite on every platform; golden hashes and identity

<!-- docs-run: skip="release-test.sh runs each step under systemd-run --user (memory caps), which the docs-run container does not have; it also starts its own podman containers" -->
```sh
.github/scripts/release-test.sh simd
.github/scripts/release-test.sh i686
.github/scripts/release-test.sh aarch64
```

`simd` runs `cargo test --workspace --locked` four times, with `DSD2PCM_SIMD` set to
`generic`, `sse2`, `avx2` and `avx2fma` (`.github/scripts/test-simd-levels.sh`, which fails
if the CPU lacks a level rather than testing a lower one). `i686` does the same for
`i686-unknown-linux-gnu` in a Fedora container with the 32-bit C libraries, and checks that
the tool is a 32-bit x86 program. `aarch64` builds for `aarch64-unknown-linux-gnu` in a
Fedora container with the cross linker and glibc sysroot and runs every test under
`qemu-aarch64-static`, at its one level, `generic`, built with `--release` plus debug
assertions and overflow checks (a debug build under emulation takes hours); the tool must
be an aarch64 program. The CLI's tests start the tool as a child process, which the host
kernel's `binfmt_misc` entry for aarch64 runs under qemu (Fedora's `qemu-user-static`
registers it with the F flag, which a container can use; without that entry or its F flag
the script fails at once with a message saying so, rather than skipping the tests), with
`QEMU_LD_PREFIX` naming the sysroot. One test is left out there: the tool's signal test
(`sigint_and_sigterm_leave_no_partial_output`), which signals that child. In its place the
release build of the aarch64 tool runs under qemu on a 120-second 5.1 file and is sent
SIGINT, then SIGTERM: it must exit 130 and 143, leave the existing output as it was and
leave no temporary file. The native aarch64 CI job runs the test itself.

The suite holds item 2: `tests/identity.rs` compares the output of the fast filter with
scarletbook-pcm 0.7.0 (a dev-dependency, run in the same test) for every rate, cutoff, gain
and channel count, and the `golden_hashes_*_at_every_simd_level` tests check the eight
golden SHA-256 hashes of the fast, precise and default filters at every level the CPU has,
each forced in turn. `tests/pysacd.rs` checks the `pysacd` filter
against its reference hashes.

macOS (aarch64) and Windows (x86_64), and native aarch64 Linux: the rows come from the
GitHub CI run of the batch pull request, jobs `tests (aarch64-macos)`,
`tests (x86_64-windows)` and `tests (aarch64-linux)`, which run the same
`test-simd-levels.sh`.

Pass: every run ends `PASS`, no test fails or is skipped other than the `#[ignore]` ones,
and the three CI jobs are green on the candidate's head.

## 3. Fuzzing

<!-- docs-run: skip="release-test.sh runs each step under systemd-run --user (memory caps), which the docs-run container does not have; it fuzzes for 1800 seconds by design" -->
```sh
.github/scripts/release-test.sh fuzz 1800
```

Builds the targets with the pinned nightly, makes the seed corpora
(`fuzz/src/bin/make_seeds.rs`), and runs every target in `fuzz/fuzz_targets` (`dsf_read`,
`dff_read`, `wav_read`, `flac`, `open_buffered`, `round_trip`) at the same time, one
process each (no `-fork`), each from a copy of its seeds, with `-max_len=32768
-rss_limit_mb=2048 -malloc_limit_mb=2048 -timeout=10`, under a 3 GiB cap per target. Logs:
`target/rtp-fuzz-logs/TARGET.log`. Each fuzzer runs in its own process group; if the
script ends early (an error, Ctrl-C or SIGTERM), it stops those groups, and only those.

Pass: every target runs the full 30 minutes with no crash, leak, timeout or memory-limit
finding. A finding is a FAIL: its input is in `fuzz/artifacts/TARGET/`.

## 4. Leak soak

<!-- docs-run: skip="release-test.sh runs each step under systemd-run --user (memory caps), which the docs-run container does not have; the soak runs for an hour by design" -->
```sh
.github/scripts/release-test.sh soak 3600
```

`tests/soak.rs` for an hour ([testing.md](testing.md#soak-test) describes what it does).

Pass: the test passes: the live heap of the second half stays within 64 KiB of the peak of
20 to 50 % of the run, the resident set within its bound, and the thread count is back to
its starting value after every threaded conversion. Record the iterations, conversions,
live-heap and resident-set figures it prints.

## 5. Miri

<!-- docs-run: skip="release-test.sh runs each step under systemd-run --user (memory caps), which the docs-run container does not have" -->
```sh
.github/scripts/release-test.sh miri
```

`cargo miri test -p dsd2pcm-capi`, then `.github/scripts/miri-core.sh` (the library's unit
tests and the `containers`, `flac` and `frames` tests), on nightly-2026-11-01 at the
generic level. The tests left out under Miri are listed in [testing.md](testing.md#miri).

Pass: both finish with no undefined behaviour, no leak and no failed test. Record the
test counts.

## 6. Real-file corpus

15 to 20 DSD64 files, read-only copies of real disc rips in a work directory, chosen so
that the set has:

- DSF and DFF, stereo and 5.1, the longest tracks at hand (over 25 minutes), and at least
  one file over the 4 GiB WAV limit at 352.8 kHz;
- files that end in a partial 4704-byte frame;
- 5.1 DFF files (made from 5.1 DSF rips when there are none) and a DSF made from a DFF rip:

  <!-- docs-run: skip="needs real DSD rips (IN.dsf, IN.dff) of the tester" -->
  ```sh
  python3 -I .github/scripts/dsd-transform.py dsf2dff IN.dsf OUT.dff
  python3 -I .github/scripts/dsd-transform.py dff2dsf IN.dff OUT.dsf
  ```

- four DSF copies whose sample count is not a multiple of 8 (none of a typical library's
  rips has one), stereo and 5.1, with 1, 3, 5 and 7 padding bits set to zero, two of
  them with a silent end (the last two frames of each channel set to the idle pattern):

  <!-- docs-run: skip="needs real DSD rips (IN.dsf) of the tester" -->
  ```sh
  python3 -I .github/scripts/dsd-transform.py odd IN.dsf OUT.dsf 3
  python3 -I .github/scripts/dsd-transform.py odd IN.dsf OUT.dsf 5 9408
  ```

  The script streams, so its memory stays small for any file length. It refuses an OUT
  that is IN (the same path, or a hard or symbolic link to it), and writes OUT through a
  temporary file beside it, so a failure or interruption leaves no partial OUT.

List the inputs one per line in a file, the container pairs (a rip and the copy made from
it, `A B` per line) in another, and run:

<!-- docs-run: skip="needs the corpus of real DSD rips listed in corpus.txt" -->
```sh
RTP_PAIRS=pairs.txt .github/scripts/release-test.sh corpus corpus.txt WORKDIR
```

First a self-test: the checker passes a correct output and fails it after one flipped
byte. Then, for every file, the release build of the `dsd2pcm` tool converts it at every
rate and cutoff of the fast filter (`--filter fast`, scarletbook-pcm's filters; 88.2 kHz
with 30 and 40; 176.4 and 352.8 kHz with 30, 40 and wide), and `crates/dsd2pcm/examples/real_identity.rs` checks each output:

- it reads back as a WAV: one `fmt ` chunk (24-bit, the input's channels and the rate), one
  `data` chunk of whole sample frames, the RIFF size equal to the file size;
- every sample of the input's whole frames equals scarletbook-pcm 0.7.0's, which converts
  the same file read by the example's own parsers (not the library's readers), streamed in
  runs with their neighbour frames (the last whole frame is left out when a partial frame
  follows it, whose context scarletbook-pcm cannot take);
- the output's length is right for the input's sample count;
- a silent end (the last frame idle in every channel) ends in exact zeros: the last 64
  samples of every channel.

Then the `pysacd` filter at 88.2 and 176.4 kHz (a WAV, the same bytes threaded and with
`--single-thread`), the default settings (the precise filter) threaded and with
`--single-thread` (the same bytes), and each container pair (the same WAV, byte for byte).

Pass: every line is `OK` or an expected refusal; the only refusal is a conversion whose
WAV would pass 4 GiB, with exit status 1, the message "WAV output over 4 GiB" and no
output file. Every odd-count copy passes with its padding converted as idle, and the
silent-end copies end in zeros.

## 7. C interface under the sanitizers

<!-- docs-run: skip="release-test.sh runs each step under systemd-run --user (memory caps), which the docs-run container does not have" -->
```sh
.github/scripts/release-test.sh capi
```

`crates/dsd2pcm-capi/check.sh` (header equal to cbindgen 0.29.4's output, every C test and
the example under gcc and clang, the C++ check, the fail-closed check that the static
library holds no claxon symbols, then `smoke.c`, `struct_size.c` and `leakloop.c` under
clang's AddressSanitizer, LeakSanitizer and UndefinedBehaviorSanitizer) and
`.github/scripts/capi-check.sh` with `ASAN_CC=clang` (the CI job's check), both with
`LEAKLOOP_ITERATIONS=1000`.

Pass: both end with no sanitizer report and "C interface checks passed" and "C tests
passed under the sanitizers, no leaks".

## 8. README Quick start in a fresh container

<!-- docs-run: skip="release-test.sh runs each step under systemd-run --user (memory caps), which the docs-run container does not have; it also starts its own podman container" -->
```sh
.github/scripts/release-test.sh quickstart
```

Before the tag the release archive does not exist yet. So, in a fresh
`registry.fedoraproject.org/fedora:45` container, the tool is built from the candidate and
packed into the archive the release will publish (`dsd2pcm-X.Y.Z-x86_64-linux.tar.gz`,
with the README and licences), and the README's Quick start block runs literally except
its `curl` line, on a generated `01 - Track.dsf`. The block must name the candidate's
version.

Pass: the commands succeed and `01 - Track.wav` is a 176.4 kHz, 24-bit WAV.

After the release, the same check runs with the real download (the `curl` line too):

<!-- docs-run: checkout=1 image=fedora-net timeout=7200 setup="dnf -y -q --setopt=install_weak_deps=False install gcc gcc-c++ git-core python3 gawk pipx && .github/scripts/rust-setup.sh 1.100.0 && mkdir -p ~/.local && ln -s ~/.cargo/bin ~/.local/bin && curl -sSfL -o /tmp/lychee.tar.gz https://github.com/lycheeverse/lychee/releases/download/lychee-v0.24.2/lychee-x86_64-unknown-linux-gnu.tar.gz && echo '1f4e0ef7f6554a6ed33dd7ac144fb2e1bbed98598e7af973042fc5cd43951c9a  /tmp/lychee.tar.gz' | sha256sum -c && tar -C /tmp -xzf /tmp/lychee.tar.gz && cp /tmp/lychee-x86_64-unknown-linux-gnu/lychee ~/.cargo/bin/" -->
```sh
.github/scripts/readme-quick-start.sh
```

in a fresh container (`podman run --rm -it -v "$PWD:/src:Z" -w /src
registry.fedoraproject.org/fedora:45` with `dnf install -y python3 gawk git-core`); it
must print "Quick start: OK".

## 9. Docs

<!-- docs-run: checkout=1 -->
```sh
.github/scripts/release-test.sh docs
```

`python3 -I .github/scripts/check-doc-versions.py` (one version everywhere),
`.github/scripts/readme-quick-start.sh` (before the tag it passes with a notice that the
release does not exist yet; item 8 covers it), and lychee over README, CHANGELOG,
SECURITY, PROVENANCE, AGENTS and `docs/` offline (relative links and anchors).

Then, by hand, every command in [user-guide.md](user-guide.md), [options.md](options.md),
[c-api.md](c-api.md) and [testing.md](testing.md) runs as written, on the candidate, with the release build of the
tool on the `PATH` where a command calls `dsd2pcm`. Commands that need a release that does
not exist yet (downloads, `gh release`, `cargo install` of the new version) are recorded as
"after release" and run then. Items 4, 5 and 7 already run testing.md's soak, Miri and C
checks; its ThreadSanitizer and valgrind commands run here.

Pass: each script ends without an error, and every command does what its doc says.
Record each command and its result.

## 10. File safety

<!-- docs-run: skip="release-test.sh runs each step under systemd-run --user (memory caps), which the docs-run container does not have" -->
```sh
.github/scripts/release-test.sh safety
```

Its files, a 635 MB input among them, go in a new directory under `target/` (or under the
directory given as `safety WORKDIR`), on disk rather than in `TMPDIR`, which is often a
RAM-backed tmpfs; the directory is removed at the end. With the release build of the tool:

- an output that is a hard link to the input, the input's own path, or a symbolic link to
  it is refused with exit status 1 and the input is unchanged;
- a bad input, a missing input, a gain out of range and `--cutoff wide` at 88.2 kHz each
  fail and leave an existing output byte for byte as it was; an unknown option exits 2;
  no temporary file remains;
- a conversion interrupted while it runs (the script checks it was running when
  signalled): SIGINT exits 130 with "interrupted; no output written", SIGTERM exits 143;
  an existing output stays as it was, no output file is created and no temporary file
  remains. A second SIGINT ends the process at once (under 2 s), and SIGKILL cannot be
  caught: then no output file is created, and at most a hidden `.dsd2pcm-PID-N.tmp`
  remains, as the [user guide](user-guide.md#output-files-and-interruptions) documents;
- refused up front, leaving everything as it was: a directory, a path ending in `/`, a
  FIFO (without blocking), a device (`/dev/null`), a socket, an existing read-only file,
  and a file in a directory that cannot be written;
- replacing an output keeps its permissions (0640 stays 0640); an output name of 255
  bytes works;
- a DFF with 4096 chunks up to and including the audio chunk converts and one with 4097
  is refused; a crafted DFF with 100 000 empty chunks before `PROP` is refused (exit 1)
  within 2 seconds and leaves an existing output as it was;
- `cargo test --release -p dsd2pcm --test output --test chunk_walk` and
  `cargo test --release -p dsd2pcm-cli` (its signal tests) pass.

Pass: every line is `ok`.

## 11. Quality comparison with the previous release

Measures the candidate and the previous release with
[dsd-measure](https://github.com/dsd-toolbox/dsd-measure) 0.1.2 and
[suites/dsd2pcm.toml](../suites/dsd2pcm.toml), on one machine, and writes
`docs/quality/X.Y.Z.md` ([how the comparison is made](quality/README.md)). Nothing is timed,
so it runs in parallel with the other items. It needs Python 3.11 or later, a C compiler
and about 15 minutes with the builds (the 19 devices take about 8 minutes on an AMD Ryzen AI
Max+ 395, and the null tests can run beside them; the 0.1.1 suite's run used 1.7 GB of
memory). The example is the first comparison, 0.2.0 against 0.1.2; for a later release,
change the two tags and the file name.

Install the pinned measurement suite and build the two versions under `target/quality/`
(the candidate from its branch, the previous release from its tag):

<!-- docs-run: checkout=1 -->
```sh
pipx install git+https://github.com/dsd-toolbox/dsd-measure@3eaec2df13634f3bfc3968326a1dd7604ddfaf08
dsd-measure --version
cargo install --locked --path crates/dsd2pcm-cli --root target/quality/new
# facts:release repo=dsd-toolbox/dsd2pcm-rs fmt="cargo install --locked --git https://github.com/{repo} --tag {tag} --root target/quality/old dsd2pcm-cli"
cargo install --locked --git https://github.com/dsd-toolbox/dsd2pcm-rs --tag v0.2.0 --root target/quality/old dsd2pcm-cli
# /facts
```

`dsd-measure --version` must print `0.1.2`. Measure both, from the repository root:

<!-- docs-run: checkout=1 -->
```sh
dsd-measure run suites/dsd2pcm.toml --only reference_floor levels noise alias_tones
```

Then the null test of each filter, rate and cutoff that both versions have, new against
old, on the suite's multitone signal (`fast` is 0.1.x's default), and of the precise filter,
the default from 0.2.0, against the previous release's default (fast) at the same cutoff and
rate, until the previous release has the precise filter too. Its SHA-256 must be
`100d1ba2cb46d98374ee5b8591b995712d12cd21c7d9370d50a1d68da3c030fc`:

<!-- docs-run: checkout=1 -->
```sh
dsd-measure signals target/quality/signals --profile quick --only multitone
sha256sum target/quality/signals/multitone.dsf
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old fast 40 88200" --device "new fast 40 88200" -o target/quality/null/fast-40-88200
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old fast 40 176400" --device "new fast 40 176400" -o target/quality/null/fast-40-176400
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old fast 40 352800" --device "new fast 40 352800" -o target/quality/null/fast-40-352800
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old fast wide 176400" --device "new fast wide 176400" -o target/quality/null/fast-wide-176400
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old fast wide 352800" --device "new fast wide 352800" -o target/quality/null/fast-wide-352800
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old pysacd 88200" --device "new pysacd 88200" -o target/quality/null/pysacd-88200
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old pysacd 176400" --device "new pysacd 176400" -o target/quality/null/pysacd-176400
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old fast 40 88200" --device "new precise 40 88200" -o target/quality/null/precise-40-88200
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old fast 40 176400" --device "new precise 40 176400" -o target/quality/null/precise-40-176400
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old fast 40 352800" --device "new precise 40 352800" -o target/quality/null/precise-40-352800
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old fast wide 176400" --device "new precise wide 176400" -o target/quality/null/precise-wide-176400
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old fast wide 352800" --device "new precise wide 352800" -o target/quality/null/precise-wide-352800
```

Copy [the template](quality/template.md) to `docs/quality/0.2.0.md` and fill it from
`target/quality/report/<device>/results.json` (the keys are listed at the top of the suite
file) and the first line of each `target/quality/null/*/index.md`, and copy the raw results
(each `results.json` and `provenance.json` under `target/quality/report/` and
`target/quality/null/`) to `docs/quality/data/0.2.0/`, with the absolute paths in them made
relative to the repository root ([how](quality/README.md#raw-data)). Fill its "Measured with"
line: the candidate's commit (`git rev-parse --short HEAD`), the previous tag's commit
(`git rev-parse --short v0.1.2^{commit}`), the Rust version (`rustc --version`), the
dsd-measure version and commit, and the CPU model (`lscpu`, the "Model name" line), never
the host name.

Pass: every command exits 0, and either nothing changed (every figure the same to
0.01 dB, the suite's measured run-to-run spread being 0.00 dB, and every null "identical"), or
each change is accepted. A worse figure is a FAIL unless it is accepted. An accepted change
or regression is recorded, with its reason, in that version's `docs/quality/X.Y.Z.md` page
and in its CHANGELOG entry.

## Results table

Copy for each release; one row per item and platform, each with its evidence (the
`PASS`/`FAIL` line, counts, or the CI job's link).

```text
Release: X.Y.Z    Candidate: BRANCH at SHA    Date:    Tester:

| Item | Platform / part | Result | Evidence |
|---|---|---|---|
| 1 | x86_64 Linux: generic, sse2, avx2, avx2fma | | |
| 1 | i686 Linux: generic, sse2, avx2, avx2fma | | |
| 1 | aarch64 Linux under qemu: generic | | |
| 1 | aarch64 Linux, native (batch CI) | | |
| 1 | aarch64 macOS (batch CI) | | |
| 1 | x86_64 Windows (batch CI) | | |
| 2 | golden hashes and scarletbook-pcm identity, every level above | | |
| 3 | fuzz: dsf_read, dff_read, wav_read, flac, open_buffered, round_trip (30 min each) | | |
| 4 | soak, 1 h | | |
| 5 | Miri: dsd2pcm-capi | | |
| 5 | Miri: core (miri-core.sh) | | |
| 6 | real-file corpus (N files) | | |
| 7 | C interface: check.sh, capi-check.sh with ASan/LSan/UBSan | | |
| 8 | README Quick start, from the tree (before tag) | | |
| 8 | README Quick start, download (after tag) | | |
| 9 | check-doc-versions.py, readme-quick-start.sh, lychee offline | | |
| 9 | user-guide.md, options.md, c-api.md, testing.md commands | | |
| 10 | file safety | | |
| 11 | quality comparison with the previous release (docs/quality/X.Y.Z.md) | | |
```

## See also

- [Releases](release.md): the release workflow that follows the plan
- [Testing](testing.md): the soak, Miri and sanitizer runs the items use
- [Release quality comparison](quality/README.md): the item 11 pages
- [Development](development.md): the checks CI runs on every pull request

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
