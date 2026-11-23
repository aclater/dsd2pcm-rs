# Provenance

| Item | Source | Licence | Used for |
|---|---|---|---|
| `crates/dsd2pcm/src/decimator/tables.rs` | scarletbook-pcm 0.7.0, `src/tables.rs`, copied unchanged (Copyright (c) 2026 The scarletbook-rs authors) | MIT OR Apache-2.0 | Filter taps of the DSD64 to PCM decimator |
| `crates/dsd2pcm/src/decimator/mod.rs` | Ported from scarletbook-pcm 0.7.0, `src/kernel.rs` and `src/lib.rs`: stage plan, S1 tables (built per 13-bit chunk here), half-to-even rounding, 24-bit quantiser, `db_to_linear` (Copyright (c) 2026 The scarletbook-rs authors). The streaming driver is new. | MIT OR Apache-2.0 | DSD64 to PCM conversion |
| `crates/dsd2pcm/src/decimator/kernel.rs` | Ported from scarletbook-pcm 0.7.0, `src/kernel.rs`: the block kernels (Fir1, Dec2, Interp2, generic over block size and multiply-add), quantiser, 24-bit interleave (Copyright (c) 2026 The scarletbook-rs authors). The S1 tables with 13-bit indices are new; they give the same sums as the original byte tables. | MIT OR Apache-2.0 | DSD64 to PCM conversion |
| `crates/dsd2pcm/src/decimator/dispatch.rs` | Ported from scarletbook-pcm 0.7.0, `src/dispatch.rs`: run-time detection and `#[target_feature]` builds of the safe kernels (Copyright (c) 2026 The scarletbook-rs authors). The separate AVX2 level, the per-stage FMA flag and the level cap are new. | MIT OR Apache-2.0 | x86 SIMD dispatch |
| `crates/dsd2pcm/src/decimator/exact.rs` | The bounds of scarletbook-pcm 0.7.0, `src/design.rs` (`f64_sums_are_exact`; Copyright (c) 2026 The scarletbook-rs authors), turned into a per-stage proof that runs on every plan | MIT OR Apache-2.0 | Exactness proof gating FMA |
| `crates/dsd2pcm/src/frames.rs` | Ported from scarletbook-pcm 0.7.0, `src/lib.rs`: the public frame-run interface and its semantics (`Converter`, `Run`, `FrameRun`, `Frames`, `PcmOut`, `PcmConfig`, `Gain`, `ConvertStats`, `pack_s24le`, input and output length checks; Copyright (c) 2026 The scarletbook-rs authors). The block driver over this crate's decimator is new. | MIT OR Apache-2.0 | Frame-run DSD64 to PCM conversion |
| DSF reader/writer | Sony, "DSF File Format Specification" v1.01 | Public specification | `dsf.rs` |
| DSDIFF reader/writer | Philips, "DSDIFF File Format Specification" 1.5 | Public specification | `dff.rs` |
| WAV reader/writer | Microsoft/IBM RIFF WAVE and `WAVE_FORMAT_EXTENSIBLE` documentation | Public specification | `wav.rs` |

scarletbook-pcm's own sources (listed in its README): Parks & McClellan 1972; Rabiner,
McClellan & Parks 1975; Herrmann, Rabiner & Chan 1973; Crochiere & Rabiner 1981; Mintzer
1982; Vaidyanathan & Nguyen 1987; Peled & Liu 1974; White 1989; Goldberg 1991.

scarletbook-pcm 0.7.0 is also a dev-dependency: the tests check that the output samples
are identical.
