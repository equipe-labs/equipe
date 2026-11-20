#!/usr/bin/env python3
"""equipe-ops' careful parts: what to back up, what a backup holds, and
whether a machine holds the same squads afterwards.

Standard library only (the image's Python 3.12). Never prints a secret: it
reads squad databases (read-only), file metadata and process tables, nothing
from sign-in stores.

  opslib.py roots                      the absolute paths a backup covers
  opslib.py manifest                   JSON: what this machine holds now
  opslib.py verify [MANIFEST]          integrity, and equal to MANIFEST if given
  opslib.py snapshot-dbs DIR           consistent copies of every live .db
  opslib.py holders                    processes holding files under the roots
  opslib.py status                     JSON for `equipe-ops status`
  opslib.py excludes {restic|tar}      the exclude list, expanded
"""
import datetime as _dt
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
from pathlib import Path

HOME = Path(os.environ.get("HOME", "/home/tenki"))
EQUIPE_LINK = Path(os.environ.get("EQUIPE_HOME", HOME / ".equipe"))
ETC = Path(os.environ.get("EQUIPE_OPS_ETC", "/etc/equipe-ops"))
STATE = Path(os.environ.get("EQUIPE_OPS_STATE", HOME / ".local/state/equipe-ops"))
USER_PATHS = HOME / ".config/equipe-ops/paths"
MANIFEST_VERSION = 1


def equipe_home() -> Path:
    return EQUIPE_LINK.resolve()


def equipe_bin() -> Path:
    held = STATE / "held-equipe"
    return held if held.is_file() else HOME / ".local/bin/equipe"


def equipe_version() -> str:
    try:
        out = subprocess.run([str(equipe_bin()), "--version"], capture_output=True, text=True, timeout=20).stdout
        return out.split()[1] if out.startswith("equipe ") else ""
    except (OSError, IndexError, subprocess.TimeoutExpired):
        return ""


def version_tuple(v: str):
    return tuple(int(x) for x in re.findall(r"\d+", v)[:3]) or (0,)


def squad_dbs():
    squads = equipe_home() / "squads"
    if not squads.is_dir():
        return []
    return sorted(p for p in squads.glob("*/squad.db") if p.is_file())


def ro(db: Path) -> sqlite3.Connection:
    conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True, timeout=30)
    conn.execute("PRAGMA query_only = 1")
    return conn


def table_exists(conn, name):
    return conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?", (name,)).fetchone() is not None


def projects_of(db: Path):
    try:
        with ro(db) as conn:
            if not table_exists(conn, "projects"):
                return []
            return [r[0] for r in conn.execute("SELECT path FROM projects ORDER BY rowid")]
    except sqlite3.Error:
        return []


def read_lines(path: Path):
    try:
        return [l.strip() for l in path.read_text().splitlines() if l.strip() and not l.lstrip().startswith("#")]
    except OSError:
        return []


def expand(line: str) -> str:
    return line.replace("$EQUIPE_HOME", str(equipe_home())).replace("$HOME", str(HOME))


def roots():
    """Every absolute path a backup must hold, deduplicated, existing only.
    `~/.equipe` itself is listed too when it is a link, so a restore puts the
    link back where the squads' paths expect it."""
    home = equipe_home()
    found = [home]
    if EQUIPE_LINK.is_symlink():
        found.append(EQUIPE_LINK)
    for db in squad_dbs():
        found += [Path(p) for p in projects_of(db)]
    for extra in (HOME / ".claude", HOME / ".codex", HOME / ".local/share/keyrings", HOME / ".config/equipe-ops/paths"):
        found.append(extra)
    for f in sorted(ETC.glob("paths.d/*.paths")) + [USER_PATHS]:
        found += [Path(expand(l)) for l in read_lines(f)]
    out = []
    for p in found:
        if not p.is_absolute() or not (p.exists() or p.is_symlink()):
            continue
        # A path inside one already listed adds nothing (the link is kept: it is not inside).
        if any(p != q and not p.is_symlink() and str(p).startswith(str(q) + "/") for q in found if q.exists() and not q.is_symlink()):
            continue
        if p not in out:
            out.append(p)
    return out


def excludes(kind: str):
    lines = []
    for f in [ETC / "backup.exclude"] + sorted(ETC.glob("paths.d/*.exclude")):
        lines += [expand(l) for l in read_lines(f)]
    if kind == "tar":
        # GNU tar matches member names, which have no leading slash.
        lines = [l.lstrip("/") for l in lines]
    return lines


def squad_facts(db: Path):
    facts = {"name": db.parent.name, "db": str(db), "projects": projects_of(db)}
    try:
        with ro(db) as conn:
            facts["quick_check"] = conn.execute("PRAGMA quick_check").fetchone()[0]
            facts["tickets"] = conn.execute("SELECT COUNT(*) FROM tickets").fetchone()[0] if table_exists(conn, "tickets") else 0
            if table_exists(conn, "tickets"):
                facts["tickets_by_status"] = dict(conn.execute("SELECT status, COUNT(*) FROM tickets GROUP BY status").fetchall())
                cols = [r[1] for r in conn.execute("PRAGMA table_info(tickets)")]
                if "worktree" in cols:
                    facts["open_worktrees"] = [r[0] for r in conn.execute(
                        "SELECT worktree FROM tickets WHERE worktree IS NOT NULL AND worktree != '' "
                        "AND status NOT IN ('done', 'cancelled')")]
            facts["entries"] = conn.execute("SELECT COUNT(*) FROM entries").fetchone()[0] if table_exists(conn, "entries") else 0
    except sqlite3.Error as e:
        facts["error"] = str(e)
    return facts


