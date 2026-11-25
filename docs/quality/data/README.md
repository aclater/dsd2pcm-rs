[dsd2pcm-rs](../../../README.md) › [Documentation](../../README.md) › [Release quality comparison](../README.md) › Measurement data

# Measurement data

Checksums behind measurements that [PCM quality](../../pcm-quality.md) publishes, so that a
rerun can be checked file by file.

## foobar2000 SACD plug-in re-run (2026-11-24)

The foobar2000 SACD plug-in, foo_input_sacd 2.0.27 in foobar2000 2.26.3, run as a black box
on Windows 11 in a virtual machine. It converted the test files of `tools/quality` in three
settings: Multistage (32fp) at 352.8 and 176.4 kHz, and Direct (32fp, 30kHz lowpass) at
352.8 kHz. The settings are listed under
[comparisons](../../pcm-quality.md#comparisons).

- [foo_input_sacd-2.0.27-inputs.sha256](foo_input_sacd-2.0.27-inputs.sha256): the 72 DSF
  inputs: 54 made by the reference modulator of `tools/quality` and 18 by SoX-DSD (commit
  0be259e of mansr/sox, as `gen_signals.py --sox` runs it).
- [foo_input_sacd-2.0.27-outputs.sha256](foo_input_sacd-2.0.27-outputs.sha256): the 288
  output WAVs (32-bit float), 72 per folder. The folders are named `<mode>-<rate>`, and
  `multistage-32fp-352k8-repeat` is a second conversion of that setting. Its files are
  identical to the first conversion's.

Both files are in `sha256sum` format; check them with `sha256sum -c` in the folder that
holds the files. The WAVs themselves are not distributed (8.3 GB).

## Licence

The data here is licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/)
([LICENSES/CC-BY-4.0.txt](../../../LICENSES/CC-BY-4.0.txt)), like the rest of the
documentation.

## See also

- [PCM quality](../../pcm-quality.md#comparisons): the plug-in's measured figures
- [Quality tools](../../../tools/quality/README.md): the scripts that made the inputs and
  measured the outputs

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
