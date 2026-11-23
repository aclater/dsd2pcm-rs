#!/usr/bin/env bash
# Runs the README's Quick start literally, as a user would on Linux x86_64.
#
#   .github/scripts/readme-quick-start.sh [README]
#
# The commands are the first fenced block under "## Quick start", taken from the README
# itself, so the check cannot drift from the text. They run with `bash -e` in an empty
# directory that holds what the README says to have there: a DSF file named
# "01 - Track.dsf" (.github/scripts/make-test-dsf.py). Then the output must be what the
# README says: a 176.4 kHz, 24-bit WAV.
#
# Between a version bump and its tag the README names a release that does not exist yet:
# when the block's download URL is missing and the tag it names is not in the repository,
# the check passes with a notice. A missing download for an existing tag fails.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
readme=$(realpath "${1:-$root/README.md}")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# The block: from the first fence after the heading to the closing fence.
awk '
    /^## / { on = ($0 == "## Quick start"); next }
    on && !inblock && /^```/ { inblock = 1; next }
    on && inblock && /^```/ { exit }
    on && inblock { print }
' "$readme" > "$work/quick-start.sh"
if [[ ! -s "$work/quick-start.sh" ]]; then
    echo "::error::README has no fenced block under \"## Quick start\""
    exit 1
fi
echo "== the README's Quick start"
cat "$work/quick-start.sh"

url=$(grep -o 'https://github.com/[^ ]*/releases/download/[^ ]*' "$work/quick-start.sh" | head -n 1 || true)
if [[ -n "$url" ]] && ! curl -fsSIL --retry 3 -o /dev/null "$url"; then
    repo=${url%%/releases/download/*}
    tag=$(sed -n 's#.*/releases/download/\([^/]*\)/.*#\1#p' <<< "$url")
    if git ls-remote --exit-code --tags "$repo.git" "refs/tags/$tag" > /dev/null; then
        echo "::error::$url does not download, but the tag $tag exists"
        exit 1
    fi
    echo "::notice::$tag is not released yet (between the version bump and the tag): Quick start not run"
    exit 0
fi

mkdir "$work/run"
python3 -I "$root/.github/scripts/make-test-dsf.py" "$work/run/01 - Track.dsf"
echo "== running it"
(cd "$work/run" && bash -e "$work/quick-start.sh")

echo "== the output"
python3 -I - "$work/run/01 - Track.wav" <<'EOF'
import struct, sys

data = open(sys.argv[1], "rb").read()
if data[:4] != b"RIFF" or data[8:12] != b"WAVE":
    sys.exit("::error::the output is not a WAV file")
pos = 12
while pos + 8 <= len(data):
    cid, size = data[pos:pos + 4], struct.unpack_from("<I", data, pos + 4)[0]
    if cid == b"fmt ":
        channels, rate = struct.unpack_from("<HI", data, pos + 10)
        bits = struct.unpack_from("<H", data, pos + 22)[0]
        print(f"{channels} channels, {rate} Hz, {bits} bits")
        if (rate, bits) != (176400, 24):
            sys.exit("::error::the README promises a 176.4 kHz, 24-bit WAV")
        break
    pos += 8 + size + (size & 1)
else:
    sys.exit("::error::the output has no fmt chunk")
EOF
echo "Quick start: OK"