def manifest():
    return {
        "manifest": MANIFEST_VERSION,
        "created_at": _dt.datetime.now(_dt.timezone.utc).isoformat(timespec="seconds"),
        "hostname": os.uname().nodename,
        "equipe_version": equipe_version(),
        "equipe_home": str(equipe_home()),
        "equipe_link": str(EQUIPE_LINK) if EQUIPE_LINK.is_symlink() else None,
        "roots": [str(p) for p in roots()],
        "squads": [squad_facts(db) for db in squad_dbs()],
    }


def verify(expected_path=None):
    """Exit 0 when every squad database is sound, every project is a git
    repository, open tickets' worktrees exist, and (given a manifest) the
    machine holds the same squads with the same tickets as it says."""
    problems, notes = [], []
    now = manifest()
    for sq in now["squads"]:
        if sq.get("error") or sq.get("quick_check") != "ok":
            problems.append(f"squad {sq['name']}: database check failed ({sq.get('error') or sq.get('quick_check')})")
        for p in sq["projects"]:
            if not Path(p).is_dir():
                problems.append(f"squad {sq['name']}: project folder missing: {p}")
            elif subprocess.run(["git", "-C", p, "rev-parse", "--git-dir"], capture_output=True).returncode != 0:
                problems.append(f"squad {sq['name']}: project is not a git repository: {p}")
        for w in sq.get("open_worktrees", []):
            if not Path(w).exists():
                problems.append(f"squad {sq['name']}: an open ticket's worktree is missing: {w}")
    if expected_path:
        exp = json.loads(Path(expected_path).read_text())
        have = {s["name"]: s for s in now["squads"]}
        for s in exp.get("squads", []):
            got = have.get(s["name"])
            if not got:
                problems.append(f"squad {s['name']} is in the manifest but not on this machine")
                continue
            for key in ("tickets", "entries"):
                if s.get(key) != got.get(key):
                    problems.append(f"squad {s['name']}: {key} {got.get(key)} here, {s.get(key)} in the manifest")
            if s.get("tickets_by_status") and s["tickets_by_status"] != got.get("tickets_by_status"):
                problems.append(f"squad {s['name']}: ticket statuses differ from the manifest")
        extra = sorted(set(have) - {s["name"] for s in exp.get("squads", [])})
        if extra:
            notes.append(f"squads here that the manifest doesn't have: {', '.join(extra)}")
        if exp.get("equipe_version") and not now["equipe_version"]:
            problems.append("can't tell which equipe is installed here (no ~/.local/bin/equipe answering --version)")
        elif exp.get("equipe_version") and version_tuple(now["equipe_version"]) < version_tuple(exp["equipe_version"]):
            problems.append(f"equipe {now['equipe_version']} here is older than {exp['equipe_version']}, which last wrote these "
                            f"squads: update equipe before starting the daemon")
        for r in exp.get("roots", []):
            if not (Path(r).exists() or Path(r).is_symlink()):
                problems.append(f"backed-up path missing here: {r}")
    report = {"ok": not problems, "problems": problems, "notes": notes,
              "squads": {s["name"]: s.get("tickets") for s in now["squads"]}}
    print(json.dumps(report, indent=2))
    return 0 if not problems else 1


def snapshot_dbs(dest: str):
    """A consistent copy of every SQLite database under the equipe home, made
    with SQLite's online backup while the daemon keeps writing. Laid out as
    DEST/<path relative to the equipe home>."""
    home = equipe_home()
    dest_root = Path(dest)
    if dest_root.exists():
        shutil.rmtree(dest_root)
    count = 0
    for db in sorted(home.rglob("*.db")):
        rel = db.relative_to(home)
        if rel.parts and rel.parts[0] in ("cache", "tmp"):
            continue
        if not db.is_file() or db.is_symlink():
            continue
        out = dest_root / rel
        out.parent.mkdir(parents=True, exist_ok=True)
        src = sqlite3.connect(f"file:{db}?mode=ro", uri=True, timeout=60)
        dst = sqlite3.connect(out)
        with dst:
            src.backup(dst, pages=4096)
        src.close()
        dst.close()
        count += 1
    print(count)


SHELLS = {"bash", "sh", "dash", "zsh", "fish", "sshd", "tmux", "screen", "sudo", "su", "login"}


