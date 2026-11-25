#!/usr/bin/env python3
"""Put synthetic squad data on a test sandbox, through equipe's own screen
protocol (`equipe attach --stdio`, the commands the console sends): a squad on
a small git repository, and N tickets nobody works on. No engine runs, no model
is called, nothing real is touched.

  seed.py --squad e2e --repo ~/repos/e2e --tickets 3
"""
import argparse
import json
import queue
import subprocess
import sys
import threading
import time
from pathlib import Path


class Link:
    def __init__(self, equipe):
        self.proc = subprocess.Popen([equipe, "attach", "--stdio"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     text=True, bufsize=1)
        self.q = queue.Queue()
        self.next = 100
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.proc.stdout:
            try:
                self.q.put(json.loads(line))
            except ValueError:
                pass
        self.q.put(None)

    def send(self, msg):
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", **msg}) + "\n")
        self.proc.stdin.flush()

    def request(self, method, params, wait=60):
        self.next += 1
        rid = self.next
        self.send({"id": rid, "method": method, "params": params})
        end = time.time() + wait
        while time.time() < end:
            msg = self.q.get(timeout=max(0.1, end - time.time()))
            if msg is None:
                sys.exit("equipe attach closed")
            if msg.get("id") == rid:
                if "error" in msg:
                    sys.exit(f"{method}: {msg['error']}")
                return msg.get("result")
        sys.exit(f"{method}: no answer")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--squad", required=True)
    ap.add_argument("--repo", required=True)
    ap.add_argument("--tickets", type=int, default=3)
    ap.add_argument("--equipe", default=str(Path.home() / ".local/bin/equipe"))
    a = ap.parse_args()
    repo = Path(a.repo).expanduser()
    if not (repo / ".git").exists():
        repo.mkdir(parents=True, exist_ok=True)
        run = lambda *c: subprocess.run(c, cwd=repo, check=True, capture_output=True)
        run("git", "init", "-q", "-b", "main")
        (repo / "README.md").write_text("# e2e\n")
        run("git", "add", "README.md")
        run("git", "-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "commit", "-qm", "init")
    if not (Path.home() / ".equipe/squads" / a.squad).is_dir():
        subprocess.run([a.equipe, "squad", "new", a.squad, str(repo)], check=True, capture_output=True)
    link = Link(a.equipe)
    link.q.get(timeout=60)  # the bridge's `attached` line
    link.request("hello", {"version": "e2e-seed", "squad": a.squad, "lang": "en"})
    for i in range(a.tickets):
        link.send({"method": "command", "params": {"type": "create_ticket", "squad": a.squad, "draft": {
            "title": f"e2e ticket {i + 1} ({time.strftime('%H:%M:%S')})", "body": "Synthetic, for the backup test. Nobody works on it.",
            "kind": "task"}}})
    time.sleep(3)
    link.proc.stdin.close()
    link.proc.wait(timeout=30)
    print(f"seeded {a.tickets} tickets in {a.squad}")


if __name__ == "__main__":
    main()
