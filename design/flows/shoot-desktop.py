#!/usr/bin/env python3
"""Shoot the desktop app's UI map (design/flows/flows.json) from the real app.

The desktop app has no URL, so `equipe design flows shoot` can't reach it.
Each state here is the app built with `--features snapshot`, attached to the
fake daemon (crates/desktop/examples/fake_daemon.rs) in a home of its own, and
driven by a snapshot script (crates/desktop/src/snapshot.rs): the steps that
reach the state, then a shot at each width, light and dark.

    CARGO_TARGET_DIR=…/t379 cargo build -p equipe-desktop --features snapshot
    CARGO_TARGET_DIR=…/t379 cargo build -p equipe-desktop --features snapshot --examples
    python3 design/flows/shoot-desktop.py --bin …/t379/debug [--only conversation/] [--jobs 3]

Widths are the window's in points at 1x: 1200 (the size it opens at) and 760
(the narrowest it allows). The app speaks English here (EQUIPE_LANG=en) unless a
state names another language; `long` states switch to German, the longest language.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
SIZES = [(1200, 760), (760, 600)]
MODES = ["light", "dark"]

# (flow/screen/state, fake daemon env, snapshot steps, options)
# options: ask_telemetry (leave the usage question unanswered), no_daemon
# (nothing to attach to), kill_daemon_ms (stop the daemon that long after
# the app starts), lang, sizes, wait (ms after the steps, before shots).
STATES = []


def state(path, steps="", env=None, **opts):
    STATES.append((path, env or {}, steps, opts))


# ── First run ──────────────────────────────────────────────────────────────
state("first-run/usage-data/default", ask_telemetry=True)
state("first-run/usage-data/long", ask_telemetry=True, lang="de")
state("first-run/git/default", "setup:nogit", no_daemon=True)
state("first-run/git/loading", "setup:gitasked", no_daemon=True)
state("first-run/ai/default", "setup:missing", no_daemon=True)
state("first-run/ai/confirm", "setup:confirm", no_daemon=True)
state("first-run/ai/loading", "setup:installing", no_daemon=True)
state("first-run/ai/error", "setup:failed", no_daemon=True)
state("first-run/ai/signedout", "setup:signedout", no_daemon=True)
state("first-run/ai/waiting", "setup:waiting", no_daemon=True)
state("first-run/ai/success", "setup:ready", no_daemon=True)
state("first-run/ai/long", "setup:missing", no_daemon=True, lang="de")
state("first-run/squad/default", "setup:squad", no_daemon=True)
state("first-run/squad/loading", "setup:creating;wait:300;scroll:20,-80", no_daemon=True)
state("first-run/squad/error", "setup:error;wait:300;scroll:20,-80", no_daemon=True)

# ── Inbox: decisions, permissions, cards ───────────────────────────
state("inbox/inbox/default", "menu:inbox")
state("inbox/inbox/empty", "menu:inbox", {"FAKE_EMPTY": "1", "FAKE_ASKS": "0"})
state("inbox/inbox/long", "menu:inbox", lang="de")
state("inbox/decision/default", "menu:inbox;wait:600;dpick:9001", {"FAKE_DECIDE": "1"})
state("inbox/decision/pictures", "menu:inbox;wait:600;dpick:9003", {"FAKE_DECIDE": "logo"}, wait=2500)
state("inbox/decision/success", "menu:inbox;wait:600;dpick:9001;dkey:1;dkey:enter;wait:300;dkey:1;dkey:enter;wait:1500;chat;wait:800;decided", {"FAKE_DECIDE": "1", "FAKE_ASKS": "0"}, wait=1500, label="Answered: the next card comes up in the conversation")
state("inbox/permission/default", "menu:inbox", {"FAKE_ASKS": "3", "FAKE_EMPTY": "1"})
state("inbox/your-turn/default", "menu:inbox;wait:600;ppick:117", {"FAKE_TURN": "1"})
state("inbox/your-turn/long", "menu:inbox;wait:600;ppick:117", {"FAKE_TURN": "files"})
state("inbox/team-change/default", "menu:inbox;wait:600;tpick:7001", {"FAKE_TEAM": "1"})
state("inbox/routine/default", "menu:inbox;wait:600;rpick:8001", {"FAKE_ROUTINE": "1"})
state("inbox/accept/default", "menu:inbox;wait:600;cpick", {"FAKE_ACCEPT": "1"})

# ── Conversation ───────────────────────────────────────────────────────────
state("conversation/chat/default", "chat")
state("conversation/chat/empty", "newtalk;wait:800")
state("conversation/chat/loading", "chat;say:Where are you up to?;wait:1500", {"FAKE_SECONDS": "12"})
state("conversation/chat/long", "chat;wait:800", {"FAKE_READING": "1"})
state("conversation/chat/pictures", "chat;wait:800", {"FAKE_REPLY_IMAGES": "1"}, wait=2500)
state("conversation/chat/ticket-cards", "chat;wait:800", {"FAKE_CARDS": "1"})
state("conversation/chat/ticket-card-missing", "chat;wait:800;hover:806,432;wait:1500", {"FAKE_CARDS": "1", "FAKE_ASKS": "0"}, label="A ticket card for a number not on the board: Not found")
state("conversation/chat/ticket-card-recovered", "chat;wait:800;hover:421,432;wait:1500", {"FAKE_CARDS": "1", "FAKE_ASKS": "0", "FAKE_STALE": "42"}, label="A ticket card for a ticket the list missed: looked up, shown")
state("conversation/chat/ask-list", "chat;wait:800", {"FAKE_ASKLIST": "1"})
state("conversation/chat/diagram", "chat;say:torture 5;wait:5000", {"FAKE_SECONDS": "1"})
state("conversation/chat/markdown", "chat;say:torture 1;wait:6000", {"FAKE_SECONDS": "1"})
state("conversation/chat/summarizing", "chat;wait:800", {"FAKE_COMPACT": "long"})
state("conversation/chat/summarize-failed", "chat;wait:3000", {"FAKE_COMPACT": "fail", "FAKE_COMPACT_AFTER": "500"})
state("conversation/engine-switch/default", "chat", {"FAKE_USAGE": "claude86", "FAKE_ASKS": "0"})
state("conversation/engine-switch/error", "chat", {"FAKE_USAGE": "claude_out", "FAKE_ASKS": "0"})
state("conversation/engine-switch/success", "chat", {"FAKE_USAGE": "claude86", "FAKE_SWITCHING": "auto", "FAKE_ASKS": "0"})
state("conversation/chat/your-turn", "chat;wait:600", {"FAKE_TURN": "1", "FAKE_ASKS": "0"})
state("conversation/composer/attachments", "chat;attach:{ROOT}/design/brand/logo-in-use.png,{ROOT}/design/guidelines.md;wait:600")
state("conversation/composer/drop", "chat;drag:{ROOT}/design/guidelines.md;wait:400")
state("conversation/composer/queued", "chat;say:First one;wait:200;say:Carry on with the second too;wait:600", {"FAKE_SECONDS": "12"})

# ── Board & tickets ────────────────────────────────────────────────────────
state("board/board/default", "menu:all", wait=1200)
state("board/board/empty", "menu:all", {"FAKE_EMPTY": "1", "FAKE_WORKING": "0", "FAKE_ASKS": "0"}, wait=1200, label="Nothing waiting for the person")
state("board/board/long", "menu:all", {"FAKE_BUSY": "1"}, lang="de", wait=1200)
state("board/board/busy", "menu:all", {"FAKE_BUSY": "1", "FAKE_SUBAGENTS": "1"}, wait=1200)
state("board/working/default", "menu:working", {"FAKE_BUSY": "1"}, wait=1200)
state("board/ticket/default", "menu:all;wait:600;ppick:47", wait=1200)
state("board/ticket/review", "menu:all;wait:600;ppick:42", {"FAKE_CARDS": "long", "FAKE_REVIEW": "1"}, wait=1500)
state("board/ticket/pictures", "menu:all;wait:600;ppick:4", {"FAKE_REPLY_IMAGES": "1"}, wait=2500)
state("board/ticket/page", "menu:all;wait:600;pick:42;wait:300;bkey:enter", wait=1500)

# ── Settings ───────────────────────────────────────────────────────────────
state("settings/general/default", "settings:general")
state("settings/general/long", "settings:general", lang="de")
state("settings/engines/default", "settings:engines", {"FAKE_USAGE": "healthy"})
state("settings/engines/error", "settings:engines", {"FAKE_USAGE": "both_low"})
# Claude Code's card: signed in (above), the sign-out question, signed out, another account's sign-in
# under way, refused, done; German (the longest) stands in for large text — the app has no text-size setting.
state("settings/engines/confirm", "settings:engines;eng:signout:claude", {"FAKE_USAGE": "healthy"}, label="Claude Code: sign-out asked first, naming who is working")
state("settings/engines/signedout", "settings:engines;eng:signout:claude;eng:sure:claude;wait:1500", {"FAKE_USAGE": "healthy"}, label="Claude Code signed out")
state("settings/engines/waiting", "settings:engines;eng:switch:claude;wait:900", {"FAKE_USAGE": "healthy"}, label="Claude Code: another account's sign-in under way")
state("settings/engines/failed", "settings:engines;eng:switch:claude;wait:900;signin:paste:bad-code;wait:1200", {"FAKE_USAGE": "healthy", "FAKE_ENGINES": "failed"}, label="Claude Code: another account's sign-in refused", kind="error")
state("settings/engines/success", "settings:engines;eng:switch:claude;wait:900;signin:paste:abc#def;wait:1200", {"FAKE_USAGE": "healthy"}, label="Claude Code signed in as another account: said a moment, its email shown", kind="success")
state("settings/engines/cancelled", "settings:engines;eng:switch:claude;wait:900;eng:cancel:claude;wait:800", {"FAKE_USAGE": "healthy"}, label="Claude Code: another account's sign-in cancelled, the card as before")
state("settings/engines/long", "settings:engines;eng:signout:claude", {"FAKE_USAGE": "healthy"}, lang="de")
state("settings/squad/default", "settings:squad")
state("settings/team/default", "settings:team")
state("settings/team/empty", "settings:team", {"FAKE_SIGNALS": "0"})
state("settings/permissions/default", "settings:permissions", lang="ko")
state("settings/projects/default", "settings:projects")
state("settings/browser/default", "settings:browser")
state("settings/accounts/default", "settings:accounts;lseed")
state("settings/accounts/empty", "settings:accounts")
state("settings/accounts/form", "settings:accounts;lform;wait:300")
state("settings/keys/default", "settings:keys")
state("settings/about/default", "settings:about")
state("overview/metrics/default", "metrics:dash", wait=1200)
state("overview/metrics/empty", "metrics:empty", wait=1200)
state("overview/metrics/error", "metrics:error", wait=1200)
state("overview/metrics/connect", "metrics:connect", wait=800)
state("overview/design/default", "design:overview", {"FAKE_DESIGN": "1"}, wait=2500)
state("overview/design/empty", "design:overview", {"FAKE_DESIGN": "empty"}, wait=1200)
state("overview/design/draft", "design:overview;wait:1500;ddraft", {"FAKE_DESIGN": "draft"}, wait=2000)

# ── Disk ───────────────────────────────────────────────────────────────────
state("disk/panel/default", "disk", {"FAKE_DISK": "low"}, wait=1500)
state("disk/panel/error", "disk", {"FAKE_DISK": "critical"}, wait=1500)
state("disk/panel/full", "disk", {"FAKE_DISK": "full"}, wait=1500)
state("disk/fix/default", "disk;wait:1500;disk:fix1", {"FAKE_DISK": "low"})
state("disk/fix/success", "disk;wait:1500;disk:fix1;wait:300;disk:yes;wait:2500", {"FAKE_DISK": "low"})
# ── Overview, roadmap, routines ─────────────────────────────────────
state("overview/overview/default", "overview", {"FAKE_BUSY": "1"}, wait=1500)
state("overview/overview/empty", "overview", {"FAKE_EMPTY": "1", "FAKE_WORKING": "0", "FAKE_ASKS": "0"}, wait=1500)
state("overview/roadmap/default", "roadmap", wait=1500)
state("overview/routines/default", "overview;rseed", wait=1200)
state("overview/routines/form", "overview;rseed;rnew:weekly", wait=1200)

# ── Panels and sheets over the work ────────────────────────────────────────
state("tools/browser/default", "chat", {"FAKE_BROWSER": "1"}, wait=3000)
state("tools/browser/empty", "chat;browser", {"FAKE_BROWSER": "empty"}, wait=1500)
state("tools/browser/handover", "chat", {"FAKE_BROWSER": "handover"}, wait=3000, lang="ko")
state("tools/terminal/default", "chat;term;wait:3000;tkeys:ls -la~", wait=1500)
state("tools/document/default", "chat;doc:{ROOT}/design/guidelines.md", wait=1500)
state("tools/subagent/default", "chat;subagent:frontend", {"FAKE_SUBAGENTS": "1"}, wait=2000)
state("tools/pair-phone/default", "pair", wait=1200)
state("tools/pair-phone/permission", "pair", {"FAKE_PAIR": "ask"}, wait=2500)
state("tools/pair-phone/error", "pair", {"FAKE_PAIR": "relay"}, wait=2500)
state("tools/notifications/default", "bell:sheet", wait=600)

# ── Designer studio ───────────────────────────────────────────────────────
state("studio/designer/waiting", "studio:205", {"FAKE_STUDIO": "blocked"}, wait=2500, lang="ko", label="Waiting for the person")


# ── Daemon disconnected ────────────────────────────────────────────────────
state("connection/daemon/offline", "wait:2500", kill_daemon_ms=4000)
state("connection/daemon/offline-first", "", no_daemon=True, home_squad=True, cli="fails", kind="offline")
state("connection/install/default", "", no_daemon=True, cli="none")
state("connection/chooser/default", "chooser;wait:500")


# In the conversation the oldest permission request sits over the composer;
# only the default state keeps it, so the others show what they are about.
for _path, _env, _steps, _opts in STATES:
    if _path.startswith("conversation/") and _path != "conversation/chat/default":
        _env.setdefault("FAKE_ASKS", "0")

# ── The map: flows, screens, what each state is ────────────────────────────
V = "crates/desktop/src/"
FLOWS = [
    ("first-run", "First run", "A Mac with no squad: git, connecting the AI (ChatGPT sign-in, or Claude Code install and sign-in), the first squad, then the usage-data question. The steps up to the squad are held by the snapshot tool's `setup:` without a daemon, as a Mac with no squad has none; the usage question needs the daemon, so it comes once the squad is made."),
    ("inbox", "Inbox · decisions · permissions", "What waits for the person: a member's questions, permission requests, a ticket put down for them, a team change, a routine, work to confirm."),
    ("conversation", "Conversation", "The PM's conversation: replies streaming, the engine-switching card, long text, pictures, ticket cards, a list of questions, diagrams and HTML cards, the summary band, the composer with attachments."),
    ("board", "Board · tickets", "Every ticket by where it stands (Tickets), what is at work (In progress), and one ticket picked — peeking beside the list (Space) or as a page (⏎): its hand-in, review and pictures."),
    ("overview", "Squad views: Overview · Roadmap · Routines · Metrics · Design system", "The sidebar's views of the whole squad: its projects side by side, the roadmap, the regular work the PM is handed, the numbers (metrics) and the design system."),
    ("settings", "Settings", "General (with privacy: anonymous usage data), AI platforms, squad and devices, team and signals, permissions, projects, browser, accounts, keys, about."),
    ("tools", "Panels and sheets", "The browser panel, terminal, a document beside the conversation, a subagent, pairing a phone, the notifications sheet."),
    ("studio", "Designer studio", "A live view of design work: what it is waiting on, its steps, current picture, findings and timeline."),
    ("disk", "Disk", "The disk panel when space runs short, a fix's sheet and its result."),
    ("connection", "Daemon and machine", "Attaching to the daemon: the daemon gone (under a banner, or with nothing to show yet), equipe not installed, the machine chooser."),
]
SCREENS = {
    "first-run/git": ("Git (developer tools)", ["views/setup.rs", "setup.rs"], "A card at the top of the AI screen (Let’s connect an AI first), shown only on a Mac without Apple's developer tools."),
    "first-run/ai": ("Connect the AI (ChatGPT · Claude Code)", ["views/setup.rs", "setup.rs", "engines.rs"], "Two cards: ChatGPT (sign in, the easiest way) and Claude Code (install, then sign in; edits files). Next lights up once one is connected. The install error is a canned curl failure."),
    "first-run/squad": ("First squad", ["views/welcome.rs"], None),
    "first-run/usage-data": ("Anonymous usage data", ["views/telemetry.rs"], "Asked once, after the first squad is made on a first run, or the first open after the update; two buttons of the same weight."),
    "inbox/inbox": ("Inbox", ["views/sidebar.rs", "views/board.rs", "views/peek.rs", "board.rs"], None),
    "inbox/decision": ("A member's questions (decision card)", ["views/decide.rs", "decide.rs", "visuals.rs", "views/visuals.rs"], None),
    "inbox/permission": ("Permission requests", ["views/approvals.rs"], "Allow once (Y/A), always (⇧A), decline (N)."),
    "inbox/your-turn": ("A ticket put down for the person", ["views/yourturn.rs", "yourturn.rs"], None),
    "inbox/team-change": ("A change to the team", ["views/team.rs", "team.rs"], None),
    "inbox/routine": ("A routine the PM proposes", ["views/routine.rs", "routine.rs"], None),
    "inbox/accept": ("Work to confirm", ["views/accept.rs", "accept.rs"], None),
    "conversation/chat": ("PM conversation", ["views/chat.rs", "render/view.rs", "render/prose.rs", "render/raster.rs", "views/compact.rs", "views/asklist.rs", "convo.rs"], "Diagrams (```mermaid) are drawn by the app itself (render/raster.rs). HTML cards (```html card) are off until the T-340 security review: the app shows such a block as code, like the code blocks in the Markdown state, so no state of its own yet — add one when cards are turned on."),
    "conversation/engine-switch": ("Engine switching card (over the composer)", ["views/switching.rs", "switching.rs", "usage.rs"], "Error: Claude's week used up; Done: what equipe did in auto, with Undo."),
    "conversation/composer": ("Composer", ["views/composer.rs", "views/attached.rs", "attach.rs", "compose.rs"], None),
    "board/board": ("Board", ["views/tickets.rs", "views/ticket_card.rs", "views/sidebar.rs", "tickets.rs"], None),
    "board/working": ("In progress (at work)", ["views/tickets.rs", "views/sidebar.rs", "board.rs"], None),
    "board/ticket": ("Ticket", ["views/peek.rs", "views/tickets.rs", "views/diffview.rs"], None),
    "overview/overview": ("Overview", ["views/overview.rs", "portfolio.rs"], None),
    "overview/roadmap": ("Roadmap", ["views/roadmap.rs", "roadmap.rs"], None),
    "overview/routines": ("Routines", ["views/routines.rs", "routine.rs"], "The two routines are the snapshot tool's own (`rseed`), no daemon asked."),
    "settings/general": ("General · privacy", ["views/settings.rs"], "Privacy (anonymous usage data) is a section of General."),
    "settings/engines": ("AI platforms", ["views/engines.rs", "engines.rs", "views/settings_switching.rs"], None),
    "settings/squad": ("Squad", ["views/settings.rs"], None),
    "settings/team": ("Team · signals", ["views/settings.rs", "views/settings_signals.rs", "views/team.rs"], "Signals are a card of their own under Team."),
    "settings/permissions": ("Permissions", ["views/settings.rs"], None),
    "settings/projects": ("Projects", ["views/settings.rs"], None),
    "settings/browser": ("Browser", ["views/settings.rs"], None),
    "settings/accounts": ("Accounts (logins)", ["views/accounts.rs"], "The two logins are the snapshot tool's own (`lseed`)."),
    "settings/keys": ("Keys", ["views/secrets.rs", "views/settings.rs"], None),
    "settings/about": ("About", ["views/settings.rs", "update.rs"], None),
    "overview/metrics": ("Metrics", ["views/metrics.rs", "metrics.rs"], "On the app's own sample (`metrics:`), no daemon asked."),
    "overview/design": ("Design system", ["views/design.rs", "design.rs"], None),
    "tools/browser": ("Browser panel", ["views/browser.rs", "browse.rs"], None),
    "tools/terminal": ("Terminal panel", ["views/terminal.rs", "term.rs"], "A real shell on this Mac, in the shot's temporary home."),
    "tools/document": ("Document panel", ["views/doc.rs", "doc.rs"], None),
    "tools/subagent": ("A subagent", ["views/subagent.rs"], None),
    "tools/pair-phone": ("Pair a phone", ["views/pair.rs", "pair.rs"], None),
    "tools/notifications": ("Notifications sheet", ["views/bell.rs", "notify.rs"], None),
    "studio/designer": ("Designer studio", ["views/studio.rs", "studio.rs"], "The waiting state shows work paused for the person's answer."),
    "disk/panel": ("Disk panel", ["views/disk.rs", "disk.rs"], None),
    "disk/fix": ("A fix's sheet", ["views/disk.rs", "disk.rs"], None),
    "connection/daemon": ("Daemon", ["views/connect.rs", "daemon.rs", "views/header.rs"], None),
    "connection/install": ("equipe not on this Mac", ["main.rs", "install.rs"], "No `equipe` command to wake the daemon with: the install line, copy and the install page."),
    "connection/chooser": ("Machine chooser", ["views/connect.rs", "ssh.rs"], None),
}
KINDS = {"default", "empty", "loading", "error", "long", "success", "permission", "offline", "first"}
LABELS = {
    "default": "Default", "empty": "Empty", "loading": "Loading / under way", "error": "Error", "long": "Long text (German)",
    "success": "Done", "first": "First run", "offline": "Daemon gone (banner over the last state)", "offline-first": "Daemon off, nothing shown yet", "permission": "Asking permission",
    "confirm": "Install confirmed before it runs", "signedout": "Installed, signed out", "waiting": "Waiting for the browser sign-in",
    "pictures": "With pictures", "review": "In review, with a review", "page": "Opened as a page (⏎)", "busy": "Busy squad with subagents",
    "ticket-cards": "Ticket cards in a reply", "ask-list": "A reply asking a numbered list", "diagram": "Mermaid diagrams",
    "markdown": "Rich Markdown (headings, lists, callouts, code)", "summarizing": "Being summarized (long)", "summarize-failed": "Summary failed",
    "your-turn": "A ticket put down for the person, in the conversation", "attachments": "Two files attached", "drop": "A file dragged over",
    "queued": "A message waiting while the member works", "handover": "Browser handed to the person", "form": "Form open", "draft": "A draft in review",
    "connect": "Connect a service", "full": "Disk full",
}
# Flows recorded as a video by design/flows/record-desktop.py (its FLOWS).
RECORDED = {"first-run"}

# States that no fake daemon or snapshot step reaches: drawn as HTML mockups
# (design/flows/mockups/) and rendered to the same paths.
MOCKUPS = {}
NOTES = {}


def write_map():
    flows = []
    by_screen = {}
    for path, env, steps, opts in STATES:
        flow, screen, st = path.split("/")
        by_screen.setdefault(f"{flow}/{screen}", []).append((st, env, steps, opts))
    for path, (label, kind, note) in MOCKUPS.items():
        flow, screen, st = path.split("/")
        by_screen.setdefault(f"{flow}/{screen}", []).append((st, None, None, {"mockup": note, "label": label, "kind": kind}))
    for fid, title, summary in FLOWS:
        screens = []
        for key, (stitle, source, note) in SCREENS.items():
            if not key.startswith(fid + "/"):
                continue
            states = []
            for st, env, steps, opts in by_screen.get(key, []):
                kind = opts.get("kind") or (st if st in KINDS else "default")
                if st == "permission" and key == "tools/pair-phone":
                    kind = "permission"
                s = {"id": st, "label": opts.get("label") or LABELS.get(st, st.replace("-", " ").capitalize()), "kind": kind}
                if opts.get("mockup"):
                    s["note"] = "HTML mockup (design/flows/mockups/): " + opts["mockup"]
                else:
                    how = []
                    if env:
                        how.append(" ".join(f"{k}={v}" for k, v in env.items()))
                    if steps:
                        how.append(f"script `{steps}`")
                    if opts.get("no_daemon"):
                        how.append("no daemon")
                    if opts.get("kill_daemon_ms"):
                        how.append(f"fake daemon stopped {opts['kill_daemon_ms']} ms after the app opened")
                    if opts.get("ask_telemetry"):
                        how.append("usage data unanswered")
                    if opts.get("lang"):
                        how.append(f"EQUIPE_LANG={opts['lang']}")
                    extra = NOTES.get(f"{key}/{st}")
                    s["note"] = (extra + " — " if extra else "") + "shot by design/flows/shoot-desktop.py: " + ("; ".join(how) or "fake daemon as it starts")
                states.append(s)
            sc = {"id": key.split("/")[1], "title": stitle}
            if note:
                sc["note"] = note
            sc["source"] = [V + f for f in source]
            sc["expects"] = sorted({s["kind"] for s in states})
            sc["states"] = states
            screens.append(sc)
        flow = {"id": fid, "title": title, "summary": summary, "screens": screens}
        if fid in RECORDED:
            flow["record_command"] = f"python3 design/flows/record-desktop.py --flow {fid} --out {{out}} --mode {{mode}} --width {{width}}"
        flows.append(flow)
    doc = {"note": "The equipe desktop app (crates/desktop) as it is. It has no URL, so `equipe design flows shoot` can't reach it: every picture is the real app built with `--features snapshot`, attached to the fake daemon (crates/desktop/examples/fake_daemon.rs) and driven by a snapshot script — see design/flows/shoot-desktop.py, which also writes this file. Widths are the window's in points, 1x: 1200 (as it opens) and 760 (the narrowest it allows, so every state's 760 picture is the narrow window). English UI unless a state names a language; `long` states are German. Each run has a throwaway home with sample SSH hosts and a stand-in `tailscale` (design/flows/fixtures/), so nothing of this Mac's own is pictured. No flow has a `walk` (`flows record` plays a walk in a browser, which can't drive the app): a recorded flow has a `record_command`, design/flows/record-desktop.py, which steps the same snapshot build through it and writes the video with its screen marks — record with `--widths 1200,760`.", "flows": flows}
    with open(os.path.join(HERE, "flows.json"), "w") as f:
        json.dump(doc, f, ensure_ascii=False, indent=2)
        f.write("\n")


def shrink(png):
    """Lossless: RGB (the window is opaque) and zlib's best. Never fewer
    colours — a palette shifts the accent and status colours the map shows."""
    try:
        from PIL import Image
    except ImportError:
        return
    Image.open(png).convert("RGB").save(png, optimize=True)


def run_state(item, bin_dir, root):
    path, env, steps, opts = item
    flow, screen, st = path.split("/")
    out = os.path.join(HERE, flow, screen)
    os.makedirs(out, exist_ok=True)
    tmp = tempfile.mkdtemp(prefix="t379-")
    home = os.path.join(tmp, "home")
    os.makedirs(home)
    if not opts.get("ask_telemetry"):
        json.dump({"consent": "off"}, open(os.path.join(home, "telemetry.json"), "w"))
    # A home of its own, with sample SSH hosts and a stand-in `tailscale`, so
    # nothing of this Mac's own (its ~/.ssh/config, its tailnet) is pictured.
    user = os.path.join(tmp, "user")
    os.makedirs(os.path.join(user, ".ssh"))
    shutil.copy(os.path.join(HERE, "fixtures", "ssh", "config"), os.path.join(user, ".ssh", "config"))
    if opts.get("home_squad"):
        os.makedirs(os.path.join(home, "squads", "demo"))
        open(os.path.join(home, "squads", "demo", "squad.db"), "w").close()
    base = dict(os.environ)
    base["PATH"] = os.path.join(HERE, "fixtures", "bin") + os.pathsep + base.get("PATH", "")
    if opts.get("cli") in ("none", "fails"):
        # No real `equipe` to wake a daemon with: none at all (the install
        # screen), or one that can't start it (the daemon off).
        base["PATH"] = os.pathsep.join(d for d in base["PATH"].split(os.pathsep) if not os.path.isfile(os.path.join(d, "equipe")))
        if opts["cli"] == "fails":
            base["PATH"] = os.path.join(HERE, "fixtures", "no-daemon") + os.pathsep + base["PATH"]
    base.update({"HOME": user, "EQUIPE_HOME": home, "EQUIPE_SQUAD": "demo", "EQUIPE_TELEMETRY": "off",
                 "EQUIPE_LANG": opts.get("lang", "en"), "TMPDIR": tmp + "/", "EQUIPE_SNAPSHOT_SCALE": "1"})
    daemon = None
    if not opts.get("no_daemon"):
        denv = dict(base)
        denv.update(env)
        daemon = subprocess.Popen([os.path.join(bin_dir, "examples", "fake_daemon")], env=denv,
                                  stdout=subprocess.DEVNULL, stderr=open(os.path.join(tmp, "daemon.log"), "w"))
        time.sleep(1.5)
    script = ["wait:2500"]
    script += [s.replace("{ROOT}", root) for s in steps.split(";") if s]
    script.append(f"wait:{opts.get('wait', 900)}")
    for w, h in opts.get("sizes", SIZES):
        script += [f"size:{w}x{h}", "wait:700"]
        for mode in MODES:
            script += [f"theme:{mode}", "wait:500", f"shot:{out}/{st}.{mode}.{w}.png"]
    script.append("quit")
    aenv = dict(base)
    aenv["EQUIPE_SCRIPT"] = ";".join(script)
    log = open(os.path.join(tmp, "app.log"), "w")
    app = subprocess.Popen([os.path.join(bin_dir, "equipe-desktop")], env=aenv, stdout=log, stderr=log, cwd=user)
    if opts.get("kill_daemon_ms") and daemon:
        time.sleep(opts["kill_daemon_ms"] / 1000)
        daemon.kill()
    try:
        app.wait(timeout=120)
    except subprocess.TimeoutExpired:
        app.kill()
    if daemon:
        daemon.kill()
    # With no fake daemon the app wakes a real one, in this throwaway home
    # (never as a login service: EQUIPE_HOME is set). Stop whatever has it open.
    pids = subprocess.run(["lsof", "-t", "+D", home], capture_output=True, text=True).stdout.split()
    for pid in set(pids):
        subprocess.run(["kill", pid])
    shots = sorted(f for f in os.listdir(out) if f.startswith(st + "."))
    want = len(opts.get("sizes", SIZES)) * len(MODES)
    ok = sum(1 for f in shots if os.path.getmtime(os.path.join(out, f)) > time.time() - 300)
    status = "ok" if ok >= want else f"MISSING ({ok}/{want}) log {tmp}"
    if ok >= want:
        shutil.rmtree(tmp, ignore_errors=True)
        for f in shots:
            shrink(os.path.join(out, f))
    return f"{path}: {status}"


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--bin", help="the target's debug folder")
    p.add_argument("--only", default="", help="paths starting with this")
    p.add_argument("--jobs", type=int, default=3)
    p.add_argument("--list", action="store_true")
    p.add_argument("--map", action="store_true", help="write flows.json and stop")
    a = p.parse_args()
    root = os.path.dirname(os.path.dirname(HERE))
    todo = [s for s in STATES if s[0].startswith(a.only)]
    if a.map:
        write_map()
        return
    if a.list:
        print("\n".join(s[0] for s in todo))
        return
    with ThreadPoolExecutor(a.jobs) as pool:
        for line in pool.map(lambda s: run_state(s, a.bin, root), todo):
            print(line, flush=True)


if __name__ == "__main__":
    sys.exit(main())
