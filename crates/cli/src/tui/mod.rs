//! `equipe` with no command: the team console in the terminal.
//!
//! A screen on the daemon: it attaches (starting the daemon if none is running),
//! draws the team from a snapshot, turns keys into commands and draws the events
//! that come back. Leaving detaches — the agents are the daemon's children and
//! keep working. It takes the whole terminal (alternate screen) because a team
//! means several agents updating at once; on leaving, a summary is printed to the
//! normal screen so it stays in the terminal's history.

mod app;
mod commands;
mod composer;
mod markdown;
mod view;

use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags, 
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event as Input, EventStream,
    KeyEventKind,
};
use crossterm::execute;
use futures_util::StreamExt as _;

use crate::onboard::Start;
use crate::{daemon, ui};

pub struct Summary {
    pub squad: String,
    pub agents: usize,
    pub working: usize,
    pub tokens: u64,
    /// Work accepted while the screen watched: `(ticket, title, branch)`.
    pub accepted: Vec<(u32, String, String)>,
    /// Tickets still waiting for the person's review.
    pub to_review: usize,
    /// The daemon went away while this screen watched.
    pub daemon_lost: bool,
}

/// Leaves the terminal as it was found, even on an early return.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags, DisableBracketedPaste, DisableMouseCapture);
        ratatui::restore();
    }
}

/// Ask the terminal to tell modified keys apart (the kitty keyboard protocol),
/// so Shift-Enter arrives as Shift-Enter — a new line — not a bare Enter that
/// sends. Terminals without it keep working; `\` then Enter makes a new line
/// there too.
fn tell_keys_apart() {
    if crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) {
        let _ = execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
}

/// A ticket's project and branch, when it has work to look at.
fn ticket_branch(app: &app::App, cwd: &std::path::Path, id: equipe_core::protocol::TicketId) -> Option<(PathBuf, String)> {
    let ticket = app.tickets.iter().find(|t| t.id == id)?;
    let dir = ticket.project.clone().or_else(|| app.projects.first().cloned()).unwrap_or_else(|| cwd.to_owned());
    Some((dir, ticket.branch.clone()?))
}

/// What the picked ticket's branch changed, as `git diff --stat` counts it —
/// once per ticket, again whenever the ticket moves.
fn count_changes(app: &mut app::App, cwd: &std::path::Path) {
    let Some(id) = app.selected_ticket().map(|t| t.id) else {
        return;
    };
    let Some((dir, branch)) = ticket_branch(app, cwd, id) else {
        return;
    };
    if app.diffs.get(&id).is_some_and(|(b, _)| *b == branch) {
        return;
    }
    let stat = std::process::Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["diff", "--stat=200", &format!("HEAD...{branch}")])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default();
    app.diffs.insert(id, (branch, stat));
}

