#!/bin/bash
# Attaches files to a release (usually a draft) by its id; an asset of the same name, left by an earlier attempt of the
# job, is replaced. By id, not by tag: a draft has no tag yet, and two runs of one commit draft the same tag name.
# Usage: attach-to-release.sh <release-id> <file>...   Needs GH_TOKEN and GITHUB_REPOSITORY (gh and curl, ubuntu runner).
set -euo pipefail

id="$1"
shift
for file in "$@"; do
  name="$(basename "$file")"
  old=$(gh api "repos/$GITHUB_REPOSITORY/releases/$id/assets" --paginate --jq ".[] | select(.name == \"$name\") | .id")
  if [ -n "$old" ]; then
    gh api -X DELETE "repos/$GITHUB_REPOSITORY/releases/assets/$old"
  fi
  curl --fail-with-body --silent --show-error --output /dev/null -X POST \
    -H "Authorization: Bearer $GH_TOKEN" -H "Content-Type: application/octet-stream" --data-binary "@$file" \
    "https://uploads.github.com/repos/$GITHUB_REPOSITORY/releases/$id/assets?name=$name"
  echo "attached $name"
done
