//! Linux: the desktop helper runs inside the person's graphical session, and
//! what that session must give it before agent-desktop can work.
//!
//! Linux has no per-app permission to grant. What members need instead is a
//! session to reach: a display (X11 or Wayland), the D-Bus session bus, and
//! the AT-SPI bus behind it (`at-spi2-core`), which is where apps publish
//! their accessibility trees. [`probe`] checks each and says, in a line the
//! person can act on, what is missing.
//!
//! The daemon usually runs as a systemd user service, whose environment may
//! lack the session's variables; [`session_env`] fills them in from the user
//! manager's environment (which desktops populate at login), and the helper is
//! started with them, so it runs inside the session it is about.
//!
//! The pure parts here (parsing, judging) build and are tested on every
//! platform; only the parts that spawn processes or serve are Linux-only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::time::Duration;

/// The variables that place a process in a graphical session.
pub const SESSION_VARS: &[&str] = &[
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_ID",
];

/// The session's variables: this process's own, each missing one taken from
/// `manager` (`systemctl --user show-environment`'s output) when it has it.
/// With no bus address, the per-user bus systemd keeps at
/// `$XDG_RUNTIME_DIR/bus` is used when `exists` finds it.
pub fn session_env(own: impl Fn(&str) -> Option<String>, manager: Option<&str>, exists: impl Fn(&Path) -> bool) -> BTreeMap<String, String> {
    let from_manager = manager.map(parse_environment).unwrap_or_default();
    let mut env = BTreeMap::new();
    for var in SESSION_VARS {
        if let Some(value) = own(var).filter(|v| !v.is_empty()).or_else(|| from_manager.get(*var).cloned().filter(|v| !v.is_empty())) {
            env.insert((*var).to_owned(), value);
        }
    }
    if !env.contains_key("DBUS_SESSION_BUS_ADDRESS")
        && let Some(runtime) = env.get("XDG_RUNTIME_DIR")
    {
        // A Linux path, spelled out so the same answer comes on every host the tests run on.
        let bus = format!("{}/bus", runtime.trim_end_matches('/'));
        if exists(Path::new(&bus)) {
            env.insert("DBUS_SESSION_BUS_ADDRESS".into(), format!("unix:path={bus}"));
        }
    }
    env
}

/// `KEY=value` lines (`systemctl --user show-environment`), values unquoted
/// the way systemd quotes them (`$'…'` or plain).
pub fn parse_environment(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return None;
            }
            let value = value.strip_prefix("$'").and_then(|v| v.strip_suffix('\'')).unwrap_or(value);
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

/// Which kind of display the session has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Display {
    X11(String),
    Wayland(String),
}

impl Display {
    pub fn kind(&self) -> &'static str {
        match self {
            Display::X11(_) => "x11",
            Display::Wayland(_) => "wayland",
        }
    }
}

/// What the session gives the helper, check by check. Each `Err` is the
/// line to show the person: what is missing and what to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readiness {
    pub display: Result<Display, String>,
    pub bus: Result<(), String>,
    /// The AT-SPI bus's address, when it answered.
    pub atspi: Result<String, String>,
    /// Whether toolkits are told to expose their trees (`org.a11y.Status`
    /// `IsEnabled`); `None` when it couldn't be read. Not a blocker: GTK apps
    /// with the AT-SPI bridge loaded register anyway, but many apps wait for it.
    pub toolkits: Option<bool>,
}

impl Readiness {
    /// The session is reachable: a display and a bus.
    pub fn session(&self) -> bool {
        self.display.is_ok() && self.bus.is_ok()
    }

    /// Everything agent-desktop needs from the session.
    pub fn ok(&self) -> bool {
        self.session() && self.atspi.is_ok()
    }

    /// What is missing, a line each, in the order to fix it.
    pub fn missing(&self) -> Vec<String> {
        [self.display.as_ref().err(), self.bus.as_ref().err(), self.atspi.as_ref().err()].into_iter().flatten().cloned().collect()
    }

    /// Only the display and the bus: what can be told without asking the
    /// session anything (the daemon's quick look).
    pub fn session_missing(&self) -> Vec<String> {
        [self.display.as_ref().err(), self.bus.as_ref().err()].into_iter().flatten().cloned().collect()
    }
}

