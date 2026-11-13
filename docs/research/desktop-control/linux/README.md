# Linux desktop control: the live run

`run.sh` checks equipe's Linux desktop control against a real X11 desktop in a
throwaway Ubuntu 24.04 container (aarch64, Docker on a Mac): Xvfb, openbox,
`at-spi2-core` 2.52, `libatk-adaptor`, and a GTK `zenity --entry` dialog.
`helper_call.py` sends one request to the helper's socket with its token, the
way the daemon does. `live.log` is the output of the run in the PR that added
Linux support.

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
   dialog with the typed text; the screenshot tools are refused with what to use
   instead; `equipe desktop remove` stops the helper.

Not covered: the daemon itself (its lease, person-first wait and send gate are
platform-independent and unit-tested; the daemon wasn't run in the container),
the systemd user-manager environment fallback, Wayland, GNOME Shell, and the
x86_64 archive (its hashes were computed from the downloaded release, and its
Sigstore attestation verified, but it wasn't run).

To run it again (from the repository, with Docker):

```bash
docker run --rm -v "$PWD":/src:ro -v eq-linux-target:/target -v eq-linux-out:/out -e CARGO_TARGET_DIR=/target -w /src rust:1.99-bookworm bash -c 'cargo build -p equipe --bin equipe && cp /target/debug/equipe /out/'
```

```bash
tar -C docs/research/desktop-control/linux -c run.sh helper_call.py | docker run --rm -i -v eq-linux-out:/out:ro ubuntu:24.04 bash -c 'mkdir /live && tar -x -C /live && bash /live/run.sh'
```
