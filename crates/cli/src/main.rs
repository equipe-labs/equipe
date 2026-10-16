//! `equipe` — the command-line entry point.
//!
//! Two engines, both on the user's own plans:
//! - **codex**: the user's installed Codex CLI, which signs itself in
//!   (`codex login`); without it, equipe's own ChatGPT sign-in, which only answers
//!   (see `equipe_providers::codex_app`).
//! - **claude**: the user's installed Claude Code, which signs itself in. equipe
//!   never sees Claude credentials (see `equipe_providers::claude_code`).

mod attach;
mod clip;
mod daemon;
mod desktop;
mod disk;
mod home;
mod i18n;
mod lang;
mod onboard;
mod remote;
mod settings;
mod telegram;
mod tui;
mod ui;

use std::io::{IsTerminal as _, Write as _};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::{CommandFactory as _, Parser, Subcommand, ValueEnum};
use equipe_core::protocol::{self, Command as Cmd, Event, SessionSpec, TurnOutcome, Usage};
use equipe_core::roles::Tier;
use equipe_core::runtime;
use equipe_core::t;
use equipe_providers::claude_code::{self, AuthStatus};
use equipe_providers::codex_app;
use equipe_providers::openai::{codex, login};
use equipe_providers::{AuthMethod, CredentialStore, Credentials};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name = "equipe", version, about = "A full software squad of AI agents.")]
struct Cli {
    /// With no command: the team console, at a terminal.
    #[command(subcommand)]
    command: Option<Command>,
    /// With no command: open this squad, wherever equipe is run
    #[arg(long)]
    squad: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Engine {
    /// Your installed Codex CLI on your ChatGPT plan, signed in by Codex itself
    Codex,
    /// Your installed Claude Code, signed in by Claude Code itself
    Claude,
}

impl From<Engine> for protocol::Engine {
    fn from(engine: Engine) -> Self {
        match engine {
            Engine::Codex => protocol::Engine::Codex,
            Engine::Claude => protocol::Engine::Claude,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Sign in to an engine (asks which at a terminal)
    Login {
        #[arg(value_enum)]
        engine: Option<Engine>,
        /// Minutes to wait for the browser (codex)
        #[arg(long, default_value_t = 10)]
        timeout: u64,
        /// Print the sign-in link instead of opening a browser (codex)
        #[arg(long)]
        no_browser: bool,
        /// Sign in again even when already signed in
        #[arg(long)]
        again: bool,
    },
    /// Forget equipe's Codex login (Claude Code manages its own)
    Logout,
    /// Show which engines are ready
    Status,
    /// Run one turn
    Try {
        prompt: String,
        /// Engine to use (default: the first one that is ready)
        #[arg(long, value_enum)]
        engine: Option<Engine>,
        /// Model id or alias (default: the engine's own default)
        #[arg(long)]
        model: Option<String>,
        /// Let the agent edit files — on a new branch, in an isolated worktree
        #[arg(long)]
        edit: bool,
    },
    /// List the built-in squad roles
    Roles,
    /// Your squads and the projects each works on
    Squads,
    /// Open the squad's folder; `changes` lists what changed there, `undo [n]` takes a change back
    Home {
        /// `changes`, or `undo [n]`
        words: Vec<String>,
        /// The squad (default: the one this folder belongs to, else your first)
        #[arg(long)]
        squad: Option<String>,
    },
    /// What the squad remembers about you and the work; `changes` lists what changed there lately
    Memory {
        /// `changes`
        words: Vec<String>,
        /// The squad (default: the one this folder belongs to, else your first)
        #[arg(long)]
        squad: Option<String>,
    },
    /// Start a squad, or change the folders one works on: `new <name> [folder]`, `add <folder>`, `remove <project>`
    Squad {
        words: Vec<String>,
        /// The squad to change (default: the one this folder belongs to, else your first)
        #[arg(long)]
        squad: Option<String>,
        /// Only check the folder and say what would happen
        #[arg(long)]
        dry_run: bool,
    },
    /// Apps where a member's send waits for your yes: `add|remove <app>`, `reset`
    SendApps {
        words: Vec<String>,
        /// The squad (default: the one this folder belongs to, else your first)
        #[arg(long)]
        squad: Option<String>,
    },
    /// Serve a member's board tools over MCP (equipe starts this for its engines)
    #[command(hide = true)]
    Mcp {
        /// `equipe` (the board) or `desktop`
        #[arg(long, default_value = "equipe")]
        server: String,
        #[arg(long)]
        squad: String,
        #[arg(long)]
        member: String,
        #[arg(long)]
        role: String,
        #[arg(long)]
        home: std::path::PathBuf,
    },
    /// Sign in to sites in a browser, for the squad to work in as you
    Browser {
        /// Where to start (default: a page of sites squads often need)
        url: Option<String>,
        /// Forget every sign-in: members have no browser until you sign in again
        #[arg(long)]
        forget: bool,
        /// How many browsers may be open at once (no number: show it)
        #[arg(long, num_args = 0..=1)]
        max: Option<Option<String>>,
    },
    /// Members working this Mac's desktop apps (experimental, macOS)
    Desktop {
        #[command(subcommand)]
        action: Option<DesktopAction>,
    },
    /// Send this Mac's clipboard image to the equipe console on another machine
    SendImage {
        /// The machine, as ssh knows it (`abel-macmini`, `me@host`)
        host: String,
    },
    /// On the Mac you type on: make ⌃⇧V in iTerm2 paste the clipboard's
    /// screenshot into equipe, here or over SSH
    PasteSetup {
        /// Give ⌃⇧V back to iTerm2
        #[arg(long)]
        remove: bool,
    },
    /// Pair a phone running the equipe app with this machine
    Pair,
    /// The phones paired with this machine
    Devices {
        #[command(subcommand)]
        action: Option<DevicesAction>,
    },
    /// A demo daemon for app store reviewers: a scripted squad and a code that never expires
    #[command(hide = true)]
    Demo {
        #[command(subcommand)]
        action: DemoAction,
    },
    /// Your squads from Telegram: requests, reviews and replies on your phone
    Telegram {
        #[command(subcommand)]
        action: Option<TelegramAction>,
    },
    /// Connect a screen on another machine to the daemon here, over SSH:
    /// `ssh <host> equipe attach --stdio`
    Attach {
        /// Carry the daemon's protocol on standard input and output
        #[arg(long)]
        stdio: bool,
        /// Fail when the daemon isn't running instead of starting it
        #[arg(long)]
        no_start: bool,
    },
    /// Keep a file a screen on another machine streams on standard input
    /// (`ssh <host> equipe receive …`): prints where it was kept
    #[command(hide = true)]
    Receive {
        /// The file's name where it came from (only a tidied stem and its extension are kept)
        #[arg(long)]
        name: String,
        /// Exactly how many bytes follow
        #[arg(long)]
        size: u64,
        /// image, video, pdf or file
        #[arg(long, default_value = "file")]
        kind: String,
    },
    /// Update equipe to the latest release
    Update,
    /// Show the language equipe speaks to you, or pick one for every screen:
    /// ko, en, ja, zh-Hans, zh-Hant, es, fr, de — or auto, the system's
    Lang {
        /// A language's code, or `auto`
        lang: Option<String>,
    },
    /// How much room the disk has, what uses it, and what to do about it:
    /// `clean` gives back what equipe may remove, after your yes; `debug
    /// lines|off|full` sets how lean members' Rust builds are
    Disk {
        /// `clean`, or `debug [lines|off|full]`
        words: Vec<String>,
        /// The squad, for `debug` (default: the one this folder belongs to, else your first)
        #[arg(long)]
        squad: Option<String>,
        /// Clean without asking
        #[arg(long, short)]
        yes: bool,
    },
    /// The background daemon the team lives in
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
}

#[derive(Subcommand)]
enum DevicesAction {
    /// Forget a device: it can't reach this machine any more
    Remove {
        /// Its name, as `equipe devices` lists it
        name: String,
    },
}

#[derive(Subcommand)]
enum DemoAction {
    /// Run it in the foreground: its own relay registration and squad, no agent
    Serve {
        /// Where it keeps its registration, code and phones (default: $EQUIPE_HOME/demo)
        #[arg(long)]
        home: Option<std::path::PathBuf>,
    },
    /// Show its pairing code: the QR and the address
    Code {
        #[arg(long)]
        home: Option<std::path::PathBuf>,
    },
    /// Forget every demo phone and start the script over
    Reset {
        /// Also make a new code; the old one stops working
        #[arg(long)]
        new_code: bool,
        #[arg(long)]
        home: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum DesktopAction {
    /// Install the desktop helper and check what it needs: permissions, the GUI session
    Setup {
        /// Sign the helper with this identity ("Developer ID Application: …"), or `self`
        /// for a certificate made on this Mac (the default)
        #[arg(long)]
        sign: Option<String>,
    },
    /// What the desktop helper has and lacks, changing nothing
    Status,
    /// Let a member work the desktop
    On {
        member: String,
        /// Desktop control is experimental: say so
        #[arg(long)]
        experimental: bool,
        #[arg(long)]
        squad: Option<String>,
    },
    /// Take the desktop back from a member
    Off {
        member: String,
        #[arg(long)]
        squad: Option<String>,
    },
    /// Stop the helper and remove it
    Remove,
    /// Run the helper (what its LaunchAgent starts)
    #[command(hide = true)]
    Serve {
        #[arg(long)]
        home: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
enum TelegramAction {
    /// Connect a bot of your own, paired with your Telegram account
    Setup,
    /// Disconnect it
    Off,
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Start it in the background (equipe does this on its own when needed)
    Start,
    /// Stop it. Open sessions resume when it starts again
    Stop,
    /// Stop it and start this build
    Restart {
        /// Wait until no member is mid-turn and no request waits for you
        #[arg(long)]
        when_idle: bool,
    },
    /// Whether it is running
    Status,
    /// Run it as a login service: up after a restart, back after a crash
    Install,
    /// Stop running it as a login service
    Uninstall,
    /// Serve in the foreground (what `start` runs)
    #[command(hide = true)]
    Run,
}

/// TLS for the relay's WebSocket (`equipe daemon`, `pair`, `demo serve`):
/// rustls picks its crypto from the process when it isn't told, and a build
/// with both ring and aws-lc-rs in it (the workspace's, with the desktop
/// app's) can't choose — it panicked on the first `wss://` connect. Say it
/// once, before anything opens a connection. reqwest names its own.
fn install_tls() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

#[tokio::main]
async fn main() -> ExitCode {
    install_tls();
    let cli = Cli::parse();
    // The helper's copy of equipe holds the person's desktop permissions: it
    // runs the helper and nothing else.
    if desktop::is_helper_copy() && !matches!(cli.command, Some(Command::Desktop { action: Some(DesktopAction::Serve { .. }) })) {
        eprintln!("This is equipe's desktop helper; it only runs as `desktop serve`.");
        return ExitCode::FAILURE;
    }
    let app = App {
        store: CredentialStore::default_location(),
        client: reqwest::Client::new(),
    };

    let Some(command) = cli.command else {
        // The team console needs a person at a terminal; scripts get the help.
        if !(ui::interactive() && std::io::stdout().is_terminal()) {
            let _ = Cli::command().print_help();
            return ExitCode::SUCCESS;
        }
        let cwd = match std::env::current_dir() {
            Ok(cwd) => cwd,
            Err(err) => {
                eprintln!("{}", ui::err().fail_line(&err.to_string()));
                return ExitCode::FAILURE;
            }
        };
        let start = match onboard::squad_for(&cwd, cli.squad.clone()).await {
            Ok(start) => start,
            Err(err) => {
                eprintln!("{}", ui::err().fail_line(&err));
                let hint = if err == t!("onboard.no_engine") { "equipe login" } else { "equipe squads" };
                eprintln!("  {}", ui::err().hint(hint));
                return ExitCode::FAILURE;
            }
        };
        return match tui::run(cwd, start).await {
            Ok(summary) => {
                tui::print_summary(&summary);
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("{}", ui::err().fail_line(&t!("main.tui_stopped", err = err)));
                ExitCode::FAILURE
            }
        };
    };

    let outcome = match command {
        Command::Login { engine, timeout, no_browser, again } => app.login(engine, timeout, no_browser, again).await,
        Command::Logout => app.logout(),
        Command::Status => app.status().await,
        Command::Try { prompt, engine, model, edit } => app.run_try(&prompt, engine, model.as_deref(), edit).await,
        Command::Daemon { action } => daemon_command(action).await,
        Command::Disk { words, squad, yes } => disk::command(&words, squad, yes).await.map_err(Failure::new),
        Command::Update => update().await,
        Command::Lang { lang } => match lang::command(&equipe_providers::credentials::equipe_home(), lang.as_deref()) {
            Ok(lines) => {
                for line in lines {
                    println!("{line}");
                }
                Ok(())
            }
            Err(why) => Err(Failure::new(why)),
        },
        Command::Attach { stdio: true, no_start } => attach::stdio(no_start).await,
        Command::Attach { stdio: false, .. } => return attach::usage(),
        Command::Receive { name, size, kind } => {
            use equipe_core::protocol::rpc::UploadKind as K;
            let kind = match kind.as_str() {
                "image" => K::Image,
                "video" => K::Video,
                "pdf" => K::Pdf,
                _ => K::File,
            };
            let home = equipe_providers::credentials::equipe_home();
            match equipe_core::gateway::receive_upload(&home, &name, Some(kind), size, std::io::stdin().lock()) {
                Ok(path) => {
                    println!("{}", path.display());
                    Ok(())
                }
                Err(why) => Err(Failure::new(why)),
            }
        }
        Command::SendImage { host } => match clip::send_image(&host) {
            Ok(()) => {
                println!("{}", ui::out().ok_line(&t!("main.image_sent", host = host)));
                Ok(())
            }
            Err(why) => Err(Failure::new(why)),
        },
        Command::PasteSetup { remove } => {
            let ran = std::process::Command::new("sh").arg("-c").arg(clip::PASTE_SETUP).arg("sh").args(remove.then_some("--remove")).status();
            match ran {
                Ok(status) if status.success() => Ok(()),
                Ok(_) => Err(Failure::new(t!("main.paste_setup_unfinished"))),
                Err(err) => Err(Failure::new(t!("main.sh_failed", err = err))),
            }
        }
        Command::Pair | Command::Devices { .. } => {
            let said = match command {
                Command::Pair => remote::cli::pair().await,
                Command::Devices { action: Some(DevicesAction::Remove { name }) } => remote::cli::remove(&name).await,
                _ => remote::cli::list().await,
            };
            match said {
                Ok(text) => {
                    println!("{}", ui::out().ok_line(&text));
                    Ok(())
                }
                Err(why) => Err(Failure::new(why)),
            }
        }
        Command::Demo { action } => {
            let said = match action {
                DemoAction::Serve { home } => remote::link::serve_demo(remote::demo::home(home), VERSION.to_owned()).await.map(|()| String::new()),
                DemoAction::Code { home } => remote::cli::demo_code(&remote::demo::home(home)),
                DemoAction::Reset { new_code, home } => remote::cli::demo_reset(&remote::demo::home(home), new_code).await,
            };
            match said {
                Ok(text) => {
                    println!("{}", ui::out().ok_line(&text));
                    Ok(())
                }
                Err(why) => Err(Failure::new(why)),
            }
        }
        Command::Telegram { action } => {
            let home = equipe_providers::credentials::equipe_home();
            let said = match action {
                None => Ok(telegram::status(&home)),
                Some(TelegramAction::Setup) => telegram::setup(&home).await,
                Some(TelegramAction::Off) => telegram::off(&home),
            };
            match said {
                Ok(text) => {
                    println!("{}", ui::out().ok_line(&text));
                    Ok(())
                }
                Err(why) => Err(Failure::new(why)),
            }
        }
        Command::Browser { max: Some(n), .. } => match settings::browser_max(&equipe_providers::credentials::equipe_home(), n.as_deref()) {
            Ok(line) => {
                println!("  {line}");
                Ok(())
            }
            Err(why) => Err(Failure::new(why)),
        },
        Command::Browser { url, forget, .. } => sign_in(url, forget).await,
        Command::Squad { words, squad, dry_run } => {
            let home = equipe_providers::credentials::equipe_home();
            let cwd = std::env::current_dir().unwrap_or_default();
            let current = home::squad_for(&home, &cwd, squad);
            let words: Vec<&str> = words.iter().map(String::as_str).collect();
            match settings::squad(&home, &cwd, current, &words, dry_run) {
                Ok(lines) => {
                    for line in lines {
                        println!("  {line}");
                    }
                    Ok(())
                }
                Err(why) => Err(Failure::new(why)),
            }
        }
        Command::SendApps { words, squad } => {
            let home = equipe_providers::credentials::equipe_home();
            let cwd = std::env::current_dir().unwrap_or_default();
            match home::squad_for(&home, &cwd, squad) {
                None => Err(Failure::new("No squad yet. Run equipe to start one.")),
                Some(squad) => match settings::send_apps(&home, &squad, &words.join(" ")) {
                    Ok(line) => {
                        println!("  {line}");
                        Ok(())
                    }
                    Err(why) => Err(Failure::new(why)),
                },
            }
        }
        Command::Mcp { server, squad, member, role, home } => serve_mcp(&server, squad, member, role, home).await,
        Command::Desktop { action } => desktop::run(action.map(|a| match a {
            DesktopAction::Setup { sign } => desktop::Action::Setup { sign },
            DesktopAction::Status => desktop::Action::Status,
            DesktopAction::On { member, experimental, squad } => desktop::Action::On { member, experimental, squad },
            DesktopAction::Off { member, squad } => desktop::Action::Off { member, squad },
            DesktopAction::Remove => desktop::Action::Remove,
            DesktopAction::Serve { home } => desktop::Action::Serve { home },
        }))
        .await
        .map_err(Failure::new),
        Command::Home { words, squad } => {
            let home = equipe_providers::credentials::equipe_home();
            let cwd = std::env::current_dir().unwrap_or_default();
            match home::squad_for(&home, &cwd, squad) {
                None => Err(Failure::new(t!("main.no_squad"))),
                Some(squad) => {
                    let words: Vec<&str> = words.iter().map(String::as_str).collect();
                    match home::run(&home::dir(&home, &squad), &words) {
                        Ok(lines) => {
                            let p = ui::out();
                            for line in lines {
                                println!("  {line}");
                            }
                            if words.is_empty() {
                                println!("  {}", p.hint("equipe home changes · equipe home undo [n]"));
                            }
                            Ok(())
                        }
                        Err(why) => Err(Failure::new(why)),
                    }
                }
            }
        }
        Command::Memory { words, squad } => {
            let home = equipe_providers::credentials::equipe_home();
            let cwd = std::env::current_dir().unwrap_or_default();
            let words: Vec<&str> = words.iter().map(String::as_str).collect();
            match home::squad_for(&home, &cwd, squad).map(|squad| home::memory(&home::dir(&home, &squad), &words)) {
                None => Err(Failure::new(t!("main.no_squad"))),
                Some(Err(why)) => Err(Failure::new(why)),
                Some(Ok(lines)) => {
                    for line in lines {
                        println!("  {line}");
                    }
                    if words.is_empty() {
                        println!("  {}", ui::out().hint(t!("main.memory_hint")));
                    }
                    Ok(())
                }
            }
        }
        Command::Squads => {
            let p = ui::out();
            let squads = equipe_core::squads::list(&equipe_providers::credentials::equipe_home());
            println!("{}\n", p.kicker(t!("main.squads_kicker")));
            if squads.is_empty() {
                println!("{}", p.note(2, t!("main.squads_none")));
            }
            for squad in squads {
                println!("  {}  {}", p.bold(&squad.name), p.dim(&home::dir(&equipe_providers::credentials::equipe_home(), &squad.name).display().to_string()));
                for project in squad.projects {
                    println!("{}", p.row("", &format!("{}  {}", equipe_core::board::project_name(&project), p.dim(&project.display().to_string()))));
                }
            }
            Ok(())
        }
        Command::Roles => {
            let p = ui::out();
            println!("{}\n", p.kicker(t!("main.roles_kicker")));
            for role in equipe_core::ROLES {
                println!("  {}  {}", p.bold(&format!("{:<9}", role.id)), ui::wrap(crate::i18n::role_summary(&role), 13, 13, ui::width()));
                let work = if role.edits { t!("main.role_edits") } else { t!("main.role_reads") };
                let model = match role.tier {
                    Tier::Top => t!("main.tier_top"),
                    Tier::Upper => t!("main.tier_upper"),
                    Tier::Mid => t!("main.tier_mid"),
                };
                println!("{}", p.row("", &p.dim(&format!("{work} · {model}"))));
            }
            println!("  {}  {}", p.bold(&format!("{:<9}", "custom")), ui::wrap(t!("main.role_custom"), 13, 13, ui::width()));
            println!("{}", p.row("", &p.dim(t!("main.role_custom_detail"))));
            println!("\n  {}", p.hint(t!("main.roles_hint")));
            Ok(())
        }
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            let p = ui::err();
            eprintln!("\n{}", p.fail_line(&failure.message));
            if let Some(next) = failure.next {
                eprintln!("  {}", p.hint(&next));
            }
            ExitCode::FAILURE
        }
    }
}

/// `equipe mcp`: a member's board tools, as a stdio MCP server. Every call goes
/// to the daemon, which decides; stdout carries only the protocol.
async fn serve_mcp(server: &str, squad: String, member: String, role: String, home: std::path::PathBuf) -> CmdResult {
    use equipe_core::mcp::{self, Step};
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

    let socket = equipe_core::gateway::socket_path(&home);
    let desktop = server == equipe_core::runtime::DESKTOP_SERVER;
    let (info, tools) = if desktop {
        (mcp::DESKTOP, equipe_desktop_control::tools::tools())
    } else {
        (mcp::BOARD, equipe_core::board::tools(role == equipe_core::roles::PM))
    };
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    while let Ok(Some(line)) = lines.next_line().await {
        let reply = match mcp::handle_for(&info, &line, &tools, VERSION) {
            Step::Reply(reply) => reply,
            Step::Call { id, name, arguments } => {
                let call = protocol::rpc::ToolCall { squad: squad.clone(), member: member.clone(), name, arguments };
                let result = if desktop {
                    equipe_core::gateway::call_desktop(&socket, &call).await
                } else {
                    equipe_core::gateway::call_tool(&socket, &call).await
                };
                let result = result.unwrap_or_else(|err| {
                    protocol::rpc::ToolResult { text: format!("equipe's daemon didn't answer: {err}"), is_error: true, image: None }
                });
                mcp::call_reply(id, &result)
            }
            Step::Nothing => continue,
        };
        let mut text = reply.to_string();
        text.push('\n');
        if out.write_all(text.as_bytes()).await.is_err() || out.flush().await.is_err() {
            break;
        }
    }
    Ok(())
}

/// `equipe browser`: a browser window for the person to sign in (or sign up)
/// where the squad should work. What they sign in to is kept as Playwright
/// storage state; members start their own browsers from it.
async fn sign_in(url: Option<String>, forget: bool) -> CmdResult {
    use equipe_core::browser;
    let p = ui::out();
    let home = equipe_providers::credentials::equipe_home();
    let state = browser::state_file(&home);
    if forget {
        let _ = std::fs::remove_file(&state);
        println!("{}", p.ok_line(t!("main.browser_forgot")));
        return Ok(());
    }
    // Members share one Chrome, run by the daemon: the person signs in there,
    // once — Google too — and every member works signed in.
    if daemon::ensure_running().await.is_ok()
        && equipe_core::gateway::person_window(&daemon::socket(), &url.clone().unwrap_or_else(start_page)).await.unwrap_or(false)
    {
        println!("{}\n", p.kicker(t!("main.browser_kicker")));
        println!("{}", p.note(2, t!("main.browser_chrome_1")));
        println!("{}", p.note(2, t!("main.browser_chrome_2")));
        println!("{}", p.note(2, t!("main.browser_chrome_3")));
        if std::env::var_os("SSH_CONNECTION").is_some() {
            println!("\n{}", p.note(2, t!("main.browser_ssh_chrome")));
        }
        return Ok(());
    }
    let npx = browser::find_npx().ok_or_else(|| Failure::new(t!("main.no_node")).next("brew install node"))?;
    let dir = state.parent().expect("the state file has a folder");
    std::fs::create_dir_all(dir).map_err(|err| Failure::new(t!("main.mkdir_failed", dir = dir.display(), err = err)))?;
    let _ = std::fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));

    println!("{}\n", p.kicker(t!("main.browser_kicker")));
    println!("{}", p.note(2, t!("main.browser_open_1")));
    println!("{}", p.note(2, t!("main.browser_open_2")));
    println!("{}", p.note(2, t!("main.browser_open_3")));
    println!("{}", p.note(2, t!("main.browser_open_4")));
    println!("{}", p.note(2, t!("main.browser_open_5")));
    if std::env::var_os("SSH_CONNECTION").is_some() {
        println!("\n{}", p.note(2, t!("main.browser_ssh")));
    }
    // Chrome as an ordinary browser, not an automated one: Google refuses to sign
    // anyone in to the latter.
    let mut open = std::process::Command::new(npx);
    open.args(["-y", "-p", browser::PLAYWRIGHT_PACKAGE, "node"])
        .arg(browser::sign_in_script(&home))
        .arg(&state)
        .arg(url.unwrap_or_else(start_page));
    let status = open.status().map_err(|err| Failure::new(t!("main.browser_open_failed", err = err)))?;
    let _ = std::fs::set_permissions(&state, std::os::unix::fs::PermissionsExt::from_mode(0o600));
    if !status.success() {
        return Err(Failure::new(t!("main.browser_error")).next("equipe browser"));
    }
    let sites = browser::sites(&std::fs::read_to_string(&state).unwrap_or_default());
    println!();
    if sites.is_empty() {
        println!("{}", p.note(2, t!("main.browser_nothing")));
    } else {
        println!("{}", p.ok_line(&t!("main.browser_sites", sites = sites.join(", "))));
        println!("{}", p.note(2, t!("main.browser_forget_hint")));
    }
    Ok(())
}

/// The page the sign-in window opens on: a few sites squads often need.
fn start_page() -> String {
    let sites = [
        ("GitHub", "https://github.com/login"),
        ("Vercel", "https://vercel.com/login"),
        ("PostHog", "https://app.posthog.com/login"),
        ("Supabase", "https://supabase.com/dashboard/sign-in"),
        ("Cloudflare", "https://dash.cloudflare.com/login"),
        ("Netlify", "https://app.netlify.com/login"),
        ("Google Search Console", "https://search.google.com/search-console"),
    ];
    let links: String = sites.iter().map(|(name, url)| format!("<li><a href='{url}'>{name}</a></li>")).collect();
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>equipe · {title}</title>\
<body style='font:16px -apple-system,system-ui,sans-serif;max-width:560px;margin:64px auto;line-height:1.6;color:#111'>\
<h1 style='font-size:22px'>equipe<span style='color:#8f9aa8'>.</span> {heading}</h1>\
<p>{intro}</p><ul>{links}</ul><p style='color:#666'>{members}</p><p style='color:#666'>{google}</p>",
        title = t!("main.page_title"),
        heading = t!("main.page_heading"),
        intro = t!("main.page_intro"),
        members = t!("main.page_members"),
        google = t!("main.page_google"),
    );
    let encoded: String = html
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b".-_~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    format!("data:text/html,{encoded}")
}

/// Where every release's install script lives; it also updates.
const INSTALL_SCRIPT: &str = "https://github.com/equipe-labs/equipe/releases/latest/download/install.sh";

/// `equipe update`: the latest release over this binary, by the same script
/// that installs it. Homebrew installs are pointed back to Homebrew.
async fn update() -> CmdResult {
    let p = ui::out();
    let exe = std::env::current_exe()
        .and_then(|exe| exe.canonicalize())
        .map_err(|err| Failure::new(t!("main.where_installed_err", err = err)))?;
    if update_by_homebrew(&exe) {
        println!("{}", p.note(0, t!("main.homebrew")));
        println!("  {}", p.hint("brew upgrade equipe"));
        return Ok(());
    }
    let dir = exe.parent().ok_or_else(|| Failure::new(t!("main.where_installed")))?;
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("curl -fsSL {INSTALL_SCRIPT} | sh"))
        .env("EQUIPE_INSTALL_DIR", dir)
        .env("EQUIPE_NO_MODIFY_PATH", "1")
        .status()
        .map_err(|err| Failure::new(t!("main.installer_failed", err = err)))?;
    if status.success() {
        // The squad keeps running the old build until the daemon restarts.
        if equipe_core::gateway::is_running(&daemon::socket()).await {
            println!("{}", p.note(2, t!("main.old_build")));
            println!("  {}", p.hint("equipe daemon restart --when-idle"));
        }
        Ok(())
    } else {
        Err(Failure::new(t!("main.update_failed")).next(format!("curl -fsSL {INSTALL_SCRIPT} | sh")))
    }
}

fn update_by_homebrew(exe: &std::path::Path) -> bool {
    let path = exe.to_string_lossy();
    path.contains("/Cellar/") || path.contains("/homebrew/") || path.contains("/linuxbrew/")
}

/// Wait until the daemon is between turns everywhere — a restart then cuts
/// no one off. Says once who it waits for, and again when that changes.
async fn wait_until_idle(p: &ui::Painter) -> CmdResult {
    let mut said = None;
    loop {
        // A daemon that doesn't answer has nothing under way worth waiting for.
        let Ok(busy) = tokio::time::timeout(daemon::ANSWER_WAIT, equipe_core::gateway::busy(&daemon::socket())).await else {
            return Ok(());
        };
        let busy = busy.map_err(|err| Failure::new(err.to_string()))?;
        let Some(busy) = busy.filter(|b| !b.idle()) else {
            return Ok(());
        };
        let mut waiting = busy.working.clone();
        if busy.approvals > 0 {
            waiting.push(if busy.approvals == 1 {
                t!("main.requests_for_you_one", n = busy.approvals)
            } else {
                t!("main.requests_for_you_many", n = busy.approvals)
            });
        }
        let line = t!("main.waiting_for", who = waiting.join(", "));
        if said.as_ref() != Some(&line) {
            println!("{}", p.note(0, &line));
            said = Some(line);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn daemon_command(action: DaemonAction) -> CmdResult {
    let p = ui::out();
    match action {
        DaemonAction::Run => daemon::run_foreground()
            .await
            .map_err(|err| Failure::new(t!("main.daemon_stopped_err", err = err))),
        DaemonAction::Start => match daemon::ensure_running().await {
            Ok(true) => {
                println!("{}", p.ok_line(t!("main.daemon_started")));
                Ok(())
            }
            Ok(false) => {
                println!("{}", p.ok_line(t!("main.daemon_already")));
                Ok(())
            }
            Err(err) => Err(Failure::new(err.to_string())),
        },
        DaemonAction::Stop => match daemon::stop().await {
            Ok(daemon::Stopped::Asked) => {
                println!("{}", p.ok_line(t!("main.daemon_stopped")));
                Ok(())
            }
            Ok(daemon::Stopped::Ended(pid)) => {
                println!("{}", p.ok_line(&t!("main.daemon_ended", pid = pid)));
                Ok(())
            }
            Ok(daemon::Stopped::None) => {
                println!("{}", p.note(0, t!("main.daemon_none")));
                Ok(())
            }
            Err(err) => Err(Failure::new(err.to_string())),
        },
        DaemonAction::Restart { when_idle } => {
            if when_idle {
                wait_until_idle(&p).await?;
            }
            if let Ok(daemon::Stopped::Ended(pid)) = daemon::stop().await {
                println!("{}", p.note(0, &t!("main.daemon_ended", pid = pid)));
            }
            daemon::ensure_running().await.map_err(|err| Failure::new(err.to_string()))?;
            println!("{}", p.ok_line(t!("main.daemon_restarted")));
            Ok(())
        }
        DaemonAction::Install => {
            let exe = std::env::current_exe().map_err(|err| Failure::new(err.to_string()))?;
            // The service takes over from a daemon already running — between turns.
            wait_until_idle(&p).await?;
            let _ = daemon::stop().await;
            daemon::service::install(&exe, &daemon::log_path()).map_err(|err| Failure::new(err.to_string()))?;
            for _ in 0..100 {
                if equipe_core::gateway::is_running(&daemon::socket()).await {
                    println!("{}", p.ok_line(t!("main.service_installed")));
                    println!("{}", p.note(2, t!("main.service_uninstall_hint")));
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(Failure::new(t!("main.service_no_answer")).next(format!("tail {}", daemon::log_path().display())))
        }
        DaemonAction::Uninstall => {
            if daemon::service::uninstall().map_err(|err| Failure::new(err.to_string()))? {
                println!("{}", p.ok_line(t!("main.service_removed")));
            } else {
                println!("{}", p.note(0, t!("main.service_none")));
            }
            Ok(())
        }
        DaemonAction::Status => {
            // Never waits for good: a daemon that takes the connection but answers
            // nothing is said to be so, with the way out.
            let answer = daemon::ask().await;
            let busy = match answer {
                daemon::Answer::NotRunning => {
                    println!("{}", p.note(0, t!("main.daemon_not_running")));
                    return Ok(());
                }
                daemon::Answer::Silent => {
                    let secs = daemon::ANSWER_WAIT.as_secs();
                    return Err(Failure::new(t!("main.daemon_silent", secs = secs)).next("equipe daemon restart"));
                }
                daemon::Answer::Busy(busy) => busy,
            };
            println!("{}", p.ok_line(t!("main.daemon_running")));
            println!("{}", p.row(t!("main.row_socket"), &daemon::socket().display().to_string()));
            let how = if daemon::service::installed() { t!("main.runs_as_service") } else { t!("main.runs_on_demand") };
            println!("{}", p.row(t!("main.row_runs_as"), how));
            if let Some(stall) = &busy.stalled {
                let since = equipe_core::watchdog::clock(stall.since);
                println!("{}", p.row(t!("main.row_stalled"), &t!("main.stalled_since", since = since, doing = stall.doing, secs = stall.secs)));
                println!("{}", p.note(2, t!("main.stalled_hint")));
                return Ok(());
            }
            let working = if busy.working.is_empty() { t!("main.nobody_working").to_owned() } else { busy.working.join(", ") };
            println!("{}", p.row(t!("main.row_working"), &working));
            println!("{}", p.row(t!("main.row_screen"), &equipe_core::i18n::label::desktop(crate::i18n::lang(), &busy.desktop, None).unwrap_or_else(|| t!("main.screen_free").into())));
            Ok(())
        }
    }
}

/// What went wrong, and the command that fixes it when there is one.
struct Failure {
    message: String,
    next: Option<String>,
}

impl Failure {
    fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), next: None }
    }

    fn next(mut self, command: impl Into<String>) -> Self {
        self.next = Some(command.into());
        self
    }
}

impl From<equipe_providers::Error> for Failure {
    fn from(err: equipe_providers::Error) -> Self {
        let next = matches!(err, equipe_providers::Error::Unauthorized { .. }).then(|| "equipe login codex".to_owned());
        Failure { message: err.to_string(), next }
    }
}

type CmdResult = Result<(), Failure>;

struct App {
    store: CredentialStore,
    client: reqwest::Client,
}

impl App {
    // ── login ──

    async fn login(&self, engine: Option<Engine>, timeout: u64, no_browser: bool, again: bool) -> CmdResult {
        let p = ui::out();
        // Named engine: that one. No name, in a terminal: let the user choose.
        // No name and no terminal (scripts): every engine that isn't ready.
        let (engines, chosen) = match engine {
            Some(engine) => (vec![engine], false),
            None if ui::interactive() => match self.pick_engine().await {
                Some(engine) => (vec![engine], true),
                None => {
                    println!("{}", p.note(0, t!("main.cancelled")));
                    return Ok(());
                }
            },
            None => (vec![Engine::Codex, Engine::Claude], false),
        };
        for (i, engine) in engines.into_iter().enumerate() {
            if i > 0 {
                println!();
            }
            match engine {
                Engine::Codex if codex_app::find(codex_app::DEFAULT_BINARY).is_some() => match codex_app::login_status(codex_app::DEFAULT_BINARY).await {
                    Ok(Some(sign_in)) if sign_in.signed_in() && !again && !chosen => {
                        println!("{}", p.ok_line(&t!("main.codex_cli_already", how = describe_codex_login(sign_in))));
                        println!("{}", p.note(2, t!("main.codex_again")));
                    }
                    _ => self.login_codex_cli().await?,
                },
                Engine::Codex => match self.store.load("openai") {
                    Ok(Some(cred)) if !again && !chosen => {
                        let account = cred.account_id.map(|a| format!(" · {a}")).unwrap_or_default();
                        println!("{}", p.ok_line(&t!("main.codex_already", account = account)));
                        println!("{}", p.note(2, t!("main.codex_again")));
                    }
                    _ => self.login_codex(timeout, no_browser).await?,
                },
                Engine::Claude => match claude_code::auth_status(claude_code::DEFAULT_BINARY).await {
                    Ok(Some(status)) if status.logged_in && !again => {
                        println!("{}", p.ok_line(&t!("main.claude_already", how = describe_claude_login(&status))));
                        println!("{}", p.note(2, t!("main.claude_again")));
                    }
                    _ => self.login_claude().await?,
                },
            }
        }
        Ok(())
    }

    /// Ask which engine to sign in to, showing where each one stands.
    async fn pick_engine(&self) -> Option<Engine> {
        let codex_ready = match codex_app::login_status(codex_app::DEFAULT_BINARY).await {
            Ok(Some(sign_in)) => sign_in.signed_in(),
            _ => matches!(self.store.load("openai"), Ok(Some(_))),
        };
        let claude = claude_code::auth_status(claude_code::DEFAULT_BINARY).await.ok().flatten();
        let claude_ready = claude.as_ref().is_some_and(|s| s.logged_in);

        // One column per fact — engine, state, plan — so the rows compare at a glance.
        let e = ui::err();
        let state = |ready: bool, missing: &str| {
            if ready {
                format!("{} {}", e.accent("✓"), ui::pad(t!("main.signed_in"), 15))
            } else {
                format!("{} {}", e.dim("○"), ui::pad(missing, 15))
            }
        };
        let claude_plan = match &claude {
            Some(s) if s.logged_in => plan_label(s),
            _ => t!("main.your_claude").to_owned(),
        };
        let items = [
            format!("{:<13}{}{}", "codex", state(codex_ready, t!("main.not_signed_in")), e.dim(t!("main.chatgpt_plan"))),
            format!(
                "{:<13}{}{}",
                "claude code",
                state(claude_ready, if claude.is_none() { t!("main.not_installed") } else { t!("main.not_signed_in") }),
                e.dim(&claude_plan)
            ),
        ];
        // Start on the first engine that still needs a sign-in.
        let default = if !codex_ready { 0 } else if !claude_ready { 1 } else { 0 };

        eprintln!("{}  {}\n", e.wordmark(), e.dim(t!("main.sign_in")));
        let picked = dialoguer::Select::with_theme(&*ui::select_theme())
            .with_prompt(t!("main.which_engine"))
            .items(&items)
            .default(default)
            .interact_opt()
            .ok()
            .flatten()?;
        eprintln!();
        Some(if picked == 0 { Engine::Codex } else { Engine::Claude })
    }

    /// Forget the Codex credential equipe holds. Claude Code's login is not equipe's.
    fn logout(&self) -> CmdResult {
        let p = ui::out();
        let path = self.store.path("openai");
        match self.store.remove("openai") {
            Ok(true) => println!("{}", p.ok_line(t!("main.codex_signed_out"))),
            Ok(false) => println!("{}", p.note(0, t!("main.codex_wasnt_signed_in"))),
            Err(err) => return Err(Failure::new(t!("main.remove_failed", path = path.display(), err = err))),
        }
        println!("{}", p.note(2, t!("main.claude_own_login")));
        println!("\n  {}", p.hint("equipe login codex"));
        Ok(())
    }

    async fn login_codex(&self, timeout_min: u64, no_browser: bool) -> CmdResult {
        let p = ui::out();
        println!("{}  {}\n", p.wordmark(), p.dim(&format!("{} · codex", t!("main.sign_in"))));
        println!("{}", p.para(0, t!("main.codex_intro")));
        println!("{}\n", p.note(0, t!("main.codex_identity")));

        let options = login::LoginOptions {
            timeout: Duration::from_secs(timeout_min * 60),
            open_browser: !no_browser,
        };
        // The link is the fallback for every way a browser can fail to show up,
        // including "opened" in a window the user can't see. Pressing Enter prints it.
        let link: Arc<Mutex<Option<String>>> = Arc::default();
        if std::io::stdin().is_terminal() {
            let link = Arc::clone(&link);
            // A plain thread: a pending stdin read must not hold up process exit.
            std::thread::spawn(move || {
                let mut line = String::new();
                if std::io::stdin().read_line(&mut line).is_ok()
                    && let Some(url) = link.lock().ok().and_then(|l| l.clone())
                {
                    println!("\n{url}\n");
                }
            });
        }
        let cred = login::login(&self.client, &self.store, options, |url, opened| {
            if let Ok(mut slot) = link.lock() {
                *slot = Some(url.to_owned());
            }
            match opened {
                Ok(()) => {
                    println!("{}", p.ok_line(t!("main.signin_opened")));
                    println!("{}", p.note(2, t!("main.signin_not_there")));
                }
                Err(reason) => {
                    if !no_browser {
                        println!("{}", p.fail_line(&t!("main.no_browser", reason = reason)));
                    }
                    // Alone on its line and unindented, so it copies cleanly.
                    println!("{}\n", t!("main.open_link"));
                    println!("{url}\n");
                }
            }
            println!("\n{}", p.note(0, &t!("main.waiting_browser", minutes = timeout_min)));
        })
        .await
        .map_err(|err| {
            let retry = if matches!(err, equipe_providers::Error::LoginTimeout) {
                format!("equipe login codex --timeout {}", timeout_min * 2)
            } else {
                "equipe login codex".into()
            };
            Failure::new(t!("main.signin_incomplete", err = err)).next(retry)
        })?;

        let account = cred.account_id.as_deref().map(|a| format!(" · {a}")).unwrap_or_default();
        println!("\n{}", p.ok_line(&t!("main.signed_in_chatgpt", account = account)));
        println!("{}", p.row(t!("main.row_saved"), &self.store.path("openai").display().to_string()));
        println!("\n  {}", p.hint("equipe try \"hello\""));
        Ok(())
    }

    /// Sign the person's own Codex CLI in: `codex login`, in their hands. equipe
    /// never sees the token.
    async fn login_codex_cli(&self) -> CmdResult {
        let p = ui::out();
        println!("{}  {}\n", p.wordmark(), p.dim(&format!("{} · codex", t!("main.sign_in"))));
        println!("{}\n", p.para(0, t!("main.codex_cli_intro")));
        let status = tokio::process::Command::new(codex_app::DEFAULT_BINARY)
            .arg("login")
            .status()
            .await
            .map_err(|err| Failure::new(err.to_string()).next("equipe status"))?;
        if !status.success() {
            return Err(Failure::new(t!("main.codex_cli_incomplete")).next("codex login"));
        }
        println!();
        self.codex_section(&ui::out()).await;
        Ok(())
    }

    async fn login_claude(&self) -> CmdResult {
        let p = ui::out();
        println!("{}  {}\n", p.wordmark(), p.dim(&format!("{} · claude code", t!("main.sign_in"))));
        println!("{}", p.para(0, t!("main.claude_intro")));
        println!("{}\n", p.note(0, t!("main.claude_identity")));

        let status = tokio::process::Command::new(claude_code::DEFAULT_BINARY)
            .args(["auth", "login"])
            .status()
            .await
            .map_err(|_| claude_missing())?;
        if !status.success() {
            return Err(Failure::new(t!("main.claude_incomplete")).next("claude auth login"));
        }
        println!();
        self.claude_section(&ui::out()).await;
        Ok(())
    }

    // ── status ──

    async fn status(&self) -> CmdResult {
        let p = ui::out();
        println!("{}  {}\n", p.wordmark(), p.dim(&format!("v{VERSION}")));
        let codex_ready = self.codex_section(&p).await;
        println!();
        let claude_ready = self.claude_section(&p).await;

        let ready: Vec<_> = [(codex_ready, "codex"), (claude_ready, "claude")]
            .into_iter()
            .filter_map(|(ok, name)| ok.then_some(name))
            .collect();
        println!();
        if ready.is_empty() {
            return Err(Failure::new(t!("main.no_engine_ready")).next("equipe login codex"));
        }
        println!("{}", p.row(t!("main.row_ready"), &p.bold(&ready.join(" · "))));
        // The Mac's screen, when a member has it, waits for it, or the person took it back.
        if let Ok(Some(busy)) = equipe_core::gateway::busy(&daemon::socket()).await
            && let Some(desktop) = equipe_core::i18n::label::desktop(crate::i18n::lang(), &busy.desktop, None)
        {
            println!("{}", p.row(t!("main.row_screen"), &desktop));
        }
        Ok(())
    }

    /// Prints the Codex section; returns whether Codex is usable. Which path
    /// equipe takes: the person's own Codex CLI (`codex app-server`), or —
    /// without it — equipe's own sign-in, which only answers.
    async fn codex_section(&self, p: &ui::Painter) -> bool {
        let binary = codex_app::DEFAULT_BINARY;
        let Some(path) = codex_app::find(binary) else {
            return self.codex_fallback_section(p).await;
        };
        println!("{}", p.section("codex", t!("main.codex_cli_section")));
        println!("{}", p.row(t!("main.row_path"), &t!("main.codex_path_cli", binary = path.display())));
        match codex_app::login_status(binary).await {
            Ok(Some(sign_in)) if sign_in.signed_in() => println!("{}", p.row(t!("main.row_login"), &p.ok(&describe_codex_login(sign_in)))),
            Ok(_) => {
                println!("{}", p.row(t!("main.row_login"), &p.fail(t!("main.codex_cli_signed_out"))));
                println!("{}", p.row("", &p.hint("equipe login codex")));
                return false;
            }
            Err(err) => {
                println!("{}", p.row(t!("main.row_login"), &p.fail(&err.to_string())));
                return false;
            }
        }
        let cwd = std::env::temp_dir();
        match codex_app::list_models(binary, &cwd).await {
            Ok(models) => {
                let shown = models.iter().take(4).cloned().collect::<Vec<_>>().join(", ");
                let more = if models.len() > 4 { ", …" } else { "" };
                println!("{}", p.row(t!("main.row_backend"), &p.ok(&t!("main.models", count = models.len(), list = p.dim(&format!("{shown}{more}"))))));
            }
            Err(err) => {
                println!("{}", p.row(t!("main.row_backend"), &p.fail(&err.to_string())));
                return false;
            }
        }
        // The plan's usage, its windows by their length (a Pro plan has no five-hour one).
        if let Ok(limits) = codex_app::read_limits(binary, &cwd).await {
            let windows: Vec<String> = limits
                .windows
                .iter()
                .map(|w| {
                    let span = match w.minutes {
                        Some(m) if m % 1440 == 0 => format!("{}d", m / 1440),
                        Some(m) if m % 60 == 0 => format!("{}h", m / 60),
                        Some(m) => format!("{m}m"),
                        None => "?".into(),
                    };
                    format!("{span} {}%", w.used_percent.round())
                })
                .collect();
            let plan = limits.plan_type.clone().unwrap_or_default();
            println!("{}", p.row(t!("main.row_usage"), &[plan, windows.join(" · ")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")));
        }
        true
    }

    /// The fallback without the Codex CLI: equipe's own sign-in, no tools.
    async fn codex_fallback_section(&self, p: &ui::Painter) -> bool {
        println!("{}", p.section("codex", t!("main.codex_section")));
        println!("{}", p.row(t!("main.row_path"), &p.dim(t!("main.codex_path_fallback"))));
        let cred = match self.codex_credentials().await {
            Ok(cred) => cred,
            Err(failure) => {
                println!("{}", p.row(t!("main.row_login"), &p.fail(&failure.message)));
                if let Some(next) = failure.next {
                    println!("{}", p.row("", &p.hint(&next)));
                }
                return false;
            }
        };
        if let Some(account) = &cred.account_id {
            println!("{}", p.row(t!("main.row_account"), account));
        }
        if let Some(residency) = &cred.residency {
            println!("{}", p.row(t!("main.row_residency"), residency));
        }
        if let Some(exp) = cred.expires {
            let left = (exp - now_secs()) / 60;
            println!("{}", p.row(t!("main.row_token"), &t!("main.token_left", minutes = left)));
        }
        match codex::list_models(&self.client, &cred, VERSION).await {
            Ok(models) => {
                let shown = models.iter().take(4).cloned().collect::<Vec<_>>().join(", ");
                let more = if models.len() > 4 { ", …" } else { "" };
                println!("{}", p.row(t!("main.row_backend"), &p.ok(&t!("main.models", count = models.len(), list = p.dim(&format!("{shown}{more}"))))));
                true
            }
            Err(err) => {
                println!("{}", p.row(t!("main.row_backend"), &p.fail(&err.to_string())));
                false
            }
        }
    }

    /// Prints the Claude Code section; returns whether Claude Code is usable.
    async fn claude_section(&self, p: &ui::Painter) -> bool {
        println!("{}", p.section("claude code", t!("main.claude_section")));
        match claude_code::auth_status(claude_code::DEFAULT_BINARY).await {
            Ok(Some(status)) if status.logged_in => {
                println!("{}", p.row(t!("main.row_login"), &p.ok(&describe_claude_login(&status))));
                true
            }
            Ok(Some(_)) => {
                println!("{}", p.row(t!("main.row_login"), &p.fail(t!("main.not_signed_in"))));
                println!("{}", p.row("", &p.hint("equipe login claude")));
                false
            }
            Ok(None) => {
                println!("{}", p.row(t!("main.row_install"), &p.fail(t!("main.claude_not_on_path"))));
                println!("{}", p.row("", &p.dim(t!("main.install_claude"))));
                false
            }
            Err(err) => {
                println!("{}", p.row(t!("main.row_login"), &p.fail(&err.to_string())));
                false
            }
        }
    }

    /// Load the Codex login and refresh it if stale.
    async fn codex_credentials(&self) -> Result<Credentials, Failure> {
        let path = self.store.path("openai");
        let cred = self
            .store
            .load("openai")
            .map_err(|err| Failure::new(t!("main.read_failed", path = path.display(), err = err)).next("equipe login codex"))?
            .filter(|c| c.method == AuthMethod::OauthSubscription)
            .ok_or_else(|| Failure::new(t!("main.not_signed_in")).next("equipe login codex"))?;
        codex::ensure_fresh(&self.client, &self.store, cred)
            .await
            .map_err(|err| Failure::new(t!("main.refresh_failed", err = err)).next("equipe login codex"))
    }

    // ── try ──

    /// One turn, as a client of the runtime: send commands, render events.
    async fn run_try(&self, prompt: &str, engine: Option<Engine>, model: Option<&str>, edit: bool) -> CmdResult {
        if prompt.trim().is_empty() {
            return Err(Failure::new(t!("main.try_needs_prompt")).next("equipe try \"summarize this repo\""));
        }
        let cwd = std::env::current_dir().map_err(|err| Failure::new(err.to_string()))?;
        let access = if edit { protocol::Access::EditFiles } else { protocol::Access::ReadOnly };
        // Nothing named: the first engine ready — for an edit, Claude Code first.
        let engine = engine.map(protocol::Engine::from);

        let (core, mut events) = runtime::start(runtime::Config::for_user(VERSION));
        const ID: protocol::SessionId = 1;
        core.send(Cmd::Start {
            session: ID,
            spec: SessionSpec { engine, model: model.map(str::to_owned), access, cwd: cwd.clone(), role: None, squad: None },
        });

        let e = ui::err();
        let p = ui::out();
        let mut printer = Printer::new(&cwd);
        let mut started = false;
        let mut failure: Option<Failure> = None;
        while let Some(event) = events.recv().await {
            match event {
                Event::SessionStarted { engine, model, workspace, .. } => {
                    started = true;
                    let model = model.map(|m| format!(" · {m}")).unwrap_or_default();
                    match &workspace {
                        Some(ws) => {
                            eprintln!("{}", e.dim(&t!("main.may_edit", engine = engine.label(), model = model)));
                            let base = &ws.base[..ws.base.len().min(7)];
                            eprintln!("{}", e.row(t!("main.row_branch"), &format!("{}  {}", e.bold(&ws.branch), e.dim(&t!("main.from_base", base = base)))));
                            eprintln!("{}\n", e.row("", &e.dim(t!("main.checkout_untouched"))));
                            printer = Printer::new(&ws.path);
                        }
                        None => eprintln!("{}\n", e.dim(&format!("{}{model}", engine.label()))),
                    }
                    core.send(Cmd::Submit { session: ID, text: prompt.to_owned(), about: None });
                }
                Event::TextDelta { text, .. } => printer.text(&text),
                Event::ToolUse { tool, target, .. } => printer.tool(&tool, target),
                Event::ModelKnown { .. } | Event::UserMessage { .. } | Event::Thread { .. } => {}
                // Work handed to a subagent shows as it starts and as it hands back.
                Event::SubagentStarted { what, .. } => eprintln!("{}", e.dim(&format!("  ↳ {what}"))),
                Event::SubagentEnded { .. } => eprintln!("{}", e.dim(&format!("  ↳ {}", t!("main.handed_back")))),
                Event::SubagentStep { .. } | Event::BrowserFrame { .. } | Event::Suggestion { .. } | Event::SteerQueued { .. } | Event::Steered { .. } | Event::LaneOpened { .. } | Event::Estimated { .. } | Event::MemoWithdrawn { .. } | Event::QueueSent { .. } => {}
                Event::ToolResult { .. } | Event::Thinking { .. } => {}
                Event::BrowserTabs { .. } | Event::BrowserControl { .. } | Event::BrowserDialog { .. } | Event::BrowserDialogClosed { .. }
                | Event::BrowserWants { .. }
                | Event::PairOffer { .. } | Event::PairAsked { .. } | Event::Paired { .. } | Event::PairEnded { .. } | Event::Devices { .. }
                | Event::DiskLevel { .. } | Event::DiskReport { .. } | Event::DiskCleaned { .. }  | Event::Engines { .. } | Event::SignIn { .. } => {}
                Event::TurnEnded { outcome, usage, .. } => {
                    printer.end(outcome, usage);
                    core.send(Cmd::Close { session: ID });
                }
                Event::BranchLanded { branch, stat, .. } => {
                    println!("\n{}", p.ok_line(&t!("main.committed_to", branch = p.bold(&branch))));
                    for line in stat.lines() {
                        println!("  {}", p.dim(line.trim()));
                    }
                    println!();
                    println!("{}", p.row(t!("main.row_review"), &p.bold(&format!("git show {branch}"))));
                    println!("{}", p.row(t!("main.row_keep"), &p.bold(&format!("git merge {branch}"))));
                    println!("{}", p.row(t!("main.row_discard"), &p.bold(&format!("git branch -D {branch}"))));
                }
                Event::NothingChanged { .. } => println!("\n{}", p.note(0, t!("main.nothing_changed"))),
                Event::SessionClosed { .. } => break,
                // One prompt, one turn: no memos, no squad, no board.
                Event::MessageQueued { .. }
                | Event::PlanUsage { .. }
                | Event::Desktop { .. }
                | Event::Output { .. }
                | Event::Notice { .. }
                | Event::WorkStarted { .. }
                | Event::WorkEnded { .. }
                | Event::TicketChanged { .. }
                | Event::TicketHistory { .. }
                | Event::ModeChanged { .. }
                | Event::ApprovalRequested { .. }
                | Event::ApprovalResolved { .. }
                | Event::QuestionsAsked { .. }
                | Event::QuestionsClosed { .. }
                | Event::TeamChangeProposed { .. }
                | Event::TeamChangeClosed { .. }
                | Event::Allowed { .. } => {}
                Event::Error { message, next, .. } => {
                    failure.get_or_insert(Failure { message, next });
                    // A session that never opened won't send anything more.
                    if !started {
                        break;
                    }
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

fn plan_label(status: &AuthStatus) -> String {
    if status.uses_subscription() {
        let plan = status.subscription_type.as_deref().unwrap_or("claude.ai");
        let mut chars = plan.chars();
        let plan: String = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
        t!("main.plan", plan = plan)
    } else {
        t!("main.billed_per_token").to_owned()
    }
}

/// How the Codex CLI is signed in, in a few words.
fn describe_codex_login(sign_in: codex_app::SignIn) -> String {
    match sign_in {
        codex_app::SignIn::ApiKey => t!("main.codex_cli_api_key").to_owned(),
        _ => t!("main.codex_cli_chatgpt").to_owned(),
    }
}

fn describe_claude_login(status: &AuthStatus) -> String {
    if status.uses_subscription() {
        let plan = status.subscription_type.as_deref().unwrap_or("claude.ai");
        t!("main.subscription", plan = plan)
    } else {
        // Billed per token, not against a subscription — worth saying out loud.
        let how = status.auth_method.as_deref().unwrap_or(t!("main.signed_in"));
        format!("{how} · {}", t!("main.billed_per_token"))
    }
}

fn claude_missing() -> Failure {
    Failure::new(t!("main.claude_missing")).next(t!("main.claude_setup"))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Streams a turn to the terminal: the answer on stdout, what the agent is doing
/// as dim lines on stderr, so `equipe try … > answer.txt` keeps only the answer.
struct Printer {
    base: std::path::PathBuf,
    mid_line: bool,
}

impl Printer {
    fn new(base: &std::path::Path) -> Self {
        Self {
            // Canonical, so tool paths under a symlinked temp dir still shorten.
            base: base.canonicalize().unwrap_or_else(|_| base.to_owned()),
            mid_line: false,
        }
    }

    fn text(&mut self, text: &str) {
        self.mid_line = !text.ends_with('\n');
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }

    fn tool(&mut self, name: &str, target: Option<String>) {
        if self.mid_line {
            println!();
            self.mid_line = false;
        }
        let verb = tool_verb(name);
        let target = target.map(|t| self.relative(&t)).unwrap_or_default();
        let e = ui::err();
        let mark = if matches!(verb, "edit" | "write") { e.accent("✎") } else { e.dim("·") };
        let shown = match verb {
            "read" => t!("main.verb_read"),
            "edit" => t!("main.verb_edit"),
            "write" => t!("main.verb_write"),
            "run" => t!("main.verb_run"),
            "search" => t!("main.verb_search"),
            "fetch" => t!("main.verb_fetch"),
            other => other,
        };
        eprintln!("{mark} {}", e.dim(&format!("{} {target}", ui::pad(shown, 6))));
    }

    fn relative(&self, target: &str) -> String {
        let path = std::path::Path::new(target);
        let path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        path.strip_prefix(&self.base)
            .map(|rel| rel.display().to_string())
            .unwrap_or_else(|_| target.to_owned())
    }

    /// Close the answer and print the footer.
    fn end(&mut self, outcome: TurnOutcome, usage: Option<Usage>) {
        if self.mid_line {
            println!();
            self.mid_line = false;
        }
        let e = ui::err();
        let mut footer = Vec::new();
        match outcome {
            TurnOutcome::Completed | TurnOutcome::Failed => {}
            TurnOutcome::Incomplete => footer.push(t!("main.ended_early").to_owned()),
            TurnOutcome::Interrupted => footer.push(t!("main.interrupted").to_owned()),
        }
        if let Some(usage) = usage {
            let n = |v: Option<u64>| v.map_or("?".into(), |v| v.to_string());
            footer.push(t!("main.tokens", input = n(usage.input_tokens), output = n(usage.output_tokens)));
        }
        if !footer.is_empty() {
            eprintln!("\n{}", e.dim(&footer.join(" · ")));
        }
    }
}

/// How a tool use reads in one word.
fn tool_verb(name: &str) -> &str {
    match name {
        "Read" => "read",
        "Edit" | "MultiEdit" | "NotebookEdit" => "edit",
        "Write" => "write",
        "Bash" => "run",
        "Glob" | "Grep" => "search",
        "WebFetch" | "WebSearch" => "fetch",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homebrew_installs_update_through_homebrew() {
        assert!(update_by_homebrew(std::path::Path::new("/opt/homebrew/Cellar/equipe/0.1.0/bin/equipe")));
        assert!(update_by_homebrew(std::path::Path::new("/home/linuxbrew/.linuxbrew/bin/equipe")));
        assert!(!update_by_homebrew(std::path::Path::new("/Users/me/.local/bin/equipe")));
    }

    /// The tests are built with ring as well as aws-lc-rs (a dev-dependency),
    /// as a workspace build is: rustls can't choose by itself, and a TLS
    /// client config — what the relay's `wss://` connect makes — panics
    /// unless one was installed.
    #[test]
    fn tls_is_ready_for_the_relay_with_two_crypto_providers_built_in() {
        install_tls();
        install_tls();
        let config = rustls::ClientConfig::builder().with_root_certificates(rustls::RootCertStore::empty()).with_no_client_auth();
        assert!(config.crypto_provider().cipher_suites.iter().any(|s| s.version() == &rustls::version::TLS13));
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }
}
