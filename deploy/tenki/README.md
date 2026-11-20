# equipe on Tenki: hosting, backups, and moving to a bigger machine

This directory runs an equipe host on a [Tenki](https://tenki.cloud) sandbox for
a customer who uses equipe from the cloud. It is built around one rule:

> **A host never grows in place. It moves.** When it needs a bigger disk, a new
> sandbox is made from the same image, the data is copied and checked, and only
> then does the name move over. The old sandbox waits, untouched, until someone
> ends it.

Resizing a disk in place means detaching it from a running machine, and when
that step stalls the host is down with no way back. That happened on
2026-11-14 with a Tenki persistent volume (stuck in `SYNC_PENDING` for hours;
see "Why not a volume"). A move can fail at any step and the old host is still
there, in service again within seconds.

## What is here

| Path | What it is |
|---|---|
| `image/provision.sh` | Turns Tenki's stock Ubuntu image into an equipe host. Idempotent. |
| `image/versions.env` | Every outside download, pinned with its checksum or key fingerprint. |
| `image/rootfs/` | Files the image installs: `equipe-ops`, its Python helper, systemd units, the backup exclude list. |
| `template/render.py` | Renders `image/` into a Tenki template spec (`template/template.json`). |
| `ops/migrate.sh` | Run from an operator's machine: plan, run, roll back and finalize a move. |
| `ops/backup.env.example` | The backup target, with `secrets://` placeholders Tenki fills in. |
| `tests/e2e.sh` | Builds nothing, spends cents: checks an image, a backup, a restore and a move on throwaway sandboxes. |

## The host

A sandbox made from the image has:

- **equipe**, the pinned release, at `~/.local/bin/equipe`, run by the same systemd user unit `equipe daemon install`
  writes. `equipe update` works as on any machine.
- **Data on the sandbox's own disk**, in `~/.equipe` and the squads' project folders. Size is chosen when the sandbox
  is made: 5–100 GB.
- **A desktop**: Xfce on TigerVNC display `:1`, on `127.0.0.1:5901` only, with a password made on the machine. equipe's
  Screen panel shows it through the app's existing SSH connection (`equipe screen --stdio`); nothing listens on a
  public interface. Google Chrome from Google's signed repository.
- **Backups every hour** to a bucket the customer owns (restic: encrypted, incremental, deduplicated), when a backup
  target is configured.
- **`equipe-ops`**, the one command an operator needs on the host:

```
equipe-ops status                 state, disk, last backup, squads
equipe-ops maintenance on|off     stop equipe so nothing can start it / put it back
equipe-ops backup [--quiesce]     back up now (live by default; --quiesce stops equipe for it)
equipe-ops snapshots              list backups
equipe-ops restore <id|latest>    restore (in maintenance), then verify
equipe-ops verify                 databases sound, projects are repositories, open worktrees exist
```

`status` warns at 85% disk and when the last backup is over two hours old.

## One-time setup

### 1. Build the image

```sh
python3 deploy/tenki/template/render.py > deploy/tenki/template/template.json
tenki template build equipe-host --file deploy/tenki/template/template.json
```

The build prints an immutable digest ref (`<workspace>/equipe-host@sha256:…`); `<workspace>/equipe-host` always means
the newest successful build. Rebuild whenever `image/` changes; commit the rendered `template.json` with it so the
image is reviewable. A build takes about 10 minutes on 4 vCPU.

### 2. A backup target (strongly recommended)

Any restic repository works; an S3-compatible bucket the customer owns is the usual choice (Cloudflare R2, S3,
Backblaze B2). Give it a key limited to that bucket. Then save three workspace secrets, each with a hidden prompt:

```sh
tenki secrets create EQUIPE_BACKUP_RESTIC_PASSWORD
tenki secrets create EQUIPE_BACKUP_S3_ACCESS_KEY_ID
tenki secrets create EQUIPE_BACKUP_S3_SECRET_ACCESS_KEY
```

Put the bucket's address in your copy of `ops/backup.env.example`. **Keep the restic password in a password manager
too**: without it nobody can read the backups.

### 3. Make the host

```sh
tenki sandbox create --image <workspace>/equipe-host --name acme-equipe \
  --cpu 4 --memory-mb 8192 --disk-size-gb 40 --sticky \
  --secret-file deploy/tenki/ops/backup.env.example:/home/tenki/.config/equipe-ops/backup.env
tenki sandbox ssh --session acme-equipe        # then: claude, and /login
equipe-ops backup --init                       # once: creates the repository, makes the first backup
```

The customer's equipe app attaches to it over SSH as `sbx-acme-equipe` (Tenki's SSH config). Sign-ins (`claude`,
`codex`, `gh`) are made on the host and **never** copied by a backup or a move.

