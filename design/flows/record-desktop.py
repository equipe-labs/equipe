#!/usr/bin/env python3
"""Record a desktop flow as a video: the `record_command` of flows.json.

`equipe design flows record` plays a flow's `walk` in a browser, which can't
drive the desktop app. A desktop flow names this script instead; it runs the
real app built with `--features snapshot` (as shoot-desktop.py does), steps it
through the flow with the snapshot tool, takes a burst of frames at each beat,
and joins them with a caption strip under the window into `{out}` — with
`walkthrough.json` beside it marking when each screen begins.

    python3 design/flows/record-desktop.py --flow first-run --out OUT.mp4 --mode light --width 1200

The app is found at $EQUIPE_DESKTOP_BIN, else $CARGO_TARGET_DIR/debug. Widths
are the window's in points at 1x, as in the pictures: 1200 and 760.
"""

import argparse
import json
import os
import shutil
import subprocess
import tempfile
import time

from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
SIZES = {1200: 760, 760: 600}
FPS = 10
CAPTION = 64

# A flow is runs of the app; a run is beats; a beat is
# (screen or None, caption, snapshot steps, frames, ms between frames, hold ms).
# The first beat of a screen marks where it begins.
FLOWS = {
    "first-run": [
        # A Mac with no squad has no daemon: each step is held by `setup:`.
        {"daemon": False, "beats": [
            ("git", "1 · Connect AI — a Mac without git has the developer tools card on top", "setup:nogit", 4, 250, 2200),
            (None, "1 · Connect AI — git installs in macOS's own window; here it waits", "setup:gitasked", 6, 250, 1800),
            ("ai", "2 · Connect AI — sign in with ChatGPT or install Claude Code", "setup:missing", 4, 250, 2400),
            (None, "2 · Connect AI — before installing, it shows the command to run and asks", "setup:confirm", 4, 250, 2400),
            (None, "2 · Connect AI — installing Claude Code (1–2 min)", "setup:installing", 8, 250, 1500),
            (None, "2 · Connect AI — installed. Sign in with Claude", "setup:signedout", 4, 250, 2200),
            (None, "2 · Connect AI — waiting for the sign-in in the browser", "setup:waiting", 8, 250, 1500),
            (None, "2 · Connect AI — both connected, on to the next", "setup:ready", 4, 250, 2200),
            ("squad", "3 · First squad — name, team, how much it works on its own, folder", "setup:squad", 4, 250, 3000),
            (None, "3 · First squad — being made", "setup:creating;wait:300;scroll:20,-80", 8, 250, 1800),
        ]},
        # Once the squad is made the app attaches to its daemon (the fake one
        # here) and asks the usage question, once.
        {"daemon": True, "ask_telemetry": True, "beats": [
            ("usage-data", "4 · Anonymous usage data — asked once, after the squad is made. Both buttons weigh the same", "", 6, 250, 3000),
        ]},
    ],
}


def font(size):
    for path in ("/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Helvetica.ttc"):
        if os.path.exists(path):
            return ImageFont.truetype(path, size)
    return ImageFont.load_default()


def home_for(tmp, ask_telemetry):
    home = os.path.join(tmp, "home")
    os.makedirs(home)
    if not ask_telemetry:
        json.dump({"consent": "off"}, open(os.path.join(home, "telemetry.json"), "w"))
    user = os.path.join(tmp, "user")
    os.makedirs(os.path.join(user, ".ssh"))
    shutil.copy(os.path.join(HERE, "fixtures", "ssh", "config"), os.path.join(user, ".ssh", "config"))
    return home, user


