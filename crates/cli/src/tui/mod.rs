//! `equipe` with no command: the team console in the terminal.
//!
//! A screen like any other: it starts the core runtime, turns keys into
//! commands, and draws the events that come back. It takes the whole terminal
//! (alternate screen) because a team means several agents updating at once;
//! what happened is printed to the normal screen when it closes.

mod app;
mod composer;
mod view;

use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste, Event as Input, EventStream, KeyEventKind};
use crossterm::execute;
use equipe_core::runtime;
use futures_util::StreamExt as _;

use crate::ui;

/// How long quitting waits for agents to land their branches.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(20);

pub struct Summary {
    pub agents: usize,
    pub tokens: u64,
    pub landed: Vec<(String, String)>,
}

/// Leaves the terminal as it was found, even on an early return.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(std::io::stdout(), DisableBracketedPaste);
        ratatui::restore();
    }
}

pub async fn run(cwd: PathBuf, version: &str) -> std::io::Result<Summary> {
    let branch = current_branch(&cwd);
    let (core, mut events) = runtime::start(runtime::Config::for_user(version));
    let (mut app, first) = app::App::new(cwd);
    first.into_iter().for_each(|c| core.send(c));

    let theme = view::Theme::detect();
    // ratatui::init enters the alternate screen and raw mode, and restores them
    // if we panic.
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    // Pastes arrive whole, so a newline inside one never sends the message.
    execute!(std::io::stdout(), EnableBracketedPaste)?;

    let mut input = EventStream::new();
    terminal.draw(|f| view::render(f, &app, theme, branch.as_deref()))?;
    loop {
        tokio::select! {
            Some(Ok(event)) = input.next() => match event {
                Input::Key(key) if key.kind != KeyEventKind::Release => {
                    for command in app.on_key(key, Instant::now()) {
                        core.send(command);
                    }
                }
                Input::Paste(text) => app.on_paste(&text),
                Input::Resize(..) => {}
                _ => continue,
            },
            Some(event) = events.recv() => {
                for command in app.on_event(event) {
                    core.send(command);
                }
                // Streaming sends many small events; draw once per burst.
                while let Ok(event) = events.try_recv() {
                    for command in app.on_event(event) {
                        core.send(command);
                    }
                }
            }
        }
        if app.should_quit {
            break;
        }
        // Drawn only after something changed — nothing redraws on a timer.
        terminal.draw(|f| view::render(f, &app, theme, branch.as_deref()))?;
    }

    // Land every editing agent's branch before leaving.
    for command in app.closing_commands() {
        core.send(command);
    }
    terminal.draw(|f| view::render(f, &app, theme, branch.as_deref()))?;
    let closing = async {
        while !app.all_closed() {
            match events.recv().await {
                Some(event) => {
                    app.on_event(event);
                }
                None => break,
            }
        }
    };
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, closing).await;

    Ok(Summary {
        agents: app.agents.len(),
        tokens: app.total_tokens(),
        landed: app.landed.clone(),
    })
}

/// What happened, printed to the normal screen so it stays in the terminal's history.
pub fn print_summary(summary: &Summary) {
    let p = ui::out();
    let agents = if summary.agents == 1 { "1 agent".to_owned() } else { format!("{} agents", summary.agents) };
    println!("{}  {}", p.wordmark(), p.dim(&format!("session ended · {agents} · {} tokens", summary.tokens)));
    for (branch, stat) in &summary.landed {
        println!("\n{}", p.ok_line(&format!("Changes committed to {}", p.bold(branch))));
        if let Some(last) = stat.lines().last() {
            println!("  {}", p.dim(last.trim()));
        }
        println!("{}", p.row("review", &p.bold(&format!("git show {branch}"))));
        println!("{}", p.row("keep", &p.bold(&format!("git merge {branch}"))));
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