/// What a probe asks the session bus. Behind a trait so the judging is
/// tested without a bus.
pub trait Bus {
    /// `org.a11y.Bus.GetAddress`: the AT-SPI bus's address, or why not.
    fn atspi_address(&self, env: &BTreeMap<String, String>) -> Result<String, String>;
    /// `org.a11y.Status.IsEnabled`.
    fn toolkits_enabled(&self, env: &BTreeMap<String, String>) -> Option<bool>;
}

pub const NO_DISPLAY: &str = "No graphical session: neither DISPLAY nor WAYLAND_DISPLAY is set. Run equipe from a terminal inside your desktop session (not a bare SSH shell or a TTY), or have the session export them (`systemctl --user import-environment DISPLAY WAYLAND_DISPLAY XAUTHORITY`).";
pub const NO_BUS: &str = "No D-Bus session bus: DBUS_SESSION_BUS_ADDRESS isn't set and $XDG_RUNTIME_DIR/bus isn't there. Log in to a desktop session (or start one under `dbus-run-session`).";

/// Check the session `env` describes. `exists` looks for sockets; `bus`
/// asks AT-SPI (only once there is a bus to ask).
pub fn probe(env: &BTreeMap<String, String>, exists: impl Fn(&Path) -> bool, bus: &dyn Bus) -> Readiness {
    let display = display(env, &exists);
    let session_bus = session_bus(env, &exists);
    let (atspi, toolkits) = match &session_bus {
        Ok(()) => (
            bus.atspi_address(env).map_err(|why| {
                format!(
                    "AT-SPI (the desktop's accessibility bus) isn't answering on the session bus ({why}). Install at-spi2-core (and, for GTK apps, the AT-SPI bridge: libatk-adaptor), then log out and in, or restart it: `systemctl --user restart at-spi-dbus-bus`."
                )
            }),
            bus.toolkits_enabled(env),
        ),
        Err(_) => (Err("AT-SPI can't be reached without a session bus.".to_owned()), None),
    };
    Readiness { display, bus: session_bus, atspi, toolkits }
}

fn display(env: &BTreeMap<String, String>, exists: &impl Fn(&Path) -> bool) -> Result<Display, String> {
    let runtime = env.get("XDG_RUNTIME_DIR").map(PathBuf::from);
    if let Some(name) = env.get("WAYLAND_DISPLAY") {
        let socket = if Path::new(name).is_absolute() { Some(PathBuf::from(name)) } else { runtime.as_ref().map(|r| r.join(name)) };
        match socket {
            Some(socket) if exists(&socket) => return Ok(Display::Wayland(name.clone())),
            // An X11 display beside it (XWayland) still works for most apps.
            _ if env.contains_key("DISPLAY") => {}
            Some(socket) => return Err(format!("WAYLAND_DISPLAY is {name}, but there is no compositor socket at {}: the session it named is gone. Run equipe from inside your current desktop session.", socket.display())),
            None => return Err(format!("WAYLAND_DISPLAY is {name}, but XDG_RUNTIME_DIR isn't set, so its socket can't be found. Run equipe from inside your desktop session.")),
        }
    }
    let Some(name) = env.get("DISPLAY") else { return Err(NO_DISPLAY.to_owned()) };
    match x11_socket(name) {
        Some(socket) if !exists(&socket) => Err(format!(
            "DISPLAY is {name}, but no X server listens at {}: the session it named is gone. Run equipe from inside your current desktop session.",
            socket.display()
        )),
        // A local socket that is there, or a TCP display, which can't be told without connecting.
        _ => Ok(Display::X11(name.clone())),
    }
}

/// The socket of a local X display (`:0`, `:1.0`, `unix:0`); `None` for a
/// TCP one (`host:0`).
pub fn x11_socket(display: &str) -> Option<PathBuf> {
    let (host, rest) = display.rsplit_once(':')?;
    if !(host.is_empty() || host == "unix") {
        return None;
    }
    let number = rest.split('.').next().filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))?;
    Some(PathBuf::from(format!("/tmp/.X11-unix/X{number}")))
}

