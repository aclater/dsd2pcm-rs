#!/bin/bash
# Fails if tracked files or commit identities carry private details:
#
#     .github/scripts/check-private-names.sh
#
# - Patterns: names in the home.arpa special-use domain, private (RFC 1918) IPv4 addresses,
#   and the extended regular expression in $PRIVATE_NAMES (a repository variable, kept
#   outside the repository; when it is empty or unset the built-in patterns still run).
#   Matching ignores case.
# - Tracked files: every file `git ls-files` lists, except this script, which spells the
#   patterns out. Binary files are skipped.
# - Commit identities: author and committer name and e-mail address of every commit from
#   $BASE_SHA (exclusive) to HEAD, or of the whole history without it.
# Adapted from the same check in sacdfs, which skips private addresses in files; here they
# are checked in files too (none is needed in this repository's docs or tests).
set -euo pipefail

patterns='\.home\.arpa\b'
patterns+='|\b192\.168\.[0-9]{1,3}\.[0-9]{1,3}\b'
patterns+='|\b10\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\b'
patterns+='|\b172\.(1[6-9]|2[0-9]|3[01])\.[0-9]{1,3}\.[0-9]{1,3}\b'
if [[ -n "${PRIVATE_NAMES:-}" ]]; then
    patterns+="|$PRIVATE_NAMES"
fi
self=.github/scripts/check-private-names.sh
status=0

if git ls-files -z | grep -zvxF -- "$self" | xargs -0 -r grep -nIiE -- "$patterns" /dev/null; then
    echo "::error::private details in tracked files (matches above)"
    status=1
fi

range=HEAD
if [[ -n "${BASE_SHA:-}" && "$BASE_SHA" != 0000000000000000000000000000000000000000 ]] \
        && git cat-file -e "$BASE_SHA^{commit}" 2>/dev/null; then
    range="$BASE_SHA..HEAD"
fi
if git rev-parse --verify -q HEAD > /dev/null; then
    hits=$(git log --format='%h %an <%ae> %cn <%ce>' "$range" | grep -iE -- "$patterns" || true)
    if [[ -n "$hits" ]]; then
        printf '%s\n' "$hits"
        echo "::error::private details in commit identities (matches above)"
        status=1
    fi
fi

if [[ $status -eq 0 ]]; then
    echo "no private details in $(git ls-files | wc -l) tracked files or the commits of $range"
fi
exit "$status"
