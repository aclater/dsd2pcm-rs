# Fuzzing

The `fuzz/` directory holds [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets.
It is a separate crate, outside the main workspace, and needs a nightly toolchain.

| Target | Input | Checks |
|---|---|---|
| `dsf_read` | arbitrary bytes as DSF | no panic; reads to the end with small, odd, zero and oversized request sizes; delivers exactly `ceil(samples / 8)` bytes per channel; reading never fails once the headers are accepted; heap growth stays proportional to the input |
| `dff_read` | arbitrary bytes as DSDIFF | as `dsf_read` |
| `wav_read` | arbitrary bytes as WAV | as `dsf_read`, in frames; every sample is finite |
| `round_trip` | arbitrary audio, channel counts, rates, write sizes and DSF metadata | the DSF, DFF and WAV writers' output reads back unchanged |

## Setup

```
rustup toolchain install nightly
cargo install cargo-fuzz
```

## Seed corpora

Small valid files made by the crate's own writers. From the repository root:

```
cd fuzz
cargo run --release --bin make_seeds
```

This writes `fuzz/corpus/dsf_read`, `fuzz/corpus/dff_read` and `fuzz/corpus/wav_read`.
The corpora and any `fuzz/artifacts` are not committed.

## Running

From the `fuzz` directory, one target for ten minutes on four processes:

```
cargo +nightly fuzz run -O dsf_read -- -fork=4 -max_total_time=600 -max_len=32768 -rss_limit_mb=2048 -malloc_limit_mb=2048 -timeout=10
```

Replace `dsf_read` with `dff_read`, `wav_read` or `round_trip` for the other targets.
A crash is saved under `fuzz/artifacts/`, and cargo-fuzz prints the exact command that
replays it. Every fixed finding also gets a regression test in `crates/dsd2pcm/tests/`, so
`cargo test` covers it without the fuzzer.

## Coverage

```
cargo +nightly fuzz coverage dsf_read
```