fn session_bus(env: &BTreeMap<String, String>, exists: &impl Fn(&Path) -> bool) -> Result<(), String> {
    let Some(address) = env.get("DBUS_SESSION_BUS_ADDRESS") else { return Err(NO_BUS.to_owned()) };
    // Several addresses may be listed (`;`); one that can be reached is enough.
    let mut paths = Vec::new();
    for part in address.split(';') {
        let Some(rest) = part.strip_prefix("unix:") else { return Ok(()) };
        let mut path = None;
        for kv in rest.split(',') {
            match kv.split_once('=') {
                Some(("path", p)) => path = Some(p),
                // Abstract sockets and `dir`/`runtime` forms can't be looked at as files.
                Some(("abstract" | "dir" | "tmpdir" | "runtime", _)) => return Ok(()),
                _ => {}
            }
        }
        if let Some(p) = path {
            if exists(Path::new(p)) {
                return Ok(());
            }
            paths.push(p.to_owned());
        }
    }
    Err(format!(
        "DBUS_SESSION_BUS_ADDRESS points at {}, which isn't there: the session it named is gone. Run equipe from inside your current desktop session.",
        if paths.is_empty() { address.clone() } else { paths.join(", ") }
    ))
}

/// The address in a D-Bus client's reply to `GetAddress`: gdbus prints
/// `('unix:path=…',)`, busctl `s "unix:path=…"`, dbus-send `string "unix:path=…"`.
pub fn parse_address(reply: &str) -> Option<String> {
    let start = reply.find(['\'', '"'])?;
    let quote = reply[start..].chars().next()?;
    let rest = &reply[start + 1..];
    let end = rest.find(quote)?;
    let address = &rest[..end];
    (!address.is_empty() && address.contains(':')).then(|| address.to_owned())
}

/// A boolean in a D-Bus client's reply: `(<true>,)`, `(true,)`, `v b true`,
/// `variant boolean true`.
pub fn parse_bool(reply: &str) -> Option<bool> {
    let words: Vec<&str> = reply.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    match (words.contains(&"true"), words.contains(&"false")) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

/// The last number in a D-Bus client's reply (`(uint64 1234,)`, `t 1234`), or
/// a bare one (`xprintidle`).
pub fn parse_number(reply: &str) -> Option<u64> {
    reply.split(|c: char| !c.is_ascii_digit()).filter(|w| !w.is_empty()).filter_map(|w| w.parse().ok()).next_back()
}

/// One D-Bus method call, through whichever client the machine has:
/// `gdbus` (GLib), `busctl` (systemd), `dbus-send`. `signature` and
/// `args` are busctl's; gdbus and dbus-send take the typed args given.
#[derive(Debug, Clone, Copy)]
pub struct Call<'a> {
    pub dest: &'a str,
    pub path: &'a str,
    pub interface: &'a str,
    pub method: &'a str,
    /// For a property read (`org.freedesktop.DBus.Properties.Get`): its interface and name.
    pub property: Option<(&'a str, &'a str)>,
}

impl Call<'_> {
    /// Each client's command line for this call, in the order they're tried.
    pub fn commands(&self) -> Vec<(&'static str, Vec<String>)> {
        let full = format!("{}.{}", self.interface, self.method);
        let mut gdbus = vec!["call", "--session", "--timeout", "3", "--dest", self.dest, "--object-path", self.path, "--method"].into_iter().map(String::from).collect::<Vec<_>>();
        let mut busctl = vec!["--user", "--timeout=3", "call", self.dest, self.path, self.interface, self.method].into_iter().map(String::from).collect::<Vec<_>>();
        let mut send = vec!["--session".to_owned(), "--print-reply".to_owned(), "--reply-timeout=3000".to_owned(), format!("--dest={}", self.dest), self.path.to_owned(), full.clone()];
        gdbus.push(full);
        if let Some((interface, name)) = self.property {
            gdbus.extend([interface.to_owned(), name.to_owned()]);
            busctl.extend(["ss".to_owned(), interface.to_owned(), name.to_owned()]);
            send.extend([format!("string:{interface}"), format!("string:{name}")]);
        }
        vec![("gdbus", gdbus), ("busctl", busctl), ("dbus-send", send)]
    }
}

/// A bus that is never asked: a quick look at the session (the daemon's,
/// at each desktop call) checks the display and the bus only, and leaves
/// AT-SPI to the helper, which runs inside the session.
pub struct Unasked;

impl Bus for Unasked {
    fn atspi_address(&self, _env: &BTreeMap<String, String>) -> Result<String, String> {
        Err("not asked".to_owned())
    }
    fn toolkits_enabled(&self, _env: &BTreeMap<String, String>) -> Option<bool> {
        None
    }
}

/// The real session bus, asked through a D-Bus client process.
pub struct SessionBus;