pub async fn run(cwd: PathBuf, start: Start) -> std::io::Result<Summary> {
    let (mut client, hello) = daemon::attach(&start.squad).await?;
    let branch = current_branch(&cwd);
    let daemon_build = hello.version.clone();
    let (mut app, first) = app::App::from_snapshot(cwd.clone(), hello.snapshot, &start.members);
    app.squad_width = saved_squad_width();
    first.into_iter().for_each(|c| client.send(c));
    if daemon_build != daemon::build_id() {
        // A daemon from an older build keeps running old code; say so, but don't
        // restart it under agents that may be mid-turn.
        app.flash = Some("The daemon runs an older build — `equipe daemon restart` when its agents are idle.".into());
    }

    // ratatui::init enters the alternate screen and raw mode, and restores them
    // if we panic.
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    // In raw mode, before anything else reads the input: ask the terminal its
    // background colour, so the person's messages can sit on a faint wash of it.
    let theme = view::Theme::detect(query_background());
    tell_keys_apart();
    if app.mouse && first_time("mouse-select") {
        app.flash = Some("The wheel scrolls here. To select text, hold Shift (Option in iTerm2) while dragging — or /mouse.".into());
    }
    // Pastes arrive whole, so a newline inside one never sends the message.
    execute!(std::io::stdout(), EnableBracketedPaste)?;
    // The wheel scrolls; `/mouse` hands the mouse back so text can be selected.
    let mut mouse = app.mouse;
    if mouse {
        execute!(std::io::stdout(), EnableMouseCapture)?;
    }

    let mut input = EventStream::new();
    let mut saved_width = app.squad_width;
    let mut last_sample = Instant::now();
    let mut daemon_lost = false;
    let mut layouts = view::Layouts::default();
    let draw = |terminal: &mut ratatui::DefaultTerminal, app: &mut app::App, layouts: &mut view::Layouts| {
        let size = terminal.size()?;
        if let Some((id, lines)) = layouts.prepare(app, theme, size.width) {
            app.grew(id, lines);
        }
        let extent = view::extent(app, theme, ratatui::layout::Rect::new(0, 0, size.width, size.height), layouts);
        app.set_extent(extent.page, extent.conversation_max, extent.board_max);
        app.set_hits(extent.hits, extent.divider);
        terminal.draw(|f| view::render(f, app, theme, branch.as_deref(), layouts)).map(|_| ())
    };
    draw(&mut terminal, &mut app, &mut layouts)?;
    // While someone works, its dot breathes and its clock ticks; with nobody
    // working, nothing redraws on a timer.
    let mut clock = tokio::time::interval(Duration::from_millis(60));
    loop {
        tokio::select! {
            _ = clock.tick() => {
                if app.working() == 0 {
                    continue;
                }
            }
            Some(Ok(event)) = input.next() => {
                if !on_input(&mut app, &client, event) {
                    continue;
                }
                // A trackpad sends a burst of events per gesture: take the ones
                // already here, then draw once. (A zero timeout polls with this
                // task's own waker, so the stream keeps waking it afterwards — a
                // no-op waker would leave later input unread until something else
                // woke the loop.)
                while let Ok(Some(Ok(event))) = tokio::time::timeout(Duration::ZERO, input.next()).await {
                    on_input(&mut app, &client, event);
                }
                // A width the person dragged to is kept for next time.
                if !app.is_dragging() && app.squad_width != saved_width {
                    saved_width = app.squad_width;
                    save_squad_width(saved_width);
                }
            }
            event = client.next_event() => match event {
                Some(event) => {
                    for command in app.on_event(event) {
                        client.send(command);
                    }
                    // Streaming sends many small events; draw once per burst.
                    while let Some(event) = client.try_event() {
                        for command in app.on_event(event) {
                            client.send(command);
                        }
                    }
                }
                None => {
                    daemon_lost = true;
                    break;
                }
            },
        }
        if app.should_quit {
            break;
        }
        if std::mem::take(&mut app.export) {
            match export(&app) {
                Ok((path, editor)) => {
                    if let Some(editor) = editor {
                        // Hand the terminal to the editor, then take it back.
                        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags, DisableBracketedPaste, DisableMouseCapture);
                        ratatui::restore();
                        let _ = std::process::Command::new("sh").arg("-c").arg(format!("{editor} \"$1\"")).arg("sh").arg(&path).status();
                        terminal = ratatui::init();
                        tell_keys_apart();
                        execute!(std::io::stdout(), EnableBracketedPaste)?;
                        if mouse {
                            execute!(std::io::stdout(), EnableMouseCapture)?;
                        }
                        terminal.clear()?;
                    }
                    app.flash = Some(format!("Saved to {}", path.display()));
                }
                Err(err) => app.flash = Some(format!("Couldn't export: {err}")),
            }
        }
        if let Some(id) = app.view_diff.take() {
            // The whole diff in the person's pager, then back.
            if let Some((dir, branch)) = ticket_branch(&app, &cwd, id) {
                let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags, DisableBracketedPaste, DisableMouseCapture);
                ratatui::restore();
                let _ = std::process::Command::new("sh")
                    .arg("-c")
                    .arg("git -C \"$1\" diff --color=always \"HEAD...$2\" | ${PAGER:-less -R}")
                    .arg("sh")
                    .arg(&dir)
                    .arg(&branch)
                    .status();
                terminal = ratatui::init();
                tell_keys_apart();
                execute!(std::io::stdout(), EnableBracketedPaste)?;
                if mouse {
                    execute!(std::io::stdout(), EnableMouseCapture)?;
                }
                terminal.clear()?;
            }
        }
        if std::mem::take(&mut app.open_browser) {
            // `equipe browser` in the terminal, then back: the person signs in there.
            let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags, DisableBracketedPaste, DisableMouseCapture);
            ratatui::restore();
            let signed_in = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg("browser").status());
            terminal = ratatui::init();
            tell_keys_apart();
            execute!(std::io::stdout(), EnableBracketedPaste)?;
            if mouse {
                execute!(std::io::stdout(), EnableMouseCapture)?;
            }
            terminal.clear()?;
            app.flash = Some(match signed_in {
                Ok(status) if status.success() => "Signed in. Members use the browser from their next turn.".into(),
                _ => "The browser didn't open — run `equipe browser` in a terminal to see why.".into(),
            });
        }
        if app.pane == app::Pane::Board {
            count_changes(&mut app, &cwd);
        }
        // Members' browsers and the memory they take, every few seconds while
        // anyone works or a browser is open.
        let busy = app.agents.iter().any(|a| a.state == app::AgentState::Working) || !app.browsers.is_empty();
        if busy && last_sample.elapsed() >= Duration::from_secs(3) {
            last_sample = Instant::now();
            if let Ok(out) = std::process::Command::new("ps").args(["-axo", "pid=,ppid=,rss=,command="]).output() {
                app.browsers = app::browser_memory(&String::from_utf8_lossy(&out.stdout), &app.squad);
            }
        }
        if app.mouse != mouse {
            mouse = app.mouse;
            if mouse {
                execute!(std::io::stdout(), EnableMouseCapture)?;
            } else {
                execute!(std::io::stdout(), DisableMouseCapture)?;
            }
        }
        // Drawn only after something changed — nothing redraws on a timer.
        draw(&mut terminal, &mut app, &mut layouts)?;
    }

    Ok(Summary {
        squad: app.squad.clone(),
        agents: app.agents.iter().filter(|a| a.is_open()).count(),
        working: app.working(),
        tokens: app.total_tokens(),
        accepted: app.accepted.clone(),
        to_review: app.to_review().len(),
        daemon_lost,
    })
}

