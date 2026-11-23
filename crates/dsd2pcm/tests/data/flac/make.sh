#!/bin/sh
# Regenerates the FLAC test files from the sample formula in tests/flac.rs (`sample`).
# Needs the reference `flac` encoder (Fedora package `flac`, 1.4 or later) and python3.
# Without a local `flac`, run it in a container:
#   podman run --rm -v "$PWD":/w:Z -w /w registry.fedoraproject.org/fedora:45 \
#     sh -c 'dnf -y -q install flac python3 && sh make.sh'
set -eu
cd "$(dirname "$0")"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

enc() { # name bits channels rate frames [flac options]
    name=$1 bits=$2 ch=$3 rate=$4 n=$5
    shift 5
    python3 -I gen.py "$bits" "$ch" "$rate" "$n" "$tmp/in.wav"
    flac -s -f -b 1152 --no-padding "$@" -o "$name" "$tmp/in.wav"
}

enc s16_2ch_44100.flac 16 2 44100 4100 -T TITLE=test -P 100
enc s24_6ch_96000.flac 24 6 96000 1500
enc s8_1ch_8000.flac 8 1 8000 1200
enc s12_1ch_22050.flac 12 1 22050 1200
enc s20_2ch_192000.flac 20 2 192000 1200
enc s32_1ch_48000.flac 32 1 48000 100