impl SessionBus {
    /// The call's reply, or why not. A client that isn't installed is skipped.
    #[cfg(target_os = "linux")]
    pub fn call(call: &Call, env: &BTreeMap<String, String>) -> Result<String, String> {
        let mut last = "no D-Bus client to ask with (gdbus, busctl or dbus-send)".to_owned();
        for (program, args) in call.commands() {
            match run(program, &args, env, Duration::from_secs(5)) {
                Ok(out) => return Ok(out),
                Err(Run::Missing) => {}
                Err(Run::Failed(why)) => {
                    last = why;
                    // The bus answered with an error: another client won't do better.
                    break;
                }
            }
        }
        Err(last)
    }

    #[cfg(not(target_os = "linux"))]
    pub fn call(_call: &Call, _env: &BTreeMap<String, String>) -> Result<String, String> {
        Err("not a Linux machine".to_owned())
    }
}

pub const ATSPI: Call<'static> = Call { dest: "org.a11y.Bus", path: "/org/a11y/bus", interface: "org.a11y.Bus", method: "GetAddress", property: None };
pub const ATSPI_ENABLED: Call<'static> = Call {
    dest: "org.a11y.Bus",
    path: "/org/a11y/bus",
    interface: "org.freedesktop.DBus.Properties",
    method: "Get",
    property: Some(("org.a11y.Status", "IsEnabled")),
};

impl Bus for SessionBus {
    fn atspi_address(&self, env: &BTreeMap<String, String>) -> Result<String, String> {
        let reply = Self::call(&ATSPI, env)?;
        parse_address(&reply).ok_or_else(|| "its answer had no address".to_owned())
    }

    fn toolkits_enabled(&self, env: &BTreeMap<String, String>) -> Option<bool> {
        Self::call(&ATSPI_ENABLED, env).ok().as_deref().and_then(parse_bool)
    }
}

/// How the screen-locked and idle questions are asked: GNOME first, then
/// the freedesktop names KDE and others answer to.
pub const LOCKED: &[Call<'static>] = &[
    Call { dest: "org.gnome.ScreenSaver", path: "/org/gnome/ScreenSaver", interface: "org.gnome.ScreenSaver", method: "GetActive", property: None },
    Call { dest: "org.freedesktop.ScreenSaver", path: "/org/freedesktop/ScreenSaver", interface: "org.freedesktop.ScreenSaver", method: "GetActive", property: None },
];
pub const IDLE_MUTTER: Call<'static> =
    Call { dest: "org.gnome.Mutter.IdleMonitor", path: "/org/gnome/Mutter/IdleMonitor/Core", interface: "org.gnome.Mutter.IdleMonitor", method: "GetIdletime", property: None };

#[cfg(target_os = "linux")]
enum Run {
    Missing,
    Failed(String),
}

/// Run `program` with only the session's variables changed; its stdout, or why not.
#[cfg(target_os = "linux")]
fn run(program: &str, args: &[String], env: &BTreeMap<String, String>, timeout: Duration) -> Result<String, Run> {
    use std::io::Read as _;
    use std::process::{Command, Stdio};
    let mut child = match Command::new(program).args(args).envs(env).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Err(Run::Missing),
        Err(err) => return Err(Run::Failed(err.to_string())),
    };
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Run::Failed(format!("{program} didn't answer within {} s", timeout.as_secs())));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(err) => return Err(Run::Failed(err.to_string())),
        }
    };
    let mut out = String::new();
    let mut err = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut out);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut err);
    }
    if status.success() {
        Ok(out)
    } else {
        let first = err.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no reason given");
        Err(Run::Failed(first.chars().take(200).collect()))
    }
}

/// This machine's session: this process's variables, completed from the user
/// manager's when it lacks them.
#[cfg(target_os = "linux")]
pub fn this_session() -> BTreeMap<String, String> {
    let own = |var: &str| std::env::var(var).ok();
    // Only asked when something is missing: a terminal in the session has it all.
    let needs_manager = own("DBUS_SESSION_BUS_ADDRESS").is_none() || (own("DISPLAY").is_none() && own("WAYLAND_DISPLAY").is_none());
    let manager = if needs_manager { run("systemctl", &["--user".into(), "show-environment".into()], &BTreeMap::new(), Duration::from_secs(5)).ok() } else { None };
    session_env(own, manager.as_deref(), |p| p.exists())
}