/// Write the selected member's conversation to a Markdown file under
/// `~/.equipe/exports`, and name the editor to open it in, if one is set.
fn export(app: &app::App) -> std::io::Result<(PathBuf, Option<String>)> {
    let agent = app.selected().ok_or_else(|| std::io::Error::other("nobody is selected"))?;
    let dir = equipe_providers::credentials::equipe_home().join("exports");
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = dir.join(format!("{}-{}-{stamp}.md", app.squad, agent.label()));
    std::fs::write(&path, app::transcript_markdown(agent, &app.squad))?;
    let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).ok().filter(|e| !e.trim().is_empty());
    Ok((path, editor))
}

/// Whether this is the first time the hint `name` is shown on this machine — and
/// remember that it was.
/// The squad pane's width as the person last dragged it, kept beside the hints.
fn squad_width_file() -> PathBuf {
    equipe_providers::credentials::equipe_home().join("hints").join("squad-width")
}

fn saved_squad_width() -> Option<u16> {
    std::fs::read_to_string(squad_width_file()).ok()?.trim().parse().ok()
}

fn save_squad_width(width: Option<u16>) {
    let file = squad_width_file();
    if let Some(width) = width {
        let _ = std::fs::create_dir_all(file.parent().expect("hints has a parent"));
        let _ = std::fs::write(&file, width.to_string());
    }
}

fn first_time(name: &str) -> bool {
    let marker = equipe_providers::credentials::equipe_home().join("hints").join(name);
    if marker.exists() {
        return false;
    }
    let _ = std::fs::create_dir_all(marker.parent().expect("hints has a parent"));
    std::fs::write(&marker, b"").is_ok()
}

