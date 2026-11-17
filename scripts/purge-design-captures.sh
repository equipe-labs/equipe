#!/usr/bin/env bash
# Take design captures (or other named paths) out of every commit of a
# repository, to give back the room its history holds. It rewrites history:
# every clone must be re-cloned and every open branch re-made afterwards, so
# run it only when the project's owner said yes. Without --apply it changes
# nothing and prints what it would take out and the estimated pack size after.
#
# See docs/DESIGN-CAPTURES.md ("Shrinking history that already grew").
#
#   scripts/purge-design-captures.sh --repo DIR [what…] [--dry-run | --apply --backup DIR [--yes]]
#
# What goes, named explicitly (one or more, repeatable):
#   --captures DESIGN_DIR   the captures under a design folder (design): pictures and
#                           videos under its flows/ qa/ checks/ mockups/ directions/
#                           references/ critiques/ sessions/ audit/ palettes/ shots/ renders/
#   --media-under DIR       every picture and video under DIR
#   --path PATH             a file or a folder, whatever it holds
#   --paths-file FILE       lines as `git filter-repo --paths-from-file` reads them:
#                           a path, glob:<pattern> or regex:<pattern>
# Then:
#   --dry-run               the default: sizes, what goes, branches and worktrees affected
#   --apply                 rewrite (needs git-filter-repo); never pushes
#   --backup DIR            with --apply: where the mirror made first goes (must not exist)
#   --backup-copy           make that mirror a full copy, not hard links to the packs
#   --yes                   with --apply: don't ask to confirm
set -euo pipefail

DIRS="flows qa checks mockups directions references critiques sessions audit palettes shots renders"
EXTS="png jpg jpeg webp gif mp4 webm mov"

die() { echo "purge-design-captures: $*" >&2; exit 1; }

repo="" mode="dry" backup="" backup_copy=0 yes=0
specs="$(mktemp "${TMPDIR:-/tmp}/purge-specs.XXXXXX")"
trap 'rm -f "$specs"' EXIT

media_globs() { # <folder>
  local dir="${1%/}" ext upper
  for ext in $EXTS; do
    upper="$(printf '%s' "$ext" | tr '[:lower:]' '[:upper:]')"
    printf 'glob:%s/*.%s\nglob:%s/*.%s\n' "$dir" "$ext" "$dir" "$upper" >>"$specs"
  done
}

while [ $# -gt 0 ]; do
  case "$1" in
    --repo) repo="${2:?--repo needs a folder}"; shift 2 ;;
    --captures)
      design="${2:?--captures needs a design folder}"; shift 2
      for d in $DIRS; do media_globs "${design%/}/$d"; done ;;
    --media-under) media_globs "${2:?--media-under needs a folder}"; shift 2 ;;
    --path) printf 'literal:%s\n' "${2:?--path needs a path}" >>"$specs"; shift 2 ;;
    --paths-file) [ -f "${2:-}" ] || die "no such file: ${2:-}"; grep -v '^[[:space:]]*$' "$2" >>"$specs" || true; shift 2 ;;
    --dry-run) mode="dry"; shift ;;
    --apply) mode="apply"; shift ;;
    --backup) backup="${2:?--backup needs a folder}"; shift 2 ;;
    --backup-copy) backup_copy=1; shift ;;
    --yes) yes=1; shift ;;
    -h|--help) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
done

[ -n "$repo" ] || die "name the repository: --repo DIR"
[ -s "$specs" ] || die "name what to take out: --captures, --media-under, --path or --paths-file"
repo="$(cd "$repo" && git rev-parse --show-toplevel)" || die "$repo isn't a git repository"
command -v python3 >/dev/null || die "python3 is needed to estimate (git-filter-repo needs it too)"
g() { git -C "$repo" "$@"; }

echo "Repository: $repo"
echo "Taking out of every commit ($(wc -l <"$specs" | tr -d ' ') patterns, as git filter-repo reads them):"
sed -n '1,8p' "$specs" | sed 's/^/  /'
[ "$(wc -l <"$specs")" -le 8 ] || echo "  …"
echo

# ── sizes ──
echo "Measuring (reads every object once; a big repository takes a while)…"
objects="$(mktemp "${TMPDIR:-/tmp}/purge-objects.XXXXXX")" sizes="$(mktemp "${TMPDIR:-/tmp}/purge-sizes.XXXXXX")"
trap 'rm -f "$specs" "$objects" "$sizes"' EXIT
g rev-list --objects --all >"$objects"
g cat-file --batch-all-objects --batch-check='%(objectname) %(objecttype) %(objectsize:disk)' >"$sizes"
python3 -I - "$specs" "$objects" "$sizes" "$(g count-objects -v | awk '/^size-pack:/{p=$2} /^size:/{l=$2} END{print (p+l)*1024}')" <<'PY'
import fnmatch, re, sys
specs_file, objects_file, sizes_file, before = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
literal, globs, regexes = [], [], []
for line in open(specs_file, encoding="utf-8"):
    line = line.rstrip("\n")
    if line.startswith("glob:"):
        globs.append(line[5:])
    elif line.startswith("regex:"):
        regexes.append(re.compile(line[6:]))
    elif line:
        literal.append(line[8:] if line.startswith("literal:") else line)