/// Look at this machine's session: display, bus, AT-SPI.
#[cfg(target_os = "linux")]
pub fn check() -> Readiness {
    probe(&this_session(), |p| p.exists(), &SessionBus)
}

/// Whether the session's screen is locked; `None` when nothing answers.
#[cfg(target_os = "linux")]
pub fn locked(env: &BTreeMap<String, String>) -> Option<bool> {
    LOCKED.iter().find_map(|call| SessionBus::call(call, env).ok().as_deref().and_then(parse_bool))
}

/// Seconds since the person last used mouse or keyboard: GNOME's idle
/// monitor, else `xprintidle` on X11; `None` when neither answers, and then
/// no pointer or keyboard input goes in (not knowing is not leave).
#[cfg(target_os = "linux")]
pub fn idle(env: &BTreeMap<String, String>) -> Option<f64> {
    let ms = SessionBus::call(&IDLE_MUTTER, env)
        .ok()
        .as_deref()
        .and_then(parse_number)
        .or_else(|| env.contains_key("DISPLAY").then(|| run("xprintidle", &[], env, Duration::from_secs(3)).ok()).flatten().as_deref().and_then(parse_number))?;
    Some(ms as f64 / 1000.0)
}

/// Where the running helper's process id is kept.
pub fn pid_path(home: &Path) -> PathBuf {
    crate::helper::dir(home).join("helper.pid")
}

/// The person set the desktop up on this machine (`equipe desktop setup`, or
/// the daemon started the helper once): the daemon keeps it running and every
/// member works the desktop by default, as the helper's LaunchAgent does on
/// macOS. `equipe desktop remove` takes it away.
pub fn setup_mark(home: &Path) -> PathBuf {
    crate::helper::dir(home).join("linux-setup")
}

pub fn mark_set_up(home: &Path) -> std::io::Result<()> {
    crate::helper::ensure_dir(home)?;
    std::fs::write(setup_mark(home), b"set up\n")
}

pub fn unmark_set_up(home: &Path) -> bool {
    std::fs::remove_file(setup_mark(home)).is_ok()
}

/// Start the helper (`equipe desktop serve`) detached, inside the session
/// `env` describes, its log appended to the helper's log.
#[cfg(target_os = "linux")]
pub fn start_helper(home: &Path, exe: &Path, env: &BTreeMap<String, String>) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt as _;
    use std::process::{Command, Stdio};
    crate::helper::ensure_dir(home)?;
    let log = std::fs::OpenOptions::new().create(true).append(true).open(crate::helper::log_path(home))?;
    Command::new(exe)
        .args(["desktop", "serve", "--home"])
        .arg(home)
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        // Its own process group: a terminal's Ctrl-C or hang-up doesn't end it.
        .process_group(0)
        .spawn()
        .map(drop)
}

/// Stop the running helper, if the pid file names one that is equipe's
/// `desktop serve`. `true` when one was stopped.
#[cfg(target_os = "linux")]
pub fn stop_helper(home: &Path) -> bool {
    let path = pid_path(home);
    let Some(pid) = std::fs::read_to_string(&path).ok().and_then(|t| t.trim().parse::<u32>().ok()) else { return false };
    let _ = std::fs::remove_file(&path);
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    let args: Vec<&[u8]> = cmdline.split(|b| *b == 0).collect();
    if !(args.contains(&b"desktop".as_slice()) && args.contains(&b"serve".as_slice())) {
        return false;
    }
    std::process::Command::new("kill").arg(pid.to_string()).status().is_ok_and(|s| s.success())
}

/// The helper on Linux: agent-desktop's accessibility tools, served on the
/// helper's socket. The screenshot-and-coordinates tools aren't built for
/// Linux yet, so [`LinuxDesktop`] answers each of them with what to use instead.
#[cfg(target_os = "linux")]
pub fn serve(home: &Path) -> std::io::Result<()> {
    crate::helper::ensure_dir(home)?;
    crate::helper::ensure_token(home)?;
    let _ = std::fs::write(pid_path(home), std::process::id().to_string());
    let mut controller = crate::controller::Controller::new(LinuxDesktop);
    crate::helper::serve(
        home,
        &mut controller,
        |_| {
            let env = this_session();
            let ready = probe(&env, |p| p.exists(), &SessionBus);
            status_json(&ready, ready.session().then(|| locked(&env)).flatten().unwrap_or(false))
        },
        |_, _| {},
    )
}

