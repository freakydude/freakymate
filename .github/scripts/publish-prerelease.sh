#!/bin/bash
# Publishes the draft prerelease of this run once all builds have ended, with what is in it: the packages of the builds
# that succeeded, and the list of those that failed. Moonraker's update manager and Obtainium see only the finished
# release. Without a release: a cancelled run (a newer commit replaces it), failed tests, or no package at all; then
# the draft goes and the previous prerelease stays. After publishing, older prereleases are deleted.
# Usage: publish-prerelease.sh <release-id> <tag> <result of the build job>
# Needs GH_TOKEN (contents: write, actions: read), GITHUB_REPOSITORY, GITHUB_RUN_ID, GITHUB_SHA, GITHUB_SERVER_URL.
set -euo pipefail

id="$1"
tag="$2"
build_result="$3"
scripts="$(dirname "${BASH_SOURCE[0]}")"
release="repos/$GITHUB_REPOSITORY/releases/$id"
run_url="$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID"

drop_draft() {
  echo "$1; no prerelease, the draft is deleted"
  gh api -X DELETE "$release" > /dev/null || true
}

if [ "$build_result" = cancelled ]; then
  drop_draft "Run cancelled"
  exit 0
fi

failed=$(bash "$scripts/failed-jobs.sh")
# Job names of the test jobs in build.yml.
if grep -q -E '^(Format, Clippy, Tests \(Linux\)|Windows tests)$' <<< "$failed"; then
  drop_draft "Tests failed"
  echo "::error title=No prerelease::Tests failed"
  exit 1
fi
if [ "$(gh api "$release" --jq '.assets | length')" -eq 0 ]; then
  drop_draft "No package was built"
  echo "::error title=No prerelease::No package was built"
  exit 1
fi

bash "$scripts/attach-to-release.sh" "$id" scripts/get-freakymate.sh

notes="Automatic build of main at $GITHUB_SHA, replaced by the next commit on main. Newest functions first, less time in use than a release; use it with care and at your own risk (see the README)."
if [ -z "$failed" ]; then
  notes+=$'\n\nAll builds succeeded.'
else
  notes+=$'\n\n'"**Incomplete:** these builds failed, their packages are missing ([run]($run_url)):"
  while read -r job; do notes+=$'\n'"- $job"; done <<< "$failed"
  echo "::warning title=Incomplete prerelease::$(tr '\n' ';' <<< "$failed")"
fi
gh api -X PATCH "$release" -F draft=false -f body="$notes" > /dev/null
echo "Published $tag"

for old in $(gh release list --repo "$GITHUB_REPOSITORY" --limit 100 --json tagName,isPrerelease,isDraft \
  --jq '.[] | select(.isPrerelease and (.isDraft | not)) | .tagName'); do
  if [ "$old" != "$tag" ]; then gh release delete "$old" --repo "$GITHUB_REPOSITORY" --yes --cleanup-tag; fi
done
