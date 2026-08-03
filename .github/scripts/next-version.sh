#!/usr/bin/env bash
#
# Print the version that follows <current> for a major/minor/patch bump.
#
# Usage: next-version.sh <current-version> <major|minor|patch>

set -euo pipefail

current="$1"
level="$2"

IFS=. read -r major minor patch <<<"$current"

case "$level" in
  major) major=$((major + 1)); minor=0; patch=0 ;;
  minor) minor=$((minor + 1)); patch=0 ;;
  patch) patch=$((patch + 1)) ;;
  *) echo "unknown bump level: $level" >&2; exit 1 ;;
esac

echo "$major.$minor.$patch"