def run(bin_dir, run_spec, frames_dir, prefix, mode, width):
    """One app run: its beats' frames into frames_dir, named prefix-beat-frame."""
    tmp = tempfile.mkdtemp(prefix="rec-")
    home, user = home_for(tmp, run_spec.get("ask_telemetry"))
    env = dict(os.environ)
    env["PATH"] = os.path.join(HERE, "fixtures", "bin") + os.pathsep + env.get("PATH", "")
    env.update({"HOME": user, "EQUIPE_HOME": home, "EQUIPE_SQUAD": "demo", "EQUIPE_TELEMETRY": "off",
                "EQUIPE_LANG": "en", "TMPDIR": tmp + "/", "EQUIPE_SNAPSHOT_SCALE": "1"})
    daemon = None
    if run_spec.get("daemon"):
        daemon = subprocess.Popen([os.path.join(bin_dir, "examples", "fake_daemon")], env=env,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(1.5)
    script = [f"size:{width}x{SIZES[width]}", f"theme:{mode}", "wait:2500"]
    for b, (_screen, _caption, steps, count, gap, _hold) in enumerate(run_spec["beats"]):
        script += [s for s in steps.split(";") if s]
        script.append("wait:600")
        for f in range(count):
            script += [f"shot:{frames_dir}/{prefix}-{b:02d}-{f:02d}.png", f"wait:{gap}"]
    script.append("quit")
    env["EQUIPE_SCRIPT"] = ";".join(script)
    app = subprocess.Popen([os.path.join(bin_dir, "equipe-desktop")], env=env, cwd=user,
                           stdout=open(os.path.join(tmp, "app.log"), "w"), stderr=subprocess.STDOUT)
    try:
        app.wait(timeout=180)
    except subprocess.TimeoutExpired:
        app.kill()
    if daemon:
        daemon.kill()
    for pid in set(subprocess.run(["lsof", "-t", "+D", home], capture_output=True, text=True).stdout.split()):
        subprocess.run(["kill", pid])
    shutil.rmtree(tmp, ignore_errors=True)


def captioned(png, caption, mode, width):
    win = Image.open(png).convert("RGB")
    bg, fg = ((246, 245, 241), (21, 22, 26)) if mode == "light" else ((10, 10, 12), (239, 237, 231))
    out = Image.new("RGB", (win.width, win.height + CAPTION), bg)
    out.paste(win, (0, 0))
    draw = ImageDraw.Draw(out)
    draw.line([(0, win.height), (win.width, win.height)], fill=(128, 128, 128), width=1)
    draw.text((24, win.height + CAPTION // 2), caption, fill=fg, font=font(22 if width > 800 else 18), anchor="lm")
    return out


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--flow", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--mode", default="light")
    p.add_argument("--width", type=int, default=1200)
    a = p.parse_args()
    bin_dir = os.environ.get("EQUIPE_DESKTOP_BIN") or os.path.join(os.environ["CARGO_TARGET_DIR"], "debug")
    width = a.width if a.width in SIZES else 1200
    work = tempfile.mkdtemp(prefix="rec-frames-")
    runs = FLOWS[a.flow]
    for r, spec in enumerate(runs):
        run(bin_dir, spec, work, f"r{r}", a.mode, width)
    # One still per video frame, so the marks fall on the video's own clock.
    seq = os.path.join(work, "seq")
    os.makedirs(seq)
    n, marks = 0, []
    per = 1000 // FPS
    for r, spec in enumerate(runs):
        for b, (screen, caption, _steps, count, gap, hold) in enumerate(spec["beats"]):
            shots = [os.path.join(work, f"r{r}-{b:02d}-{f:02d}.png") for f in range(count)]
            shots = [s for s in shots if os.path.exists(s)]
            if not shots:
                raise SystemExit(f"no frames for {screen or caption}: is the app built with --features snapshot at {bin_dir}?")
            if screen:
                marks.append({"screen": screen, "at": n * per})
            # The burst as it was taken, then its last frame held.
            plan = [(s, gap) for s in shots] + [(shots[-1], hold)]
            for shot, ms in plan:
                first = os.path.join(seq, f"{n:05d}.png")
                captioned(shot, caption, a.mode, width).save(first)
                n += 1
                for _ in range(max(1, ms // per) - 1):
                    os.link(first, os.path.join(seq, f"{n:05d}.png"))
                    n += 1
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-framerate", str(FPS), "-i", os.path.join(seq, "%05d.png"),
                    "-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2", "-c:v", "libx264", "-pix_fmt", "yuv420p",
                    "-movflags", "+faststart", a.out], check=True)
    with open(os.path.splitext(a.out)[0] + ".json", "w") as f:
        json.dump({"millis": n * per, "marks": marks}, f)
    shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
