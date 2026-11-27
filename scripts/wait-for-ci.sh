#!/bin/bash
# Wait for CI (.github/workflows/ci.yml) on one commit and succeed only when
# it passed. The Release workflow's `ci` job runs this before it publishes, so
# a tag on a commit CI failed (or never ran on) builds but never ships.
#
# Every CI job counts except "warm release cache", which only fills the
# release builds' caches (the release builds the same things itself).
#
#   GH_TOKEN=… GITHUB_REPOSITORY=owner/repo scripts/wait-for-ci.sh <sha>
#
# CI runs on a push to main and by hand (`gh workflow run ci.yml --ref vX.Y.Z`).
# Any such run of the commit counts; pull request runs test a merge commit, so
# they don't. It waits for a run to appear (WAIT_FOR_RUN seconds, the push
# that starts it may land a little after the tag) and then for it to finish.
set -euo pipefail
repo="${GITHUB_REPOSITORY:?}"
sha="${1:?usage: wait-for-ci.sh <sha>}"
wait_for_run="${WAIT_FOR_RUN:-600}"
poll="${POLL:-30}"
started=$(date +%s)

while :; do
  runs=$(gh api "/repos/$repo/actions/workflows/ci.yml/runs?head_sha=$sha&per_page=20" \
    -q '.workflow_runs[] | select(.event != "pull_request") | "\(.id) \(.status) \(.html_url)"')
  if [ -z "$runs" ]; then
    if [ $(($(date +%s) - started)) -ge "$wait_for_run" ]; then
      echo "::error::No CI run for $sha. Push the commit to main, or run CI on the tag: gh workflow run ci.yml --ref <tag>"
      exit 1
    fi
    echo "No CI run for $sha yet; waiting"
    sleep "$poll"
    continue
  fi

  pending=0
  failed=""
  while read -r id status url; do
    # The latest attempt's jobs, so a re-run of failed jobs counts.
    jobs=$(gh api --paginate "/repos/$repo/actions/runs/$id/jobs?filter=latest&per_page=100" \
      -q '.jobs[] | select(.name | startswith("warm release cache") | not) | "\(.status)\t\(.conclusion)\t\(.name)"')
    if [ "$status" != "completed" ] || grep -qv '^completed' <<<"$jobs"; then
      pending=1
      continue
    fi
    bad=$(grep -vE $'^completed\t(success|skipped)\t' <<<"$jobs" || true)
    if [ -n "$jobs" ] && [ -z "$bad" ]; then
      echo "CI passed on $sha: $url"
      exit 0
    fi
    failed+="$url"$'\n'"$(cut -f2- <<<"$bad" | sed 's/^/  /')"$'\n'
  done <<<"$runs"

  if [ "$pending" = 0 ]; then
    echo "::error::CI did not pass on $sha, so this release is not published. Fix main and tag again, or re-run CI's failed jobs and then this run's."
    printf '%s' "$failed"
    exit 1
  fi
  echo "CI on $sha is still running"
  sleep "$poll"
done