/// The helper's `_status` on Linux. There is no permission to grant:
/// `accessibility` means AT-SPI answers, and `missing` says what doesn't.
pub fn status_json(ready: &Readiness, locked: bool) -> serde_json::Value {
    use serde_json::json;
    let line = |r: Result<String, &String>| match r {
        Ok(detail) => json!({ "ok": true, "detail": detail }),
        Err(why) => json!({ "ok": false, "detail": why }),
    };
    json!({
        "role": "linux",
        // Not a permission here: the picture tools aren't built for Linux yet.
        "screen": null,
        "accessibility": ready.ok(),
        "post_events": ready.ok(),
        "display": null,
        "session": { "locked": locked, "kind": ready.display.as_ref().map(Display::kind).ok() },
        "missing": ready.missing(),
        "linux": {
            "display": line(ready.display.as_ref().map(|d| match d { Display::X11(n) | Display::Wayland(n) => format!("{} {n}", d.kind()) })),
            "bus": line(ready.bus.as_ref().map(|()| String::new())),
            "atspi": line(ready.atspi.as_ref().cloned()),
            "toolkits": ready.toolkits,
        },
    })
}

/// What a member is told when it reaches for a screenshot tool on Linux.
pub const ACCESSIBILITY_ONLY: &str = "On Linux, equipe works apps through the accessibility tree only: use snapshot / find / act (and apps, windows, wait, clipboard). The screenshot, click-at-a-point and keyboard tools aren't available on Linux yet.";

/// The Linux desktop as the screenshot tools see it: not there. Every one of
/// them is refused before anything runs ([`crate::controller::Desktop::unavailable`]).
pub struct LinuxDesktop;

impl crate::controller::Desktop for LinuxDesktop {
    type Element = ();