## Moving to a bigger disk (or a new image)

From any machine with the `tenki` CLI signed in to the workspace:

```sh
deploy/tenki/ops/migrate.sh plan --from acme-equipe --image <workspace>/equipe-host --disk 80
deploy/tenki/ops/migrate.sh run  --from acme-equipe --image <workspace>/equipe-host --disk 80
```

`run` does, in order, and journals each step in `~/.local/state/equipe-tenki/<run-id>.json`:

1. Makes the new sandbox from the image (same CPU and memory unless told otherwise), sticky, named `<name>-new-<time>`.
2. Stops equipe on the old sandbox (`maintenance on`: the binary is held aside so the app's reconnects can't
   restart it). **Downtime starts.**
3. Copies the data:
   - `--via stream` (default): a zstd tar straight from the old sandbox, through the operator's machine, into the new
     one. Nothing is stored on the way; both ends compare a SHA-256 of the stream.
   - `--via backup`: a quiesced backup to the bucket, restored on the new sandbox.
4. Checks the new sandbox against the old one's manifest: same squads, same ticket and conversation counts, ticket
   statuses, project repositories, open worktrees; and an equipe at least as new as the one that last wrote the data.
5. Starts equipe on the new sandbox. **Downtime ends.**
6. Renames: the old sandbox becomes `<name>-retired-<time>`, the new one takes `<name>`, so `sbx-<name>` reaches it.
   Each machine whose app attaches runs `tenki sandbox ssh --session <name> -- -T true` once to refresh its alias.

If anything fails before step 6, the old sandbox is put back in service and the new one is left for inspection.
After step 6:

```sh
deploy/tenki/ops/migrate.sh rollback --run <run-id>                      # back to the old sandbox
deploy/tenki/ops/migrate.sh finalize --run <run-id> --terminate-retired  # end the old sandbox for good
```

Downtime is the copy time: roughly 1–3 minutes per GB over a stream. Rebuildable data (equipe's build caches, lane
temp folders, `node_modules`) is not copied, and the members rebuild it.

The source can be a sandbox made before this image existed: `migrate.sh` copies `equipe-ops` onto it first.

## Restoring from a backup

On a host (in maintenance) or on a fresh sandbox from the image:

```sh
equipe-ops maintenance on
equipe-ops snapshots
equipe-ops restore latest          # or a snapshot id; what was there is moved aside, not deleted
equipe-ops maintenance off
```

If Tenki itself is unavailable, the bucket is still the customer's: any Linux machine with restic, the repository
address and the password can read it (`restic restore latest --target /`).

Rehearse a restore on a throwaway sandbox at least once a quarter. `tests/e2e.sh` is that rehearsal.

## Limits and choices

- **Disk: 100 GB per sandbox** (Tenki's maximum for a root disk, chosen at create time). Persistent volumes stop at
  50 GB. A plan's storage quota covers sandboxes, volumes and snapshots together; a move briefly needs both sandboxes'
  disks. For more, ask Tenki about Enterprise limits.
- **Why not a volume.** A volume outlives its sandbox, which is what backups now do, and better: off Tenki,
  versioned, encrypted. A volume can only be resized detached, and a detach that stalls (`SYNC_PENDING`) can't be
  undone from the customer's side. Data on the sandbox's own disk plus off-site backups has neither problem.
- **Live backups** copy each SQLite database with SQLite's online backup API and leave the live files out, so a
  snapshot is consistent without stopping equipe. Git repositories and worktrees are copied as files: a commit being
  written at that instant can be missed, never corrupted — the next hourly backup has it.
- **Retention**: 24 hourly, 14 daily, 8 weekly, 12 monthly; pruned daily; 1% of the data is re-read and checked weekly.

## Not done yet

- **Shorter downtime.** A move copies everything while equipe is stopped. The next step is a two-phase copy: most of
  the data while equipe runs, then only what changed.
- **Alerts.** `equipe-ops status` reports a full disk or a stale backup; nothing pages anyone yet.
- **equipe's secret store on Linux** needs a Secret Service (`secret-tool`); the image has none, so keys members save
  ("Keys & secrets") don't persist on a Tenki host yet.
- **Wayland and GNOME** desktops: the image is X11/Xfce only.
