#!/bin/bash
# Prints the jobs of this run that failed, timed out or were cancelled, one name per line without the prefix of the
# reusable workflow ("build-main / "). Latest attempt of each job, so "Re-run failed jobs" counts.
# Needs GH_TOKEN (actions: read), GITHUB_REPOSITORY and GITHUB_RUN_ID.
set -euo pipefail

gh api "repos/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID/jobs?filter=latest" --paginate \
  --jq '.jobs[] | select(.conclusion == "failure" or .conclusion == "timed_out" or .conclusion == "cancelled") | .name' |
  sed 's|^[^/]* / ||'
