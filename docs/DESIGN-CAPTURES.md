# Design captures

equipe's design tools make many pictures and videos: the UI map's shots and walkthroughs,
design QA's and the lint's renders, mockups, directions, reference shots, critiques' and
sessions' renders. They are re-rendered again and again. While they were committed, one
project's `.git` grew to 13.7 GB — 13.6 GB of it PNG, 11.4 GB from `design/flows` alone, 139,204
versions of pictures over 1,604 commits — and GitHub refused its push.

Since then **captures never go into git**. Git holds what they are (a small manifest); equipe's
store holds their bytes; the main checkout holds them on disk at the same paths as before, so the
desktop, the phone apps and the members' tools find them where they always did. A ticket's
worktree holds only the captures its own work made (and the work it builds on): the rest,
unchanged from main, are read from the main checkout — a worktree of a project with a big map
costs nothing for it.

Code: `crates/core/src/design/captures.rs` (all of it), hooked into
`crates/core/src/worktree.rs` (commit, checkout, merge, land) and the daemon's hourly sweep
(`runtime.rs`, `tidy_lane_tmp`).

## What a capture is

A file under a design folder's capture folders with a picture or video extension:

| | |
|---|---|
| folders (in `design/`) | `flows` `qa` `checks` `mockups` `directions` `references` `critiques` `sessions` `audit` `palettes` `shots` `renders` |
| extensions (any case) | `png` `jpg` `jpeg` `webp` `gif` `mp4` `webm` `mov` |

Only a design folder equipe keeps counts — one with `tokens.json`, `guidelines.md` or
`flows/flows.json`. The design system's own assets (`assets/`, `icons/`, `components/`, a logo)
are made once and stay in git. JSON, Markdown, HTML and every other metadata file in those
folders (flow definitions, verdicts, check records, a picture's note) stay in git too.

## The layout — the contract a sync uses

Everything a sync (equipe Hub) needs is the manifest and the bytes it names. Nothing in either
is an absolute path or names a machine.

### The manifest: `design/captures.json`, in git

```json
{"version": 1, "captures": [
{"path":"flows/login/form/error.dark.390.png","hash":"9f2c…","size":48211,"created_at":1763350000,"ticket":142,"flow":"login","screen":"form","state":"error","mode":"dark","width":390},
{"path":"qa/t1661/charts/after-1280.png","hash":"51d0…","size":203117,"created_at":1763351200,"ticket":1661}
]}
```

- One capture a line, sorted by `path`, so a diff and a merge see one line per picture.
- `path`: relative to the design folder.
- `hash`: git's blob id of the bytes (`git hash-object`: SHA-1 of `blob <size>\0<bytes>`;
  SHA-256 in a SHA-256 repository). The same picture has the same hash everywhere.
- `size` in bytes; `created_at` in seconds since 1970 (when it was rendered — a checkout sets
  the file's time to it, which the UI map's staleness check reads); `ticket`, the ticket whose
  work made it, when one did.
- `flow`, `screen`, `state`, `mode` (`light`/`dark`), `width`: a UI map picture's, from its
  name (`flows/<flow>/<screen>/<state>.<mode>.<width>.png`); absent for other captures.
- Unknown fields are ignored; entries whose path leaves the design folder or isn't a capture
  are dropped.

It is versioned with the code: each branch has its own, a merge merges it, and the manifest at a
commit says which pictures that commit's design had. Two branches' manifests are merged entry by
entry — a capture one side changed is taken, one both changed is the one rendered last — so it
never stops a merge (`captures::merge3`, used when equipe merges and when a ticket's worktree
starts from other tickets' work).

### The bytes: equipe's store, outside git

```text
<repository>/.git/equipe/captures/
  objects/<first 2 hex of hash>/<rest of hash>   the bytes, one file per hash
  reports/<branch>.json                          what landing told a member (transient)
  tmp/, hold/                                    work in progress (cleared after a day)
```

- In the repository's common git folder: shared by the main checkout and every worktree, never
  committed or pushed, gone with the clone.
- Content-addressed: identical renders are kept once. A copy is a clone on APFS, so a picture
  in the store and in two checkouts takes the room of one.
- An object no manifest names — not at any local branch's tip, not on main in the last 30
  days, not in a worktree's manifest on disk — goes once a week old (`captures::gc`, hourly).
- A sync uploads the objects the manifests name and downloads the ones a manifest names that the
  store lacks; equipe then puts them in place on the next checkout or merge.

### On disk: where they always were

The main checkout's design folder has every capture at its path; a ticket's worktree has the ones
its manifest names with other bytes than main's. Both are git-ignored by a block equipe keeps in
`design/.gitignore` (`# >>> equipe: design captures stay out of git` … `# <<<`). equipe rewrites
that block; anything outside it is the project's.

Where a worktree's design folder is read, a capture it lacks that its manifest names with the same
hash as the main checkout's manifest is read from the main checkout (`captures::borrowed`,
`captures::lent`): the UI map of a draft shows main's unchanged pictures beside its own, so
Compare marks only what the ticket changed, and design QA's precheck finds the map's pictures. A
picture at a past commit — what used to be `git show <rev>:<path>` — is read through that commit's
manifest from the store (`captures::file_at`).

