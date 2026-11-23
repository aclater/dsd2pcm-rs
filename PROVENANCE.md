# Provenance

| Item | Source | Licence | Used for |
|---|---|---|---|
| `crates/dsd2pcm/src/decimator/tables.rs` | scarletbook-pcm 0.7.0, `src/tables.rs`, copied unchanged (Copyright (c) 2026 The scarletbook-rs authors) | MIT OR Apache-2.0 | Filter taps of the DSD64 to PCM decimator |
| `crates/dsd2pcm/src/decimator/mod.rs` | Ported from scarletbook-pcm 0.7.0, `src/kernel.rs` and `src/lib.rs`: stage plan, S1 byte tables, half-to-even rounding, 24-bit quantiser, `db_to_linear` (Copyright (c) 2026 The scarletbook-rs authors). The streaming driver is new. | MIT OR Apache-2.0 | DSD64 to PCM conversion |
| DSF reader/writer | Sony, "DSF File Format Specification" v1.01 | Public specification | `dsf.rs` |
| DSDIFF reader/writer | Philips, "DSDIFF File Format Specification" 1.5 | Public specification | `dff.rs` |
| WAV reader/writer | Microsoft/IBM RIFF WAVE and `WAVE_FORMAT_EXTENSIBLE` documentation | Public specification | `wav.rs` |

scarletbook-pcm's own sources (listed in its README): Parks & McClellan 1972; Rabiner,
McClellan & Parks 1975; Herrmann, Rabiner & Chan 1973; Crochiere & Rabiner 1981; Mintzer
1982; Vaidyanathan & Nguyen 1987; Peled & Liu 1974; White 1989; Goldberg 1991.

scarletbook-pcm 0.7.0 is also a dev-dependency: the tests check that the output samples
are identical.
