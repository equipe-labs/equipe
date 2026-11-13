# Linux desktop control: the live runs

`vm-xfce.txt` is the run on a real Xfce desktop (an Ubuntu 24.04 x86_64 VM,
Xvnc + xfwm4 with Mousepad and Chrome open): `equipe desktop setup` from a
squad with no `desktop on`, the daemon as a systemd user unit, and one real
Claude Code member taking a screenshot, clicking into Mousepad, pressing keys
and typing a line, read back independently afterwards; plus one capture with
xfwm4's compositor on.

`run.sh` checks equipe's Linux desktop control against a real X11 desktop in a
throwaway Ubuntu 24.04 container (aarch64, Docker on a Mac): Xvfb, openbox,
`at-spi2-core` 2.52, `libatk-adaptor`, and a GTK `zenity --entry` dialog.
`helper_call.py` sends one request to the helper's socket with its token, the
way the daemon does. `live.txt` is its latest output.

What it covers, in order:

1. a bare shell (no `DISPLAY`, no session bus): `equipe desktop status` and
   `setup` say what is missing; `setup` still downloads and verifies the
   pinned aarch64 engine, then exits 1;
2. a display and a bus but no AT-SPI service: the AT-SPI line says why
   (`org.a11y.Bus was not provided by any .service files`) and what to do;
3. a full session: `setup` starts the helper inside the session and reports
   ready; through the helper, `snapshot` of zenity returns refs, the send gate's
   `_scene` for typing names the text field, `act type` is delivered and
   verified, a fresh snapshot shows the value, `act click` on OK closes the
   dialog with the typed text;
4. the screenshot, pointer and keyboard tools on a second dialog: a
   whole-screen and a window screenshot, `list_windows`, `elements` refused
   with what to use instead, a click outside the app refused before any input,
   `type_text`, `alt+f4` refused by agent-desktop and told, `return` closing
   the dialog with the typed text; `equipe desktop remove` stops the helper.

Not covered by either: Wayland, GNOME Shell, lock detection, the Chromium
headed-click fallback (unit-tested), and a person using the machine while a
member waits for the screen (the daemon's person-first wait is
platform-independent and unit-tested). The VM run covers what the container
can't: the daemon, the x86_64 archive, the user manager's environment, a
compositing window manager and a real member.

To run it again (from the repository, with Docker):

```bash
docker run --rm -v "$PWD":/src:ro -v eq-linux-target:/target -v eq-linux-out:/out -e CARGO_TARGET_DIR=/target -w /src rust:1.99-bookworm bash -c 'cargo build -p equipe --bin equipe && cp /target/debug/equipe /out/'
```

```bash
tar -C docs/research/desktop-control/linux -c run.sh helper_call.py | docker run --rm -i -v eq-linux-out:/out:ro ubuntu:24.04 bash -c 'mkdir /live && tar -x -C /live && bash /live/run.sh'
```