## When what happens

| when | what equipe does |
|---|---|
| a ticket's worktree is made (or made again for a reopened ticket) | only the captures its manifest names with other bytes than the main checkout's — its own work, or the work it starts from — are put in place from the store; main's unchanged ones are not copied (read from the main checkout). What it was given is kept in the worktree's git admin folder |
| equipe commits a worktree's work | `design/.gitignore` gets the block; any tracked capture is untracked (the file stays); `design/captures.json` is rewritten from what is on disk, each capture kept in the store. A capture its member removed — one the worktree was given — leaves the manifest; one it was never given stays, unless it is a UI map picture of a state the map no longer lists. A re-render is a manifest change, so capture-only work still makes a commit and lands. |
| equipe merges a branch into main (the PM's accept with checks, the person's merge, a task into the squad's folder) | **the guard**: main is migrated first if it isn't; the branch's own commits (those main lacks) that add captures — `git add -f`, an older equipe — are rewritten without them, keeping authors, dates and messages (a signature can't stay); their bytes go to the store and the manifest. None reaches main's history. A conflict only in the manifest or in captures is settled, not reported. A commit that adds more than 10 MB of other pictures or videos is noted. Then the merge lands, and the captures its manifest changed are put in the main checkout; those it removed are taken away if nobody changed them since. |
| after that merge | the member who made the work is told what was kept out and why (in English, in its conversation), the PM is briefed, and the person sees one line (`news.captures_kept_out`, `news.captures_heavy`) |
| equipe upgrades (the daemon's hourly sweep, and before any merge) | **the migration**: one normal commit on main, made apart from any checkout — untracks every capture main holds, adds the ignore block and the manifest (each capture kept in the store first). The checkout main is checked out in fast-forwards and keeps every file (each is held by a second link while git removes it, then put back). History is not rewritten. Nothing happens to a project without an equipe design folder. |

A checkout with uncommitted changes to a capture refuses the fast-forward; nothing changes and
the next sweep tries again.

Captures from another machine (origin's main merged in) whose bytes this machine doesn't have
are logged; the files stay missing until a sync brings them or the map is shot again.

## Design registries: merged entry by entry

A few JSON files under a design folder are registries every design ticket adds to:
`flows/flows.json` (the UI map, kept on one line), `references/board.json`,
`directions/log.json` and `captures.json`. Merged as text, two tickets that each add an entry
conflict on almost every landing — the one-line map always. equipe merges them as JSON instead
(`crates/core/src/design/registry.rs`):

- An array whose entries each carry an `id` (else a `path`, else a `name`; every entry, each
  once) is merged entry by entry: both sides' additions kept (ours in its order, theirs' after
  the entry before them), an entry one side changed taken from it, one side's removal kept unless
  the other changed that entry. Objects merge key by key, keeping their order. Other arrays keep
  both sides' additions at their end. Only a value both sides changed differently is a conflict,
  named by its place (`flows[id=erp].screens[id=list].title`).
- The result keeps the file's style: one line stays one line, an indented file keeps its indent,
  and a side whose text is the result is kept byte for byte. `captures.json` merges as its
  manifest does (above).
- **equipe's landing** (`prepare_merge`, `merge_conflicts` — the PM's `accept_work`, the checks
  gate, the person's merge, origin's main merged in before a push) merges a registry `git
  merge-tree` reports conflicting as JSON, and tells of a conflict only when one is left.
- **git itself**: equipe adds a merge driver to the repository's config and names the registries
  in its `info/attributes` — both local to the machine, shared by every worktree, nothing
  committed:

  ```
  [merge "equipe-registry"]
      driver = '/path/to/equipe' design merge-registry %O %A %B %P || git merge-file -q --marker-size=%L -L ours -L base -L theirs %A %O %B
  # info/attributes, in equipe's block:
  **/design/flows/flows.json merge=equipe-registry
  ```

  So a member's `git merge main` in its worktree merges them the same way. Where a real conflict
  is left, or equipe isn't there, git's own text merge runs and marks it as usual.

## Files equipe writes on main

Most design work is a ticket's, committed in its worktree. A few files equipe writes where a
project's design folder is — its main checkout — when the person acts outside any ticket: the
style profile and taste notes (`design/taste.md`), a design decision and its picture
(`design/decisions/`), a reference kept from a conversation (`design/references/<pattern>/notes.md`),
a critique or a style profile written by `equipe design critique` / `equipe design session profile`
run there. Left uncommitted, they made the next landing's fast-forward of that checkout refuse
("couldn't move without touching its changes"), and accepted work stayed off main.

