#!/usr/bin/env python3
"""Render deploy/tenki/image/ into a Tenki template spec (tenki.template.v1).

Tenki reads a template's files from a Git context, which would mean pushing
before every build. Instead the image's files travel inside the spec: one `run`
step unpacks a base64 tarball of deploy/tenki/image/ into /opt/equipe-ops/src
and runs provision.sh. The spec is then self-contained, reviewable, and
builds the same from any checkout or CI job.

  python3 deploy/tenki/template/render.py > deploy/tenki/template/template.json
  tenki template build equipe-host --file deploy/tenki/template/template.json

The tarball is made reproducibly (sorted, fixed owner and mtime), so the same
files give the same spec, and Tenki's digest only moves when they change.
"""
import base64
import gzip
import io
import json
import subprocess
import sys
import tarfile
from pathlib import Path

IMAGE = Path(__file__).resolve().parent.parent / "image"
SRC = "/opt/equipe-ops/src"


def revision() -> str:
    try:
        rev = subprocess.run(["git", "-C", str(IMAGE), "log", "-1", "--format=%h", "--", "."],
                             capture_output=True, text=True, check=True).stdout.strip()
        dirty = subprocess.run(["git", "-C", str(IMAGE), "status", "--porcelain", "--", "."],
                               capture_output=True, text=True).stdout.strip()
        return (rev or "uncommitted") + ("+dirty" if dirty else "")
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def tarball(rev: str) -> bytes:
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.PAX_FORMAT) as tar:
        def add(name: str, data: bytes, mode: int):
            info = tarfile.TarInfo(name)
            info.size, info.mode, info.mtime, info.uid, info.gid = len(data), mode, 0, 0, 0
            info.uname = info.gname = "root"
            tar.addfile(info, io.BytesIO(data))

        for path in sorted(p for p in IMAGE.rglob("*") if p.is_file()):
            rel = path.relative_to(IMAGE).as_posix()
            add(rel, path.read_bytes(), 0o755 if path.stat().st_mode & 0o111 else 0o644)
        add("REVISION", (rev + "\n").encode(), 0o644)
    out = io.BytesIO()
    with gzip.GzipFile(fileobj=out, mode="wb", mtime=0, compresslevel=9) as gz:
        gz.write(raw.getvalue())
    return out.getvalue()


def main() -> int:
    rev = revision()
    blob = base64.b64encode(tarball(rev)).decode()
    unpack = (
        f"set -eu; sudo -n rm -rf {SRC}; sudo -n install -d -m 0755 {SRC}; "
        f"echo '{blob}' | base64 -d | sudo -n tar -xz -C {SRC} --no-same-owner; "
        f"sudo -n sh {SRC}/provision.sh"
    )
    # One default `run` step and no Git context: the only shape Tenki builds
    # without a repository checkout (the files travel in the step itself).
    # Its default timeout (30 min) covers provision.sh with room to spare.
    spec = {
        "specVersion": "tenki.template.v1",
        "base": {"image": "sandbox"},
        "workdir": "/home/tenki",
        "steps": [{"run": {"command": unpack}}],
        # The smallest disk a host can have; each sandbox picks its own, up to 100 GB.
        "resources": {"cpuCores": 4, "memoryMb": 8192, "diskSizeGb": 12},
    }
    json.dump(spec, sys.stdout, indent=2)
    sys.stdout.write("\n")
    print(f"render.py: {len(blob)} bytes of image files, revision {rev}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