/// Ask the terminal for its background colour (OSC 11) and wait a moment for the
/// answer. Raw mode must be on, and nothing else reading the input yet.
fn query_background() -> Option<(u8, u8, u8)> {
    use std::io::Write as _;
    use std::os::fd::AsRawFd as _;
    let mut out = std::io::stdout();
    out.write_all(b"\x1b]11;?\x1b\\").ok()?;
    out.flush().ok()?;
    let fd = std::io::stdin().as_raw_fd();
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut answer = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let mut ready = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd, and a buffer we own for the read.
        if unsafe { libc::poll(&mut ready, 1, left.as_millis() as libc::c_int) } <= 0 {
            break;
        }
        let mut chunk = [0u8; 64];
        let n = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if n <= 0 {
            break;
        }
        answer.extend_from_slice(&chunk[..n as usize]);
        if answer.ends_with(b"\x07") || answer.ends_with(b"\x1b\\") {
            break;
        }
    }
    parse_background(&String::from_utf8_lossy(&answer))
}

/// `…]11;rgb:1e1e/1e1e/2020…` → (30, 30, 32): channels of one to four hex digits.
fn parse_background(answer: &str) -> Option<(u8, u8, u8)> {
    let rgb = answer.split("rgb:").nth(1)?;
    let mut parts = rgb.split(['/', '\x07', '\x1b']);
    let mut channel = || {
        let hex = parts.next()?;
        let value = u32::from_str_radix(hex, 16).ok()?;
        let max = (1u32 << (4 * hex.len().clamp(1, 4))) - 1;
        Some((value * 255 / max) as u8)
    };
    Some((channel()?, channel()?, channel()?))
}

/// Apply one terminal event. Returns whether it could have changed the screen.
fn on_input(app: &mut app::App, client: &equipe_core::gateway::Client, event: Input) -> bool {
    match event {
        Input::Key(key) if key.kind != KeyEventKind::Release => {
            for command in app.on_key(key, Instant::now()) {
                client.send(command);
            }
        }
        Input::Paste(text) => app.on_paste(&text),
        Input::Mouse(event) => match event.kind {
            crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) => app.on_click(event.column, event.row),
            crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left) => app.on_drag(event.column),
            kind => app.on_mouse(kind, Instant::now()),
        },
        Input::Resize(..) => {}
        _ => return false,
    }
    true
}

/// What happened, printed to the normal screen so it stays in the terminal's history.
pub fn print_summary(summary: &Summary) {
    let p = ui::out();
    let team = match summary.agents {
        0 => "nobody on the squad".to_owned(),
        1 => "1 member".to_owned(),
        n => format!("{n} members"),
    };
    println!(
        "{}  {}",
        p.wordmark(),
        p.dim(&format!("detached from {} · {team} · {} tokens", summary.squad, summary.tokens))
    );
    if summary.daemon_lost {
        println!("{}", p.fail_line("The daemon stopped while you watched."));
        println!("  {}", p.hint("equipe daemon start"));
    } else if summary.working > 0 {
        let who = if summary.working == 1 { "1 member is".to_owned() } else { format!("{} members are", summary.working) };
        println!("{}", p.ok_line(&format!("{who} still working. The squad keeps going without this screen.")));
    }
    if summary.to_review > 0 {
        let what = if summary.to_review == 1 { "1 ticket waits".to_owned() } else { format!("{} tickets wait", summary.to_review) };
        println!("{}", p.note(2, &format!("{what} for your review.")));
    }
    for (ticket, title, branch) in &summary.accepted {
        println!("\n{}", p.ok_line(&format!("T-{ticket} {title} — accepted, on {}", p.bold(branch))));
        println!("{}", p.row("review", &p.bold(&format!("git log -p main..{branch}"))));
        println!("{}", p.row("merge", &p.bold(&format!("git merge {branch}"))));
    }
    if !summary.daemon_lost {
        println!("\n  {}", p.hint("equipe   to come back"));
    }
    let _ = std::io::stdout().flush();
}

fn current_branch(cwd: &std::path::Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|b| !b.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_background_answer_reads_in_any_precision() {
        assert_eq!(parse_background("\x1b]11;rgb:1e1e/1e1e/2020\x1b\\"), Some((30, 30, 32)));
        assert_eq!(parse_background("\x1b]11;rgb:ff/ff/ff\x07"), Some((255, 255, 255)));
        assert_eq!(parse_background(""), None);
    }
}