    fn unavailable(&self, _tool: &str) -> Option<&'static str> {
        Some(ACCESSIBILITY_ONLY)
    }
    fn permissions(&self) -> crate::controller::Permissions {
        crate::controller::Permissions::default()
    }
    fn display(&self) -> Option<crate::controller::Display> {
        None
    }
    fn capture(&self, _window: Option<u32>, _width: usize, _height: usize) -> Result<Vec<u8>, String> {
        Err(ACCESSIBILITY_ONLY.to_owned())
    }
    fn apps(&self) -> Vec<crate::controller::App> {
        Vec::new()
    }
    fn open(&self, _app: &str) -> Result<(), String> {
        Err(ACCESSIBILITY_ONLY.to_owned())
    }
    fn windows(&self) -> Vec<crate::controller::Window> {
        Vec::new()
    }
    fn raise(&self, _pid: i32, _window: Option<&crate::controller::Window>) -> Result<(), String> {
        Err(ACCESSIBILITY_ONLY.to_owned())
    }
    fn focused_app(&self) -> Option<i32> {
        None
    }
    fn focused_element(&self) -> Option<()> {
        None
    }
    fn hit(&self, _at: crate::geometry::Pt) -> Option<crate::controller::Hit> {
        None
    }
    fn elements(&self, _pid: i32, _window: Option<&crate::controller::Window>, _max: usize) -> Vec<((), crate::controller::Node)> {
        Vec::new()
    }
    fn same(&self, _a: &(), _b: &()) -> bool {
        false
    }
    fn frame(&self, _element: &()) -> Option<crate::geometry::Frame> {
        None
    }
    fn value(&self, _element: &()) -> Option<String> {
        None
    }
    fn press(&self, _element: &()) -> Result<(), String> {
        Err(ACCESSIBILITY_ONLY.to_owned())
    }
    fn focus(&self, _element: &()) -> Result<(), String> {
        Err(ACCESSIBILITY_ONLY.to_owned())
    }
    fn pointer(&self, _action: crate::controller::Pointer) {}
    fn type_char(&self, _c: char) {}
    fn key(&self, _combo: crate::keys::Combo) {}
    fn pause(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    struct FakeBus {
        atspi: Result<&'static str, &'static str>,
        toolkits: Option<bool>,
    }

    impl Bus for FakeBus {
        fn atspi_address(&self, _env: &BTreeMap<String, String>) -> Result<String, String> {
            self.atspi.map(str::to_owned).map_err(str::to_owned)
        }
        fn toolkits_enabled(&self, _env: &BTreeMap<String, String>) -> Option<bool> {
            self.toolkits
        }
    }

    const ANSWERING: FakeBus = FakeBus { atspi: Ok("unix:path=/run/user/1000/at-spi/bus_0"), toolkits: Some(true) };

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    fn files(paths: &[&str]) -> impl Fn(&Path) -> bool {
        let set: BTreeSet<PathBuf> = paths.iter().map(PathBuf::from).collect();
        move |p: &Path| set.contains(p)
    }

    #[test]
    fn a_full_x11_session_with_atspi_is_ready() {
        let e = env(&[("DISPLAY", ":99"), ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/tmp/dbus-abc,guid=1")]);
        let ready = probe(&e, files(&["/tmp/.X11-unix/X99", "/tmp/dbus-abc"]), &ANSWERING);
        assert_eq!(ready.display, Ok(Display::X11(":99".into())));
        assert!(ready.ok() && ready.missing().is_empty(), "{:?}", ready.missing());
        let status = status_json(&ready, false);
        assert_eq!((status["accessibility"].as_bool(), status["screen"].as_bool(), status["role"].as_str()), (Some(true), None, Some("linux")));
        assert_eq!(status["session"]["kind"], "x11");
        assert_eq!(status["linux"]["toolkits"], true);
    }

    #[test]
    fn a_bare_ssh_shell_is_told_it_has_no_display_and_no_bus() {
        let ready = probe(&env(&[]), files(&[]), &ANSWERING);
        assert!(!ready.session() && !ready.ok());
        let missing = ready.missing();
        assert_eq!(missing.len(), 3, "{missing:?}");
        assert_eq!(missing[0], NO_DISPLAY);
        assert_eq!(missing[1], NO_BUS);
        assert!(missing[2].contains("without a session bus"));
        assert_eq!(ready.session_missing().len(), 2, "the quick look leaves AT-SPI out");
        let status = status_json(&ready, false);
        assert_eq!(status["accessibility"], false);
        assert_eq!(status["missing"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn atspi_not_answering_says_how_to_bring_it_up() {
        let e = env(&[("WAYLAND_DISPLAY", "wayland-0"), ("XDG_RUNTIME_DIR", "/run/user/1000"), ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus")]);
        let bus = FakeBus { atspi: Err("The name org.a11y.Bus was not provided by any .service files"), toolkits: None };
        let ready = probe(&e, files(&["/run/user/1000/wayland-0", "/run/user/1000/bus"]), &bus);
        assert_eq!(ready.display, Ok(Display::Wayland("wayland-0".into())));
        assert!(ready.session() && !ready.ok());
        let missing = ready.missing();
        assert_eq!(missing.len(), 1);
        assert!(missing[0].contains("at-spi2-core") && missing[0].contains("org.a11y.Bus was not provided"), "{}", missing[0]);
    }

    #[test]
    fn a_session_that_is_gone_is_told_apart_from_none() {
        let e = env(&[("DISPLAY", ":1"), ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus")]);
        let ready = probe(&e, files(&[]), &ANSWERING);
        assert!(ready.display.as_ref().unwrap_err().contains("no X server listens at /tmp/.X11-unix/X1"));
        assert!(ready.bus.as_ref().unwrap_err().contains("/run/user/1000/bus, which isn't there"));
        // A stale Wayland name with a live X display beside it still works.
        let e = env(&[("WAYLAND_DISPLAY", "wayland-9"), ("XDG_RUNTIME_DIR", "/run/user/1000"), ("DISPLAY", ":0")]);
        assert_eq!(display(&e, &files(&["/tmp/.X11-unix/X0"])), Ok(Display::X11(":0".into())));
        let e = env(&[("WAYLAND_DISPLAY", "wayland-9"), ("XDG_RUNTIME_DIR", "/run/user/1000")]);
        assert!(display(&e, &files(&[])).unwrap_err().contains("no compositor socket"));
    }

    #[test]
    fn abstract_and_tcp_addresses_are_taken_on_trust() {
        let e = env(&[("DISPLAY", "remote:10.0"), ("DBUS_SESSION_BUS_ADDRESS", "unix:abstract=/tmp/dbus-XYZ,guid=2")]);
        let ready = probe(&e, files(&[]), &ANSWERING);
        assert!(ready.ok(), "{:?}", ready.missing());
        assert_eq!(x11_socket(":0.0"), Some(PathBuf::from("/tmp/.X11-unix/X0")));
        assert_eq!(x11_socket("unix:3"), Some(PathBuf::from("/tmp/.X11-unix/X3")));
        assert_eq!(x11_socket("host:0"), None);
        assert_eq!(x11_socket("nonsense"), None);
    }

    #[test]
    fn the_daemons_missing_variables_come_from_the_user_manager() {
        let manager = "HOME=/home/p\nDISPLAY=:0\nXAUTHORITY=/run/user/1000/gdm/Xauthority\nXDG_RUNTIME_DIR=/run/user/1000\nLANG=$'en_US.UTF-8'\n";
        let own = |var: &str| (var == "XDG_SESSION_TYPE").then(|| "x11".to_owned());
        let env = session_env(own, Some(manager), files(&["/run/user/1000/bus"]));
        assert_eq!(env.get("DISPLAY").map(String::as_str), Some(":0"));
        assert_eq!(env.get("XAUTHORITY").map(String::as_str), Some("/run/user/1000/gdm/Xauthority"));
        assert_eq!(env.get("XDG_SESSION_TYPE").map(String::as_str), Some("x11"), "its own variables win");
        assert_eq!(env.get("DBUS_SESSION_BUS_ADDRESS").map(String::as_str), Some("unix:path=/run/user/1000/bus"), "systemd's per-user bus");
        assert!(!env.contains_key("HOME") && !env.contains_key("LANG"), "only the session's variables");
        assert_eq!(parse_environment("A=$'x y'\nbad line\n=v\nB=2")["A"], "x y");
        assert!(session_env(|_| None, None, files(&[])).is_empty());
    }

    #[test]
    fn every_d_bus_clients_reply_is_read() {
        let address = Some("unix:path=/run/user/1000/at-spi/bus_0,guid=abc".to_owned());
        assert_eq!(parse_address("('unix:path=/run/user/1000/at-spi/bus_0,guid=abc',)\n"), address);
        assert_eq!(parse_address("s \"unix:path=/run/user/1000/at-spi/bus_0,guid=abc\"\n"), address);
        assert_eq!(parse_address("method return time=1 sender=:1.2 -> destination=:1.3 serial=4 reply_serial=2\n   string \"unix:path=/run/user/1000/at-spi/bus_0,guid=abc\"\n"), address);
        assert_eq!(parse_address("('',)"), None);
        assert_eq!(parse_bool("(<true>,)"), Some(true));
        assert_eq!(parse_bool("v b false"), Some(false));
        assert_eq!(parse_bool("   variant       boolean true"), Some(true));
        assert_eq!(parse_bool("(true, false)"), None);
        assert_eq!(parse_number("(uint64 1234,)"), Some(1234));
        assert_eq!(parse_number("t 98765"), Some(98765));
        assert_eq!(parse_number("4321\n"), Some(4321));
        assert_eq!(parse_number("nothing"), None);
    }

    #[test]
    fn each_client_is_asked_the_same_question() {
        let commands = ATSPI_ENABLED.commands();
        let names: Vec<_> = commands.iter().map(|(p, _)| *p).collect();
        assert_eq!(names, ["gdbus", "busctl", "dbus-send"]);
        assert!(commands[0].1.ends_with(&["org.freedesktop.DBus.Properties.Get".into(), "org.a11y.Status".into(), "IsEnabled".into()]));
        assert!(commands[1].1.ends_with(&["Get".into(), "ss".into(), "org.a11y.Status".into(), "IsEnabled".into()]));
        assert!(commands[2].1.ends_with(&["string:org.a11y.Status".into(), "string:IsEnabled".into()]));
        assert!(ATSPI.commands()[1].1.ends_with(&["org.a11y.Bus".into(), "GetAddress".into()]));
    }

    #[test]
    fn the_screenshot_tools_are_refused_with_what_to_use() {
        let mut controller = crate::controller::Controller::new(LinuxDesktop);
        for tool in ["screenshot", "click", "type", "key", "elements", "open_app", "list_windows"] {
            let reply = controller.call("frontend", tool, &serde_json::json!({ "x": 1, "y": 1, "text": "hi", "app": "Files" }));
            assert!(reply.is_error && reply.text == ACCESSIBILITY_ONLY, "{tool}: {}", reply.text);
        }
        let scene = controller.scene("frontend", "click", &serde_json::json!({ "x": 1, "y": 1 }));
        assert_eq!(scene.unwrap_err().text, ACCESSIBILITY_ONLY);
        let capture = controller.capture();
        assert!(capture.is_error && capture.jpeg.is_none());
    }
}
