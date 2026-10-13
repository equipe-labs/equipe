//! `equipe` — the command-line entry point.
//!
//! Two engines, both on the user's own plans:
//! - **codex**: equipe signs in to the ChatGPT (Codex) subscription itself.
//! - **claude**: the user's installed Claude Code, which signs itself in. equipe
//!   never sees Claude credentials (see `equipe_providers::claude_code`).

mod daemon;
mod onboard;
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
use equipe_providers::claude_code::{self, AuthStatus};
use equipe_providers::openai::{codex, login};
use equipe_providers::{AuthMethod, CredentialStore, Credentials};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name = "equipe", version, about = "A full software squad of AI agents.")]
struct Cli {
    /// With no command: the team console, at a terminal.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Engine {
    /// ChatGPT (Codex) subscription, signed in by equipe
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
    /// Serve a member's board tools over MCP (equipe starts this for its engines)
    #[command(hide = true)]
    Mcp {
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
    },
    /// Update equipe to the latest release
    Update,
    /// The background daemon the team lives in
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
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

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
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
        let start = match onboard::squad_for(&cwd).await {
            Ok(start) => start,
            Err(err) => {
                eprintln!("{}", ui::err().fail_line(&err));
                eprintln!("  {}", ui::err().hint("equipe login"));
                return ExitCode::FAILURE;
            }
        };
        return match tui::run(cwd, start).await {
            Ok(summary) => {
                tui::print_summary(&summary);
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("{}", ui::err().fail_line(&format!("The terminal UI stopped: {err}")));
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
        Command::Update => update().await,
        Command::Browser { url, forget } => sign_in(url, forget),
        Command::Mcp { squad, member, role, home } => serve_mcp(squad, member, role, home).await,
        Command::Squads => {
            let p = ui::out();
            let squads = equipe_core::squads::list(&equipe_providers::credentials::equipe_home());
            println!("{}\n", p.kicker("squads"));
            if squads.is_empty() {
                println!("{}", p.note(2, "None yet. Run equipe in a repository to start one."));
            }
            for squad in squads {
                println!("  {}", p.bold(&squad.name));
                for project in squad.projects {
                    println!("{}", p.row("", &format!("{}  {}", equipe_core::board::project_name(&project), p.dim(&project.display().to_string()))));
                }
            }
            Ok(())
        }
        Command::Roles => {
            let p = ui::out();
            println!("{}\n", p.kicker("squad roles"));
            for role in equipe_core::ROLES {
                println!("  {}  {}", p.bold(&format!("{:<9}", role.id)), ui::wrap(role.summary, 13, 13, ui::width()));
                let work = if role.edits { "edits files on its own branch" } else { "reads and reports" };
                let model = match role.tier {
                    Tier::Top => "opus · Codex's top model",
                    Tier::Upper => "sonnet · Codex's top model",
                    Tier::Mid => "sonnet · a lighter Codex model",
                };
                println!("{}", p.row("", &p.dim(&format!("{work} · {model}"))));
            }
            println!("\n  {}", p.hint("equipe   then /add designer frontend"));
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
async fn serve_mcp(squad: String, member: String, role: String, home: std::path::PathBuf) -> CmdResult {
    use equipe_core::mcp::{self, Step};
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

    let socket = equipe_core::gateway::socket_path(&home);
    let tools = equipe_core::board::tools(role == equipe_core::roles::PM);
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    while let Ok(Some(line)) = lines.next_line().await {
        let reply = match mcp::handle(&line, &tools, VERSION) {
            Step::Reply(reply) => reply,
            Step::Call { id, name, arguments } => {
                let call = protocol::rpc::ToolCall { squad: squad.clone(), member: member.clone(), name, arguments };
                let result = equipe_core::gateway::call_tool(&socket, &call).await.unwrap_or_else(|err| {
                    protocol::rpc::ToolResult { text: format!("equipe's daemon didn't answer: {err}"), is_error: true }
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
fn sign_in(url: Option<String>, forget: bool) -> CmdResult {
    use equipe_core::browser;
    let p = ui::out();
    let home = equipe_providers::credentials::equipe_home();
    let state = browser::state_file(&home);
    if forget {
        let _ = std::fs::remove_file(&state);
        println!("{}", p.ok_line("Signed out everywhere. Members have no browser until you sign in again."));
        return Ok(());
    }
    let npx = browser::find_npx().ok_or_else(|| Failure::new("The browser runs on Node.js, which isn't installed.").next("brew install node"))?;
    let dir = state.parent().expect("the state file has a folder");
    std::fs::create_dir_all(dir).map_err(|err| Failure::new(format!("Couldn't make {}: {err}", dir.display())))?;
    let _ = std::fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));

    println!("{}\n", p.kicker("browser"));
    println!("{}", p.note(2, "A browser opens. Sign in — or sign up — wherever your squad should work: Vercel, PostHog, GitHub…"));
    println!("{}", p.note(2, "Close the window when you're done. Members then work there signed in as you, and ask you before"));
    println!("{}", p.note(2, "creating accounts, accepting terms, paying, or deleting anything."));
    if std::env::var_os("SSH_CONNECTION").is_some() {
        println!("\n{}", p.note(2, "You're connected over SSH: the window opens on this machine's own screen. Reach it with Screen Sharing."));
    }
    let mut open = std::process::Command::new(npx);
    open.args(["-y", browser::PLAYWRIGHT_PACKAGE, "open", "--channel", "chrome", "--save-storage"]).arg(&state);
    if state.is_file() {
        open.arg("--load-storage").arg(&state);
    }
    open.arg(url.unwrap_or_else(start_page));
    let status = open.status().map_err(|err| Failure::new(format!("Couldn't open the browser: {err}")))?;
    let _ = std::fs::set_permissions(&state, std::os::unix::fs::PermissionsExt::from_mode(0o600));
    if !status.success() {
        return Err(Failure::new("The browser closed with an error — needs Google Chrome installed.").next("equipe browser"));
    }
    let sites = browser::sites(&std::fs::read_to_string(&state).unwrap_or_default());
    println!();
    if sites.is_empty() {
        println!("{}", p.note(2, "Nothing signed in yet. Run equipe browser again whenever you're ready."));
    } else {
        println!("{}", p.ok_line(&format!("Members can work signed in on: {}", sites.join(", "))));
        println!("{}", p.note(2, "equipe browser --forget signs them out everywhere."));
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
        "<!doctype html><meta charset=utf-8><title>equipe · sign in</title>\
<body style='font:16px -apple-system,system-ui,sans-serif;max-width:560px;margin:64px auto;line-height:1.6;color:#111'>\
<h1 style='font-size:22px'>equipe<span style='color:#8f9aa8'>.</span> sign in for your squad</h1>\
<p>Sign in, or sign up, wherever your squad should work. Open as many tabs as you like; close the window when \
you're done.</p><ul>{links}</ul><p style='color:#666'>Members work there signed in as you. They ask you before \
creating accounts, accepting terms, paying, or deleting anything.</p>"
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
        .map_err(|err| Failure::new(format!("Couldn't tell where equipe is installed: {err}")))?;
    if update_by_homebrew(&exe) {
        println!("{}", p.note(0, "equipe came from Homebrew, so Homebrew updates it:"));
        println!("  {}", p.hint("brew upgrade equipe"));
        return Ok(());
    }
    let dir = exe.parent().ok_or_else(|| Failure::new("Couldn't tell where equipe is installed."))?;
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("curl -fsSL {INSTALL_SCRIPT} | sh"))
        .env("EQUIPE_INSTALL_DIR", dir)
        .env("EQUIPE_NO_MODIFY_PATH", "1")
        .status()
        .map_err(|err| Failure::new(format!("Couldn't run the installer: {err}")))?;
    if status.success() {
        // The squad keeps running the old build until the daemon restarts.
        if equipe_core::gateway::is_running(&daemon::socket()).await {
            println!("{}", p.note(2, "Your squad still runs the old build. This restarts it once no one is mid-turn:"));
            println!("  {}", p.hint("equipe daemon restart --when-idle"));
        }
        Ok(())
    } else {
        Err(Failure::new("The update didn't finish — equipe is unchanged.").next(format!("curl -fsSL {INSTALL_SCRIPT} | sh")))
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
        let busy = equipe_core::gateway::busy(&daemon::socket()).await.map_err(|err| Failure::new(err.to_string()))?;
        let Some(busy) = busy.filter(|b| !b.idle()) else {
            return Ok(());
        };
        let mut waiting = busy.working.clone();
        if busy.approvals > 0 {
            waiting.push(format!("{} request{} for you", busy.approvals, if busy.approvals == 1 { "" } else { "s" }));
        }
        let line = format!("Waiting for {} — Ctrl-C to leave it running", waiting.join(", "));
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
            .map_err(|err| Failure::new(format!("The daemon stopped: {err}"))),
        DaemonAction::Start => match daemon::ensure_running().await {
            Ok(true) => {
                println!("{}", p.ok_line("Daemon started. The team lives here now; terminals come and go."));
                Ok(())
            }
            Ok(false) => {
                println!("{}", p.ok_line("Daemon already running."));
                Ok(())
            }
            Err(err) => Err(Failure::new(err.to_string())),
        },
        DaemonAction::Stop => match daemon::stop().await {
            Ok(true) => {
                println!("{}", p.ok_line("Daemon stopped. Open sessions resume when it starts again."));
                Ok(())
            }
            Ok(false) => {
                println!("{}", p.note(0, "No daemon was running."));
                Ok(())
            }
            Err(err) => Err(Failure::new(err.to_string())),
        },
        DaemonAction::Restart { when_idle } => {
            if when_idle {
                wait_until_idle(&p).await?;
            }
            let _ = daemon::stop().await;
            daemon::ensure_running().await.map_err(|err| Failure::new(err.to_string()))?;
            println!("{}", p.ok_line("Daemon restarted on this build."));
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
                    println!("{}", p.ok_line("The daemon runs as a login service: it starts when you log in and comes back if it crashes."));
                    println!("{}", p.note(2, "equipe daemon uninstall goes back to starting it on demand."));
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(Failure::new("The service was installed but the daemon didn't answer.").next(format!("tail {}", daemon::log_path().display())))
        }
        DaemonAction::Uninstall => {
            if daemon::service::uninstall().map_err(|err| Failure::new(err.to_string()))? {
                println!("{}", p.ok_line("No longer a login service. equipe starts the daemon when it needs it."));
            } else {
                println!("{}", p.note(0, "The daemon wasn't installed as a service."));
            }
            Ok(())
        }
        DaemonAction::Status => {
            if equipe_core::gateway::is_running(&daemon::socket()).await {
                println!("{}", p.ok_line("Daemon running."));
                println!("{}", p.row("socket", &daemon::socket().display().to_string()));
                let how = if daemon::service::installed() { "login service — starts at login, back after a crash" } else { "on demand · equipe daemon install keeps it up" };
                println!("{}", p.row("runs as", how));
                if let Ok(Some(busy)) = equipe_core::gateway::busy(&daemon::socket()).await {
                    let working = if busy.working.is_empty() { "nobody mid-turn".to_owned() } else { busy.working.join(", ") };
                    println!("{}", p.row("working", &working));
                }
            } else {
                println!("{}", p.note(0, "Daemon not running. `equipe` starts it when you open the console."));
            }
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
                    println!("{}", p.note(0, "Cancelled."));
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
                Engine::Codex => match self.store.load("openai") {
                    Ok(Some(cred)) if !again && !chosen => {
                        let account = cred.account_id.map(|a| format!(" · {a}")).unwrap_or_default();
                        println!("{}", p.ok_line(&format!("codex · already signed in{account}")));
                        println!("{}", p.note(2, "Sign in again: equipe login codex --again"));
                    }
                    _ => self.login_codex(timeout, no_browser).await?,
                },
                Engine::Claude => match claude_code::auth_status(claude_code::DEFAULT_BINARY).await {
                    Ok(Some(status)) if status.logged_in && !again => {
                        println!("{}", p.ok_line(&format!("claude code · already signed in · {}", describe_claude_login(&status))));
                        println!("{}", p.note(2, "Sign in again: equipe login claude --again (changes Claude Code's login everywhere)"));
                    }
                    _ => self.login_claude().await?,
                },
            }
        }
        Ok(())
    }

    /// Ask which engine to sign in to, showing where each one stands.
    async fn pick_engine(&self) -> Option<Engine> {
        let codex_ready = matches!(self.store.load("openai"), Ok(Some(_)));
        let claude = claude_code::auth_status(claude_code::DEFAULT_BINARY).await.ok().flatten();
        let claude_ready = claude.as_ref().is_some_and(|s| s.logged_in);

        // One column per fact — engine, state, plan — so the rows compare at a glance.
        let e = ui::err();
        let state = |ready: bool, missing: &str| {
            if ready {
                format!("{} {:<15}", e.accent("✓"), "signed in")
            } else {
                format!("{} {:<15}", e.dim("○"), missing)
            }
        };
        let claude_plan = match &claude {
            Some(s) if s.logged_in => plan_label(s),
            _ => "your claude".to_owned(),
        };
        let items = [
            format!("{:<13}{}{}", "codex", state(codex_ready, "not signed in"), e.dim("ChatGPT plan")),
            format!(
                "{:<13}{}{}",
                "claude code",
                state(claude_ready, if claude.is_none() { "not installed" } else { "not signed in" }),
                e.dim(&claude_plan)
            ),
        ];
        // Start on the first engine that still needs a sign-in.
        let default = if !codex_ready { 0 } else if !claude_ready { 1 } else { 0 };

        eprintln!("{}  {}\n", e.wordmark(), e.dim("sign in"));
        let picked = dialoguer::Select::with_theme(&*ui::select_theme())
            .with_prompt("Which engine?")
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
        match std::fs::remove_file(&path) {
            Ok(()) => println!("{}", p.ok_line("codex · signed out")),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => println!("{}", p.note(0, "codex · wasn't signed in")),
            Err(err) => return Err(Failure::new(format!("couldn't remove {}: {err}", path.display()))),
        }
        println!("{}", p.note(2, "Claude Code keeps its own login."));
        println!("\n  {}", p.hint("equipe login codex"));
        Ok(())
    }

    async fn login_codex(&self, timeout_min: u64, no_browser: bool) -> CmdResult {
        let p = ui::out();
        println!("{}  {}\n", p.wordmark(), p.dim("sign in · codex"));
        println!("{}", p.para(0, "Sign in with your ChatGPT subscription — your account, your plan."));
        println!("{}\n", p.note(0, "equipe introduces itself to OpenAI as equipe, never as Codex CLI."));

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
                    println!("{}", p.ok_line("Opened the sign-in page in your browser."));
                    println!("{}", p.note(2, "Not there? Press Enter to print the link."));
                }
                Err(reason) => {
                    if !no_browser {
                        println!("{}", p.fail_line(&format!("Couldn't open a browser: {reason}")));
                    }
                    // Alone on its line and unindented, so it copies cleanly.
                    println!("Open this link to sign in:\n");
                    println!("{url}\n");
                }
            }
            println!("\n{}", p.note(0, &format!("Waiting for the browser ({timeout_min} min) · Ctrl-C to cancel")));
        })
        .await
        .map_err(|err| {
            let retry = if matches!(err, equipe_providers::Error::LoginTimeout) {
                format!("equipe login codex --timeout {}", timeout_min * 2)
            } else {
                "equipe login codex".into()
            };
            Failure::new(format!("Sign-in didn't complete: {err}")).next(retry)
        })?;

        let account = cred.account_id.as_deref().map(|a| format!(" · {a}")).unwrap_or_default();
        println!("\n{}", p.ok_line(&format!("Signed in to ChatGPT{account}")));
        println!("{}", p.row("saved", &self.store.path("openai").display().to_string()));
        println!("\n  {}", p.hint("equipe try \"hello\""));
        Ok(())
    }

    async fn login_claude(&self) -> CmdResult {
        let p = ui::out();
        println!("{}  {}\n", p.wordmark(), p.dim("sign in · claude code"));
        println!("{}", p.para(0, "Claude Code signs itself in, on Anthropic's own page."));
        println!("{}\n", p.note(0, "equipe never sees your Claude credentials."));

        let status = tokio::process::Command::new(claude_code::DEFAULT_BINARY)
            .args(["auth", "login"])
            .status()
            .await
            .map_err(|_| claude_missing())?;
        if !status.success() {
            return Err(Failure::new("Claude Code sign-in didn't complete").next("claude auth login"));
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
            return Err(Failure::new("No engine is ready yet").next("equipe login codex"));
        }
        println!("{}", p.row("ready", &p.bold(&ready.join(" · "))));
        Ok(())
    }

    /// Prints the Codex section; returns whether Codex is usable.
    async fn codex_section(&self, p: &ui::Painter) -> bool {
        println!("{}", p.section("codex", "ChatGPT plan · equipe signs in"));
        let cred = match self.codex_credentials().await {
            Ok(cred) => cred,
            Err(failure) => {
                println!("{}", p.row("login", &p.fail(&failure.message)));
                if let Some(next) = failure.next {
                    println!("{}", p.row("", &p.hint(&next)));
                }
                return false;
            }
        };
        if let Some(account) = &cred.account_id {
            println!("{}", p.row("account", account));
        }
        if let Some(residency) = &cred.residency {
            println!("{}", p.row("residency", residency));
        }
        if let Some(exp) = cred.expires {
            let left = (exp - now_secs()) / 60;
            println!("{}", p.row("token", &format!("valid · {left} min left")));
        }
        match codex::list_models(&self.client, &cred, VERSION).await {
            Ok(models) => {
                let shown = models.iter().take(4).cloned().collect::<Vec<_>>().join(", ");
                let more = if models.len() > 4 { ", …" } else { "" };
                println!("{}", p.row("backend", &p.ok(&format!("{} models  {}", models.len(), p.dim(&format!("{shown}{more}"))))));
                true
            }
            Err(err) => {
                println!("{}", p.row("backend", &p.fail(&err.to_string())));
                false
            }
        }
    }

    /// Prints the Claude Code section; returns whether Claude Code is usable.
    async fn claude_section(&self, p: &ui::Painter) -> bool {
        println!("{}", p.section("claude code", "your claude · signs itself in"));
        match claude_code::auth_status(claude_code::DEFAULT_BINARY).await {
            Ok(Some(status)) if status.logged_in => {
                println!("{}", p.row("login", &p.ok(&describe_claude_login(&status))));
                true
            }
            Ok(Some(_)) => {
                println!("{}", p.row("login", &p.fail("not signed in")));
                println!("{}", p.row("", &p.hint("equipe login claude")));
                false
            }
            Ok(None) => {
                println!("{}", p.row("install", &p.fail("`claude` not found on PATH")));
                println!("{}", p.row("", &p.dim("Install Claude Code, then run `equipe login claude`.")));
                false
            }
            Err(err) => {
                println!("{}", p.row("login", &p.fail(&err.to_string())));
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
            .map_err(|err| Failure::new(format!("couldn't read {}: {err}", path.display())).next("equipe login codex"))?
            .filter(|c| c.method == AuthMethod::OauthSubscription)
            .ok_or_else(|| Failure::new("not signed in").next("equipe login codex"))?;
        codex::ensure_fresh(&self.client, &self.store, cred)
            .await
            .map_err(|err| Failure::new(format!("couldn't refresh the login: {err}")).next("equipe login codex"))
    }

    // ── try ──

    /// One turn, as a client of the runtime: send commands, render events.
    async fn run_try(&self, prompt: &str, engine: Option<Engine>, model: Option<&str>, edit: bool) -> CmdResult {
        if prompt.trim().is_empty() {
            return Err(Failure::new("try needs a prompt").next("equipe try \"summarize this repo\""));
        }
        let cwd = std::env::current_dir().map_err(|err| Failure::new(err.to_string()))?;
        let access = if edit { protocol::Access::EditFiles } else { protocol::Access::ReadOnly };
        // An edit needs Claude Code for now; don't let auto-selection pick Codex.
        let engine = engine.map(protocol::Engine::from).or(edit.then_some(protocol::Engine::Claude));

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
                            eprintln!("{}", e.dim(&format!("{}{model} · may edit files", engine.label())));
                            let base = &ws.base[..ws.base.len().min(7)];
                            eprintln!("{}", e.row("branch", &format!("{}  {}", e.bold(&ws.branch), e.dim(&format!("from {base}")))));
                            eprintln!("{}\n", e.row("", &e.dim("Your checkout stays untouched.")));
                            printer = Printer::new(&ws.path);
                        }
                        None => eprintln!("{}\n", e.dim(&format!("{}{model}", engine.label()))),
                    }
                    core.send(Cmd::Submit { session: ID, text: prompt.to_owned() });
                }
                Event::TextDelta { text, .. } => printer.text(&text),
                Event::ToolUse { tool, target, .. } => printer.tool(&tool, target),
                Event::ModelKnown { .. } | Event::UserMessage { .. } => {}
                Event::TurnEnded { outcome, usage, .. } => {
                    printer.end(outcome, usage);
                    core.send(Cmd::Close { session: ID });
                }
                Event::BranchLanded { branch, stat, .. } => {
                    println!("\n{}", p.ok_line(&format!("Changes committed to {}", p.bold(&branch))));
                    for line in stat.lines() {
                        println!("  {}", p.dim(line.trim()));
                    }
                    println!();
                    println!("{}", p.row("review", &p.bold(&format!("git show {branch}"))));
                    println!("{}", p.row("keep", &p.bold(&format!("git merge {branch}"))));
                    println!("{}", p.row("discard", &p.bold(&format!("git branch -D {branch}"))));
                }
                Event::NothingChanged { .. } => println!("\n{}", p.note(0, "No files changed, so no branch was kept.")),
                Event::SessionClosed { .. } => break,
                // One prompt, one turn: no memos, no squad, no board.
                Event::MessageQueued { .. }
                | Event::Notice { .. }
                | Event::WorkStarted { .. }
                | Event::WorkEnded { .. }
                | Event::TicketChanged { .. }
                | Event::ModeChanged { .. }
                | Event::ApprovalRequested { .. }
                | Event::ApprovalResolved { .. } => {}
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
        format!("{plan} plan")
    } else {
        "billed per token".to_owned()
    }
}

fn describe_claude_login(status: &AuthStatus) -> String {
    if status.uses_subscription() {
        let plan = status.subscription_type.as_deref().unwrap_or("claude.ai");
        format!("claude.ai · {plan} subscription")
    } else {
        // Billed per token, not against a subscription — worth saying out loud.
        let how = status.auth_method.as_deref().unwrap_or("signed in");
        format!("{how} · billed per token")
    }
}

fn claude_missing() -> Failure {
    Failure::new("Claude Code isn't installed (no `claude` on PATH)").next("see https://code.claude.com/docs/en/setup")
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
        eprintln!("{mark} {}", e.dim(&format!("{verb:<6} {target}")));
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
            TurnOutcome::Incomplete => footer.push("ended early".to_owned()),
            TurnOutcome::Interrupted => footer.push("interrupted".to_owned()),
        }
        if let Some(usage) = usage {
            let n = |v: Option<u64>| v.map_or("?".into(), |v| v.to_string());
            footer.push(format!("{} in · {} out tokens", n(usage.input_tokens), n(usage.output_tokens)));
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
}
