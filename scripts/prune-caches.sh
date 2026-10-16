#!/bin/bash
# Keep the Actions cache under GitHub's 10 GB per repository: of each cache
# family (the key without its last two hashes — rust-cache's environment and
# lockfile hashes), keep only the one used last and delete the rest. Past the
# limit GitHub evicts on its own, and a build that loses its cache starts from
# nothing: the desktop app's took twice as long.
#
#   GH_TOKEN=… GITHUB_REPOSITORY=owner/repo scripts/prune-caches.sh
set -euo pipefail
repo="${GITHUB_REPOSITORY:?}"
gh api --paginate "/repos/$repo/actions/caches?per_page=100&sort=last_accessed_at&direction=desc" \
  -q '.actions_caches[] | "\(.id) \(.key)"' |
  awk '{
    n = split($2, part, "-")
    family = part[1]
    for (i = 2; i <= n - 2; i++) family = family "-" part[i]
    if (family in seen) print $1, $2; else seen[family] = 1
  }' |
  while read -r id key; do
    echo "delete $key"
    gh api -X DELETE "/repos/$repo/actions/caches/$id" >/dev/null || true
  done
gh api "/repos/$repo/actions/cache/usage" -q '"cache now \(.active_caches_size_in_bytes / 1048576 | floor) MB in \(.active_caches_count) entries"'
