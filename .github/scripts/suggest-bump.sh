#!/usr/bin/env bash
#
# Print the version bump implied by a range of commits, as `level=` and
# `reason=` lines suitable for appending to $GITHUB_OUTPUT.
#
# Usage: suggest-bump.sh <git-range> [extra-subject]
#
# The optional extra subject lets a pull request title participate in the
# decision even when none of its commits carry a conventional prefix.

set -euo pipefail

range="$1"
extra="${2:-}"

messages="$(git log --no-merges --format='%s%n%b' "$range"; printf '%s\n' "$extra")"

if grep -qE '^[a-zA-Z]+(\([^)]*\))?!:' <<<"$messages" \
  || grep -qE '^BREAKING[ -]CHANGE' <<<"$messages"; then
  level=major
  reason='a change is marked breaking (`!:` or a `BREAKING CHANGE:` footer)'
elif grep -qE '^feat(\([^)]*\))?:' <<<"$messages"; then
  level=minor
  reason='a change uses `feat:`'
else
  level=patch
  reason='nothing is marked `feat:` or breaking'
fi

echo "level=$level"
echo "reason=$reason"
