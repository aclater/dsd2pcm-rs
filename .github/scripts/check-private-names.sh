#!/bin/bash
# Fails if tracked files or commit identities carry private details:
#
#     .github/scripts/check-private-names.sh
#
# - Patterns: names in the home.arpa special-use domain, private (RFC 1918) IPv4 addresses,
#   and the extended regular expression in $PRIVATE_NAMES (a repository variable, kept
#   outside the repository; when it is empty or unset the built-in patterns still run).
#   Matching ignores case. A $PRIVATE_NAMES that matches the empty string (a stray `|`,
#   only spaces) or is not a valid regex is refused: it would match every line, or nothing.
# - Tracked files: every file in the index (`git grep`), except this script and its test,
#   which spell the patterns out. Binary files are skipped.
# - Commit identities: author and committer name and e-mail address of every commit from
#   $BASE_SHA (exclusive) to HEAD, or of the whole history without it.
# Exit status: 0 clean, 1 private details found, 2 the check itself failed (a bad regex, a
# git error). An error never passes. Tests: check-private-names-test.sh.
# Adapted from the same check in sacdfs, which skips private addresses in files; here they
# are checked in files too (none is needed in this repository's docs or tests).
set -euo pipefail

fail() {
    echo "::error::the private-names check itself failed: $*"
    exit 2
}

cd "$(git rev-parse --show-toplevel)" || fail "not in a git work tree"
patterns='\.home\.arpa\b'
patterns+='|\b192\.168\.[0-9]{1,3}\.[0-9]{1,3}\b'
patterns+='|\b10\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\b'
patterns+='|\b172\.(1[6-9]|2[0-9]|3[01])\.[0-9]{1,3}\.[0-9]{1,3}\b'
if [[ -n "${PRIVATE_NAMES:-}" ]]; then
    [[ -n "${PRIVATE_NAMES//[[:space:]]/}" ]] || fail "PRIVATE_NAMES is only white space"
    # grep: 0 = the regex matches an empty line, 1 = it does not, 2 = it is invalid.
    rc=0
    printf '\n' | grep -qiE -e "$PRIVATE_NAMES" 2> /dev/null || rc=$?
    case $rc in
        0) fail "PRIVATE_NAMES matches the empty string (a stray '|' or only spaces?)" ;;
        1) patterns+="|$PRIVATE_NAMES" ;;
        *) fail "PRIVATE_NAMES is not a valid extended regular expression" ;;
    esac
fi
status=0

# git grep: 0 = a match, 1 = none, anything else = an error.
rc=0
git grep -nIiE -e "$patterns" -- . \
    ':(exclude).github/scripts/check-private-names.sh' \
    ':(exclude).github/scripts/check-private-names-test.sh' || rc=$?
case $rc in
    0) echo "::error::private details in tracked files (matches above)"; status=1 ;;
    1) ;;
    *) fail "git grep exited with $rc" ;;
esac

range=HEAD
if [[ -n "${BASE_SHA:-}" && "$BASE_SHA" != 0000000000000000000000000000000000000000 ]] \
        && git cat-file -e "$BASE_SHA^{commit}" 2> /dev/null; then
    range="$BASE_SHA..HEAD"
fi
if git rev-parse --verify -q HEAD > /dev/null; then
    log=$(git log --format='%h %an <%ae> %cn <%ce>' "$range") || fail "git log exited with $?"
    rc=0
    hits=$(printf '%s\n' "$log" | grep -iE -e "$patterns") || rc=$?
    case $rc in
        0) printf '%s\n' "$hits"; echo "::error::private details in commit identities (matches above)"; status=1 ;;
        1) ;;
        *) fail "grep exited with $rc" ;;
    esac
fi

if [[ $status -eq 0 ]]; then
    echo "no private details in $(git ls-files | wc -l) tracked files or the commits of $range"
fi
exit "$status"