- Each is committed on main as soon as it is written: one commit of equipe's, made apart from the
  checkout, which then fast-forwards (the files are staged there first as they are on disk, so the
  fast-forward finds nothing of its own to keep). A file the person staged otherwise isn't taken.
  In a ticket's worktree nothing is done — its member commits its work — and a squad's folder is
  saved its own way.
- Before every landing, such files still uncommitted in the checkout main is checked out in —
  changed or new, never staged, deleted or conflicted ones — are committed the same way first
  (`equipe: commit the design notes equipe wrote`). Nothing else there is touched: the person's own
  changes still refuse the fast-forward, and nothing merges.
- The taste tuner's signals are the person's own on this machine: a project's are kept outside the
  working tree, in `<git common dir>/equipe/taste/design/signals.jsonl`, never committed. One an
  older equipe left at `design/tuner/signals.jsonl` is moved there before the next landing.
- A merge that fails for another reason than a conflict leaves the ticket accepted but not on main
  (`merged: false`, `off_main: failed`, "accepted, not on main" on the board). The PM is told at
  once that it was not merged, and equipe tries the merge again every five minutes, six times
  (after a restart too, for a failure in the last three days); the PM hears when it lands or when
  equipe stops trying.

## A squad's folder: what its tasks hand over

The same mechanism keeps a squad's own folder (`~/.equipe/squads/<squad>/home`) small. What its
tasks hand over under `outputs/` as pictures, videos, sound, archives and PDFs (by extension, any
case: see `OUTPUT_EXTS` in `crates/core/src/design/captures.rs`) is kept out of the folder's git
the way design captures are: `outputs/.gitignore` carries equipe's block, `outputs/.captures.json`
lists each file (path inside `outputs/`, hash, size, time, ticket), and the bytes go into the same
store under the folder's `.git/equipe/captures`. A ticket's sparse worktree gives back only what it
holds; files left out of it stay as main has them. The guard takes them out of a task's commits
before it lands, the migration untracks what an older folder tracked (one commit, every file kept),
and the landing puts them in the folder. A project's own `outputs/` is never touched.

A squad's folder also never runs git's own automatic gc or maintenance (`gc.auto=0`,
`maintenance.auto=false` in its config, and on every git command equipe runs there): a commit
started one whatever room was left, and a repack that ran out of room left gigabytes of
unfinished packs. equipe repacks the folder itself in its hourly sweep — loose objects past 6,700
or packs past 50 — only while the disk is fine and twice what the repack writes is free
(`docs/DISK.md`).

## Shrinking history that already grew

The migration stops the growth; it doesn't give back what history holds. That takes rewriting
history and a force-push — every clone must be re-cloned and every open branch re-made — so it is
never automatic. `scripts/purge-design-captures.sh` does it, only when a person runs it:

```sh
# 1. What it would take out, and how big the pack would be after (changes nothing):
scripts/purge-design-captures.sh --repo ~/Projects/site --captures design --dry-run

# 2. Only after the project's owner said yes:
scripts/purge-design-captures.sh --repo ~/Projects/site --captures design --apply \
    --backup ~/purge-backups/site.git
```

- What goes is named explicitly: `--captures <design folder>` (the captures under it, as above,
  in every commit), `--media-under <folder>` (every picture and video under a folder),
  `--path <path>` (a file or folder, whatever it holds), `--paths-file <file>` (lines in
  `git filter-repo --paths-from-file` form: a path, `glob:<pattern>` or `regex:<pattern>`).
- `--dry-run` (the default) prints the pack's size now, what would go (the blobs only those
  paths ever held, by size on disk), the estimated pack size after, and every local branch,
  worktree, stash and remote that the rewrite affects.
- `--apply` first makes a mirror of the repository (`git clone --mirror`, to `--backup`), refuses
  while linked worktrees exist (it lists them: remove them, re-make them afterwards) or the
  checkout has uncommitted changes, runs `git filter-repo`, puts `origin` back (filter-repo removes
  it), expires the reflogs and repacks, and prints the size after and what to do next. It never
  pushes: the force-push (`git push --force --all origin && git push --force --tags origin`) and
  telling everyone with a clone are the person's.
- Needs [git-filter-repo](https://github.com/newren/git-filter-repo) for `--apply` only.

The estimate is the sum of the on-disk (compressed, possibly deltified) sizes of the blobs that
appear only under the purged paths in all of history (`git rev-list --objects --all` joined
with `git cat-file --batch-all-objects --batch-check='%(objectname) %(objecttype)
%(objectsize:disk)'`). A repack after the rewrite may compress the rest a little differently, so
the real size lands near it, not on it.
