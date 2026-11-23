[dsd2pcm-rs](../README.md) › [Documentation](README.md) › Fuzzing

# Fuzzing

The `fuzz/` directory holds [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets.
It is a separate crate, outside the main workspace, and needs a nightly toolchain.

| Target | Input | Checks |
|---|---|---|
| `dsf_read` | arbitrary bytes as DSF | no panic; reads to the end with small, odd, zero and oversized request sizes; delivers exactly `ceil(samples / 8)` bytes per channel; reading never fails once the headers are accepted; heap growth stays proportional to the input |
| `dff_read` | arbitrary bytes as DSDIFF | as `dsf_read` |
| `wav_read` | arbitrary bytes as WAV | as `dsf_read`, in frames; every sample is finite |
| `flac` | arbitrary bytes as FLAC | no panic; reads to the end with the same request sizes; delivers exactly the declared frame count unless a read fails (frames are decoded as they are read, so damaged audio is an error there); samples within full scale; memory bounded by one decoded block and one read's output |
| `open_buffered` | arbitrary bytes, opened by every reader through a 1 MiB `BufReader` | opening reads the input a bounded number of times over (a reader that seeks once per chunk would refill the buffer for every chunk) |
| `round_trip` | arbitrary audio, channel counts, rates, write sizes and DSF metadata | the DSF, DFF and WAV writers' output reads back unchanged |

Its dependencies are checked with their own cargo-deny config, `fuzz/deny.toml`, which
also allows libFuzzer's NCSA licence; from the repository root:

```sh
cargo deny --manifest-path fuzz/Cargo.toml check
```

## Setup

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
```

## Seed corpora

Small valid files made by the crate's own writers, and the FLAC test files of
`crates/dsd2pcm/tests/data/flac` with unknown-length and ID3 variants. From the repository
root:

```sh
cd fuzz
cargo run --release --bin make_seeds
```

This writes `fuzz/corpus/dsf_read`, `fuzz/corpus/dff_read`, `fuzz/corpus/wav_read` and
`fuzz/corpus/flac`.
The corpora and any `fuzz/artifacts` are not committed.

## Running

From the `fuzz` directory, one target for ten minutes on four processes:

```sh
cargo +nightly fuzz run -O dsf_read -- -fork=4 -max_total_time=600 -max_len=32768 -rss_limit_mb=2048 -malloc_limit_mb=2048 -timeout=10
```

Replace `dsf_read` with `dff_read`, `wav_read`, `flac` or `round_trip` for the other
targets.
A crash is saved under `fuzz/artifacts/`, and cargo-fuzz prints the exact command that
replays it. Every fixed finding also gets a regression test in `crates/dsd2pcm/tests/`, so
`cargo test` covers it without the fuzzer.

## Coverage

```sh
rustup component add llvm-tools-preview --toolchain nightly
cargo +nightly fuzz coverage dsf_read
```

This replays the corpus and writes `fuzz/coverage/dsf_read/coverage.profdata` for
`llvm-cov`.

## See also

- [Development](development.md): the other checks
- [Design](design.md#robustness): how the parsers guard against bad input
- [Security policy](../SECURITY.md): reporting a vulnerability

Part of the [DSD Toolbox](https://github.com/aclater/dsd-toolbox).
