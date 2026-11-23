#!/bin/bash
# Check a macOS release file (the dsd2pcm tool or libdsd2pcm.1.dylib) the way Gatekeeper sees
# it, for .github/workflows/release.yml:
#
#     .github/scripts/check-macos-signature.sh FILE
#
# - the signature is valid under strict checking;
# - it is a Developer ID Application signature with the hardened runtime and a secure
#   timestamp;
# - Apple's notarisation ticket for it is found online (codesign --check-notarization with
#   the requirement `notarized`; a bare Mach-O file has no stapled ticket). The lookup is
#   retried a few times, as a fresh ticket can take a moment to appear;
# - Gatekeeper (spctl) accepts it.
# Needs no credentials. Exits non-zero on the first failure.
set -euo pipefail

f=$1
fail() { echo "::error::$f: $*"; exit 1; }

codesign --verify --strict --verbose=2 "$f"
info=$(codesign --display --verbose=4 "$f" 2>&1)
echo "$info" | grep -E '^(Identifier|Format|CodeDirectory|Authority|Timestamp|TeamIdentifier|Runtime Version)='
grep -q '^Authority=Developer ID Application: ' <<< "$info" || fail "not signed with a Developer ID Application certificate"
grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' <<< "$info" || fail "no hardened runtime"
grep -q '^Timestamp=' <<< "$info" || fail "no secure timestamp"

ok=false
for attempt in 1 2 3 4 5 6; do
    if codesign --verify --strict --check-notarization -R='notarized' --verbose=2 "$f"; then
        ok=true
        break
    fi
    echo "notarisation ticket not found yet (attempt $attempt); retrying in 20 s"
    sleep 20
done
$ok || fail "Apple's notarisation ticket was not found"

spctl --assess --type open --context context:primary-signature --verbose=2 "$f" 2>&1 | tee "${TMPDIR:-/tmp}/spctl.txt"
grep -q 'Notarized Developer ID' "${TMPDIR:-/tmp}/spctl.txt" || fail "Gatekeeper does not report a notarised Developer ID signature"
echo "$f: Developer ID, hardened runtime, timestamp, notarised"
