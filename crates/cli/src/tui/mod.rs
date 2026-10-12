//! `equipe` with no command: the team console in the terminal.
//!
//! A screen on the daemon: it attaches (starting the daemon if none is running),
//! draws the team from a snapshot, turns keys into commands and draws the events
//! that come back. Leaving detaches — the agents are the daemon's children and
//! keep working. It takes the whole terminal (alternate screen) because a team
//! means several agents updating at once; on leaving, a summary is printed to the
//! normal screen so it stays in the terminal's history.

mod app;
mod composer;
mod markdown;
mod view;

use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{
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
        let _ = execute!(std::io::stdout(), DisableBracketedPaste, DisableMouseCapture);
        ratatui::restore();
    }
}

pub async fn run(cwd: PathBuf, start: Start) -> std::io::Result<Summary> {
    let (mut client, hello) = daemon::attach(&start.squad).await?;
    let branch = current_branch(&cwd);
    let daemon_build = hello.version.clone();
    let (mut app, first) = app::App::from_snapshot(cwd, hello.snapshot, &start.members);
    first.into_iter().for_each(|c| client.send(c));
    if daemon_build != daemon::build_id() {
        // A daemon from an older build keeps running old code; say so, but don't
        // restart it under agents that may be mid-turn.
        app.flash = Some("The daemon runs an older build — `equipe daemon restart` when its agents are idle.".into());
    }

    let theme = view::Theme::detect();
    // ratatui::init enters the alternate screen and raw mode, and restores them
    // if we panic.
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    // Pastes arrive whole, so a newline inside one never sends the message.
    execute!(std::io::stdout(), EnableBracketedPaste)?;
    // The wheel scrolls; `/mouse` hands the mouse back so text can be selected.
    let mut mouse = app.mouse;
    if mouse {
        execute!(std::io::stdout(), EnableMouseCapture)?;
    }

    let mut input = EventStream::new();
    let mut daemon_lost = false;
    let mut layouts = view::Layouts::default();
    let draw = |terminal: &mut ratatui::DefaultTerminal, app: &mut app::App, layouts: &mut view::Layouts| {
        let width = terminal.size()?.width;
        if let Some((id, lines)) = layouts.prepare(app, theme, width) {
            app.grew(id, lines);
        }
        terminal.draw(|f| view::render(f, app, theme, branch.as_deref(), layouts)).map(|_| ())
    };
    draw(&mut terminal, &mut app, &mut layouts)?;
    // A working member's clock ticks; with nobody working, nothing redraws on a timer.
    let mut clock = tokio::time::interval(Duration::from_secs(1));
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

/// Apply one terminal event. Returns whether it could have changed the screen.
fn on_input(app: &mut app::App, client: &equipe_core::gateway::Client, event: Input) -> bool {
    match event {
        Input::Key(key) if key.kind != KeyEventKind::Release => {
            for command in app.on_key(key, Instant::now()) {
                client.send(command);
            }
        }
        Input::Paste(text) => app.on_paste(&text),
        Input::Mouse(event) => app.on_mouse(event.kind),
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
