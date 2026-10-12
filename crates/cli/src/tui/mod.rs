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
mod view;

use std::io::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste, Event as Input, EventStream, KeyEventKind};
use crossterm::execute;
use equipe_core::runtime;
use futures_util::StreamExt as _;

use crate::{daemon, ui};

pub struct Summary {
    pub agents: usize,
    pub working: usize,
    pub tokens: u64,
    pub landed: Vec<(String, String)>,
    /// The daemon went away while this screen watched.
    pub daemon_lost: bool,
}

/// Leaves the terminal as it was found, even on an early return.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(std::io::stdout(), DisableBracketedPaste);
        ratatui::restore();
    }
}

pub async fn run(cwd: PathBuf) -> std::io::Result<Summary> {
    let project = runtime::project_of(&cwd);
    let (mut client, hello) = daemon::attach(&project).await?;
    let branch = current_branch(&cwd);
    let daemon_build = hello.version.clone();
    let (mut app, first) = app::App::from_snapshot(cwd, project, hello.snapshot);
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

    let mut input = EventStream::new();
    let mut daemon_lost = false;
    terminal.draw(|f| view::render(f, &app, theme, branch.as_deref()))?;
    loop {
        tokio::select! {
            Some(Ok(event)) = input.next() => match event {
                Input::Key(key) if key.kind != KeyEventKind::Release => {
                    for command in app.on_key(key, Instant::now()) {
                        client.send(command);
                    }
                }
                Input::Paste(text) => app.on_paste(&text),
                Input::Resize(..) => {}
                _ => continue,
            },
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
        // Drawn only after something changed — nothing redraws on a timer.
        terminal.draw(|f| view::render(f, &app, theme, branch.as_deref()))?;
    }

    Ok(Summary {
        agents: app.agents.iter().filter(|a| a.is_open()).count(),
        working: app.working(),
        tokens: app.total_tokens(),
        landed: app.landed.clone(),
        daemon_lost,
    })
}

/// What happened, printed to the normal screen so it stays in the terminal's history.
pub fn print_summary(summary: &Summary) {
    let p = ui::out();
    let team = match summary.agents {
        0 => "no agents open".to_owned(),
        1 => "1 agent".to_owned(),
        n => format!("{n} agents"),
    };
    println!("{}  {}", p.wordmark(), p.dim(&format!("detached · {team} · {} tokens", summary.tokens)));
    if summary.daemon_lost {
        println!("{}", p.fail_line("The daemon stopped while you watched."));
        println!("  {}", p.hint("equipe daemon start"));
    } else if summary.working > 0 {
        let who = if summary.working == 1 { "1 agent is".to_owned() } else { format!("{} agents are", summary.working) };
        println!("{}", p.ok_line(&format!("{who} still working. The team keeps going without this screen.")));
    }
    for (branch, stat) in &summary.landed {
        println!("\n{}", p.ok_line(&format!("Changes committed to {}", p.bold(branch))));
        if let Some(last) = stat.lines().last() {
            println!("  {}", p.dim(last.trim()));
        }
        println!("{}", p.row("review", &p.bold(&format!("git show {branch}"))));
        println!("{}", p.row("keep", &p.bold(&format!("git merge {branch}"))));
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