def holders():
    """Processes of this user holding a file open under a root, or working in
    one (an interactive shell sitting in a project folder writes nothing and
    is left alone)."""
    uid = os.getuid()
    rts = [str(p) for p in roots() if not p.is_symlink()]
    me = os.getpid()
    out = []
    for pid in filter(str.isdigit, os.listdir("/proc")):
        if int(pid) in (me, os.getppid()):
            continue
        try:
            if os.stat(f"/proc/{pid}").st_uid != uid:
                continue
            comm = Path(f"/proc/{pid}/comm").read_text().strip()
            paths = [] if comm in SHELLS else [os.readlink(f"/proc/{pid}/cwd")]
            for fd in os.listdir(f"/proc/{pid}/fd"):
                try:
                    paths.append(os.readlink(f"/proc/{pid}/fd/{fd}"))
                except OSError:
                    pass
        except OSError:
            continue
        if any(p == r or p.startswith(r + "/") for p in paths for r in rts):
            cmd = Path(f"/proc/{pid}/cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace").strip()
            out.append({"pid": int(pid), "cmd": cmd[:160]})
    print(json.dumps(out))


def du(path: Path) -> int:
    r = subprocess.run(["du", "-sxb", str(path)], capture_output=True, text=True)
    try:
        return int(r.stdout.split()[0])
    except (IndexError, ValueError):
        return 0


def status():
    def read_json(p):
        try:
            return json.loads(Path(p).read_text())
        except (OSError, ValueError):
            return None

    def active(unit):
        return subprocess.run(["systemctl", "--user", "is-active", unit], capture_output=True, text=True).stdout.strip()

    disk = shutil.disk_usage(str(equipe_home()) if equipe_home().exists() else str(HOME))
    last = read_json(STATE / "last-backup.json")
    age = None
    if last and last.get("at"):
        age = int((_dt.datetime.now(_dt.timezone.utc) - _dt.datetime.fromisoformat(last["at"])).total_seconds())
    return {
        "hostname": os.uname().nodename,
        "instance": (HOME / ".config/equipe-ops/instance").read_text().strip() if (HOME / ".config/equipe-ops/instance").exists() else None,
        "image": read_json(ETC / "image.json"),
        "equipe_version": equipe_version(),
        "maintenance": read_json(STATE / "maintenance.json"),
        "services": {u: active(u) for u in ("equipe.service", "equipe-desktop.service", "equipe-backup.timer")},
        "disk": {"path": str(equipe_home()), "total": disk.total, "used": disk.used, "free": disk.free,
                 "used_pct": round(100 * disk.used / disk.total, 1) if disk.total else None},
        "backup": {"configured": (HOME / ".config/equipe-ops/backup.env").is_file(), "last": last, "age_seconds": age},
        "squads": [{"name": s["name"], "tickets": s.get("tickets")} for s in (squad_facts(d) for d in squad_dbs())],
    }


def status_text(s):
    def gb(n):
        return f"{n / 2**30:.1f} GB"

    lines = []
    inst = s.get("instance")
    lines.append(f"machine     {s['hostname']}" + (f"  (instance {inst})" if inst else ""))
    img = s.get("image") or {}
    lines.append(f"equipe      {s['equipe_version'] or '?'}" + (f"  (image {img.get('equipe')}, built {img.get('built_at')})" if img else ""))
    m = s.get("maintenance")
    lines.append("state       " + (f"MAINTENANCE since {m['since']} ({m.get('by')})" if m else "in service"))
    lines.append("services    " + ", ".join(f"{k.split('.')[0]} {v or '?'}" for k, v in s["services"].items()))
    d = s["disk"]
    warn = "  <- running low: plan a move to a bigger disk" if d["used_pct"] and d["used_pct"] >= 85 else ""
    lines.append(f"disk        {gb(d['used'])} of {gb(d['total'])} used ({d['used_pct']}%), {gb(d['free'])} free{warn}")
    b = s["backup"]
    if not b["configured"]:
        lines.append("backup      NOT CONFIGURED (no ~/.config/equipe-ops/backup.env)")
    elif not b["last"]:
        lines.append("backup      configured, none made yet")
    else:
        last, age = b["last"], b["age_seconds"] or 0
        stale = "  <- older than 2 hours" if age > 7200 else ""
        what = last.get("snapshot") or last.get("message")
        lines.append(f"backup      last {'ok' if last['ok'] else 'FAILED'} {age // 60} min ago ({last['kind']}, {what}){stale}")
    for q in s["squads"]:
        lines.append(f"squad       {q['name']}: {q['tickets']} tickets")
    return "\n".join(lines)


def main(argv):
    if not argv:
        print(__doc__)
        return 2
    cmd, rest = argv[0], argv[1:]
    if cmd == "roots":
        print("\n".join(str(p) for p in roots()))
    elif cmd == "manifest":
        print(json.dumps(manifest(), indent=2))
    elif cmd == "verify":
        return verify(rest[0] if rest else None)
    elif cmd == "snapshot-dbs":
        snapshot_dbs(rest[0])
    elif cmd == "holders":
        holders()
    elif cmd == "status":
        print(json.dumps(status(), indent=2))
    elif cmd == "status-text":
        print(status_text(status()))
    elif cmd == "excludes":
        print("\n".join(excludes(rest[0] if rest else "restic")))
    elif cmd == "version":
        print(equipe_version())
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