def goes(path):
    return any(path == p or path.startswith(p.rstrip("/") + "/") for p in literal) or any(fnmatch.fnmatchcase(path, g) for g in globs) or any(r.search(path) for r in regexes)
paths = {}
for line in open(objects_file, encoding="utf-8", errors="replace"):
    oid, _, path = line.rstrip("\n").partition(" ")
    if path:
        paths[oid] = path
gone = count = 0
by_top = {}
for line in open(sizes_file):
    oid, kind, disk = line.split()
    if kind != "blob" or oid not in paths or not goes(paths[oid]):
        continue
    count += 1
    gone += int(disk)
    top = "/".join(paths[oid].split("/")[:2])
    by_top[top] = by_top.get(top, 0) + int(disk)
def words(n):
    for unit in ["B", "KB", "MB", "GB", "TB"]:
        if n < 1000 or unit == "TB":
            return f"{n:.1f} {unit}" if unit != "B" else f"{n} B"
        n /= 1000
print(f"Objects now:            {words(before)} (packed and loose)")
print(f"Blobs that would go:    {count}, {words(gone)} on disk")
for top, size in sorted(by_top.items(), key=lambda kv: -kv[1])[:15]:
    print(f"  {top:<40} {words(size)}")
print(f"Estimated after:        {words(max(before - gone, 0))}")
print("(Each blob counted by the first path history reaches it at; a repack after may land a little off.)")
PY
echo

# ── what the rewrite affects ──
echo "Local branches (each is rewritten; re-make any branch others have from the new history):"
g for-each-ref --format='  %(refname:short)  %(objectname:short)  %(upstream:short)' refs/heads
echo "Worktrees (linked ones must be removed before --apply and made again after):"
g worktree list | sed 's/^/  /'
stashes="$(g stash list || true)"
if [ -n "$stashes" ]; then
  echo "Stashes (they keep old commits; apply or drop them first):"
  printf '%s\n' "$stashes" | sed 's/^/  /'
fi
echo "Remotes (git-filter-repo removes origin; this script puts it back, never pushes):"
g remote -v | sed 's/^/  /'
echo

if [ "$mode" = "dry" ]; then
  echo "Dry run: nothing changed. Run again with --apply --backup DIR once the owner said yes."
  exit 0
fi

# ── apply ──
command -v git-filter-repo >/dev/null || git filter-repo --version >/dev/null 2>&1 || die "git-filter-repo isn't installed (https://github.com/newren/git-filter-repo)"
[ -n "$backup" ] || die "--apply needs --backup DIR: a mirror is made before anything is rewritten"
[ ! -e "$backup" ] || die "$backup is there already"
[ -z "$(g status --porcelain --untracked-files=no)" ] || die "the checkout has uncommitted changes: commit or stash them first"
linked="$(g worktree list --porcelain | grep -c '^worktree ' || true)"
if [ "${linked:-0}" -gt 1 ]; then
  echo "Linked worktrees — remove each (git -C '$repo' worktree remove <path>), then run again:" >&2
  g worktree list --porcelain | awk '/^worktree /{w=$2} /^branch /{print "  " w "  (" $2 ")"}' | tail -n +2 >&2
  exit 1
fi
if [ "$yes" -ne 1 ]; then
  printf 'This rewrites every commit of %s. Type the folder name (%s) to go on: ' "$repo" "$(basename "$repo")"
  read -r answer
  [ "$answer" = "$(basename "$repo")" ] || die "not confirmed; nothing changed"
fi

echo "Mirroring to $backup…"
if [ "$backup_copy" -eq 1 ]; then git clone -q --mirror --no-hardlinks "$repo" "$backup"; else git clone -q --mirror "$repo" "$backup"; fi
origin="$(g remote get-url origin 2>/dev/null || true)"

echo "Rewriting…"
g filter-repo --force --invert-paths --paths-from-file "$specs"
if [ -n "$origin" ] && ! g remote get-url origin >/dev/null 2>&1; then
  g remote add origin "$origin"
fi
echo "Repacking…"
g reflog expire --expire=now --all
g gc -q --prune=now
echo
echo "Objects after: $(g count-objects -vH | awk '/^size-pack:/{print $2, $3}') packed"
echo "Backup mirror: $backup"
echo
echo "Next, by hand:"
echo "  1. check the result (git -C '$repo' log --stat -5, the app builds)"
echo "  2. push it: git -C '$repo' push --force --all origin && git -C '$repo' push --force --tags origin"
echo "  3. every other clone: re-clone (a pull would bring the old history back)"
echo "  4. make the worktrees listed above again, and re-create branches others had from the new history"
