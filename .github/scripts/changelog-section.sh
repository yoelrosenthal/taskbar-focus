#!/usr/bin/env bash
#
# Print one Keep a Changelog section for a release.
#
# Usage: changelog-section.sh <version> <git-range> <iso-date>
#
# Commits carrying a conventional `feat:` or `fix:` prefix are filed under
# Added and Fixed; everything else lands under Changed, which is what plain
# imperative subjects get. Release bookkeeping commits are dropped.

set -euo pipefail

version="$1"
range="$2"
date="$3"

added=""
fixed=""
changed=""

# Collected to a file first so a failing `git log` is fatal. Reading it from a
# process substitution hides the exit status, and the loop then sees no input -
# indistinguishable from a release with no commits, which would quietly publish
# a changelog claiming nothing changed.
subjects=$(mktemp)
trap 'rm -f "$subjects"' EXIT

if ! git log --no-merges --format='%s' "$range" >"$subjects"; then
  echo "changelog-section.sh: git log failed for range '$range'" >&2
  exit 1
fi

while IFS= read -r subject; do
  [ -n "$subject" ] || continue
  case "$subject" in
    "scoop: v"*|"Release "[0-9]*) continue ;;
    feat:*|"feat("*) added+="- ${subject#*: }"$'\n' ;;
    fix:*|"fix("*) fixed+="- ${subject#*: }"$'\n' ;;
    *) changed+="- $subject"$'\n' ;;
  esac
done <"$subjects"

printf '## [%s] - %s\n' "$version" "$date"

if [ -z "$added$fixed$changed" ]; then
  printf '\n### Changed\n\n- Maintenance release with no user-visible changes.\n'
  exit 0
fi

[ -n "$added" ] && printf '\n### Added\n\n%s' "$added"
[ -n "$fixed" ] && printf '\n### Fixed\n\n%s' "$fixed"
[ -n "$changed" ] && printf '\n### Changed\n\n%s' "$changed"

exit 0
