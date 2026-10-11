//! `equipe` — the command-line entry point.
//!
//! Two engines, both on the user's own plans:
//! - **codex**: equipe signs in to the ChatGPT (Codex) subscription itself.
//! - **claude**: the user's installed Claude Code, which signs itself in. equipe
//!   never sees Claude credentials (see `equipe_providers::claude_code`).

mod ui;
mod worktree;

use std::io::{IsTerminal as _, Write as _};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use equipe_providers::claude_code::{self, Access, Activity, AuthStatus};
use equipe_providers::openai::{codex, login, responses};
use equipe_providers::turn::TurnResult;
use equipe_providers::{AuthMethod, CredentialStore, Credentials};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name = "equipe", version, about = "A full software squad of AI agents.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Engine {
    /// ChatGPT (Codex) subscription, signed in by equipe
    Codex,
    /// Your installed Claude Code, signed in by Claude Code itself
    Claude,
}

impl Engine {
    fn label(self) -> &'static str {
        match self {
            Engine::Codex => "codex",
            Engine::Claude => "claude code",
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Sign in to an engine, or to every engine that isn't ready
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
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let app = App {
        store: CredentialStore::default_location(),
        client: reqwest::Client::new(),
    };

    let outcome = match cli.command {
        Command::Login { engine, timeout, no_browser, again } => app.login(engine, timeout, no_browser, again).await,
        Command::Logout => app.logout(),
        Command::Status => app.status().await,
        Command::Try { prompt, engine, model, edit } => app.run_try(&prompt, engine, model.as_deref(), edit).await,
        Command::Roles => {
            let p = ui::out();
            println!("{}\n", p.kicker("squad roles"));
            for role in equipe_core::ROLES {
                println!("  {}  {}", p.bold(&format!("{:<9}", role.id)), p.dim(role.summary));
            }
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

    async fn run_try(&self, prompt: &str, engine: Option<Engine>, model: Option<&str>, edit: bool) -> CmdResult {
        if prompt.trim().is_empty() {
            return Err(Failure::new("try needs a prompt").next("equipe try \"summarize this repo\""));
        }
        if edit {
            return match engine {
                Some(Engine::Codex) => Err(Failure::new(
                    "--edit runs on Claude Code for now; Codex edits arrive with its ACP engine",
                )
                .next("equipe try --edit --engine claude \"…\"")),
                _ => self.run_edit(prompt, model).await,
            };
        }
        let engine = match engine {
            Some(engine) => engine,
            None => self.first_ready_engine().await?,
        };

        let e = ui::err();
        let cwd = std::env::current_dir().map_err(|err| Failure::new(err.to_string()))?;
        let mut printer = Printer::new(&cwd);

        let result: TurnResult = match engine {
            Engine::Codex => {
                let cred = self.codex_credentials().await?;
                // Ask the backend what this account has rather than hardcoding an id.
                let model = match model {
                    Some(model) => model.to_owned(),
                    None => codex::list_models(&self.client, &cred, VERSION)
                        .await?
                        .into_iter()
                        .next()
                        .ok_or_else(|| Failure::new("the backend offered no models").next("equipe try --model <id> \"…\""))?,
                };
                eprintln!("{}\n", e.dim(&format!("{} · {model}", engine.label())));
                let request = responses::TurnRequest { prompt, model: &model, instructions: None };
                responses::run_turn(&self.client, &cred, VERSION, request, |d| printer.text(d)).await?
            }
            Engine::Claude => {
                let label = model.map(|m| format!("{} · {m}", engine.label())).unwrap_or_else(|| engine.label().to_owned());
                eprintln!("{}\n", e.dim(&label));
                claude_code::run_turn(claude_code::DEFAULT_BINARY, &cwd, prompt, model, Access::ReadOnly, |a| printer.activity(a))
                    .await
                    .map_err(claude_failure)?
            }
        };
        printer.finish(&result);
        Ok(())
    }

    /// One turn that may edit files: on a new branch, in a worktree, through Claude Code.
    async fn run_edit(&self, prompt: &str, model: Option<&str>) -> CmdResult {
        let e = ui::err();
        match claude_code::auth_status(claude_code::DEFAULT_BINARY).await {
            Ok(Some(status)) if status.logged_in => {}
            Ok(None) => return Err(claude_missing()),
            _ => return Err(Failure::new("Claude Code isn't signed in").next("equipe login claude")),
        }

        let cwd = std::env::current_dir().map_err(|err| Failure::new(err.to_string()))?;
        let repo = worktree::repo_root(&cwd).map_err(Failure::new)?;
        let id = format!("{:x}", now_secs());
        let sandbox = worktree::create(&repo, &equipe_providers::credentials::equipe_home(), &id).map_err(Failure::new)?;

        let short_base = &sandbox.base[..sandbox.base.len().min(7)];
        let model_label = model.map(|m| format!(" · {m}")).unwrap_or_default();
        eprintln!("{}", e.dim(&format!("claude code{model_label} · may edit files")));
        eprintln!("{}", e.row("branch", &format!("{}  {}", e.bold(&sandbox.branch), e.dim(&format!("from {short_base}")))));
        eprintln!("{}\n", e.row("", &e.dim("Your checkout stays untouched.")));

        let mut printer = Printer::new(&sandbox.path);
        let outcome = claude_code::run_turn(
            claude_code::DEFAULT_BINARY,
            &sandbox.path,
            prompt,
            model,
            Access::EditFiles,
            |a| printer.activity(a),
        )
        .await;

        let subject: String = prompt.lines().next().unwrap_or("").chars().take(60).collect();
        let result = match outcome {
            Ok(result) => result,
            Err(err) => {
                // Keep whatever the agent managed to change before it stopped.
                let kept = sandbox
                    .commit_all(&format!("equipe try (incomplete): {subject}"))
                    .ok()
                    .flatten();
                let _ = if kept.is_some() { sandbox.remove_worktree() } else { sandbox.discard() };
                let failure = claude_failure(err);
                return Err(match kept {
                    Some(_) => Failure::new(format!("{} — partial changes kept on {}", failure.message, sandbox.branch)),
                    None => failure,
                });
            }
        };
        printer.finish(&result);

        let message = format!("equipe try: {subject}\n\nPrompt:\n{prompt}\n\nRun by equipe through Claude Code.");
        let committed = sandbox.commit_all(&message).map_err(Failure::new)?;
        let p = ui::out();
        match committed {
            None => {
                sandbox.discard().map_err(Failure::new)?;
                println!("\n{}", p.note(0, "No files changed, so no branch was kept."));
            }
            Some(_) => {
                let stat = sandbox.diff_stat().unwrap_or_default();
                sandbox.remove_worktree().map_err(Failure::new)?;
                println!("\n{}", p.ok_line(&format!("Changes committed to {}", p.bold(&sandbox.branch))));
                for line in stat.lines() {
                    println!("  {}", p.dim(line.trim()));
                }
                println!();
                println!("{}", p.row("review", &p.bold(&format!("git show {}", sandbox.branch))));
                println!("{}", p.row("keep", &p.bold(&format!("git merge {}", sandbox.branch))));
                println!("{}", p.row("discard", &p.bold(&format!("git branch -D {}", sandbox.branch))));
            }
        }
        Ok(())
    }

    /// Codex if signed in, else Claude Code if signed in.
    async fn first_ready_engine(&self) -> Result<Engine, Failure> {
        if matches!(self.store.load("openai"), Ok(Some(_))) {
            return Ok(Engine::Codex);
        }
        if let Ok(Some(status)) = claude_code::auth_status(claude_code::DEFAULT_BINARY).await
            && status.logged_in
        {
            return Ok(Engine::Claude);
        }
        Err(Failure::new("No engine is ready. Sign in to Codex or Claude Code first.").next("equipe status"))
    }
}

/// "Max plan", "Pro plan" — or how a non-subscription login is billed.
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

/// Point Claude Code failures at the command that fixes them.
fn claude_failure(err: equipe_providers::Error) -> Failure {
    let failure = Failure::from(err);
    let lower = failure.message.to_lowercase();
    if lower.contains("not installed") {
        failure.next("see https://code.claude.com/docs/en/setup")
    } else if lower.contains("login") || lower.contains("logged in") {
        failure.next("equipe login claude")
    } else {
        failure
    }
}

/// Streams a turn to the terminal: the answer on stdout, what the agent is doing
/// as dim lines on stderr, so `equipe try … > answer.txt` keeps only the answer.
struct Printer {
    base: std::path::PathBuf,
    streamed: bool,
    mid_line: bool,
}

impl Printer {
    fn new(base: &std::path::Path) -> Self {
        Self {
            // Canonical, so tool paths under a symlinked temp dir still shorten.
            base: base.canonicalize().unwrap_or_else(|_| base.to_owned()),
            streamed: false,
            mid_line: false,
        }
    }

    fn text(&mut self, text: &str) {
        self.streamed = true;
        self.mid_line = !text.ends_with('\n');
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }

    fn activity(&mut self, activity: Activity) {
        match activity {
            Activity::Text(text) => self.text(&text),
            Activity::Tool { name, target } => {
                if self.mid_line {
                    println!();
                    self.mid_line = false;
                }
                let verb = match name.as_str() {
                    "Read" => "read",
                    "Edit" | "MultiEdit" | "NotebookEdit" => "edit",
                    "Write" => "write",
                    "Bash" => "run",
                    "Glob" | "Grep" => "search",
                    "WebFetch" | "WebSearch" => "fetch",
                    other => other,
                };
                let target = target.map(|t| self.relative(&t)).unwrap_or_default();
                let e = ui::err();
                let mark = if matches!(verb, "edit" | "write") { e.accent("✎") } else { e.dim("·") };
                eprintln!("{mark} {}", e.dim(&format!("{verb:<6} {target}")));
            }
        }
    }

    fn relative(&self, target: &str) -> String {
        let path = std::path::Path::new(target);
        let path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        path.strip_prefix(&self.base)
            .map(|rel| rel.display().to_string())
            .unwrap_or_else(|_| target.to_owned())
    }

    /// Close the answer and print the footer.
    fn finish(&mut self, result: &TurnResult) {
        // A turn that sent only a terminal event streamed nothing — print it now.
        if !self.streamed {
            print!("{}", result.text);
            self.mid_line = !result.text.ends_with('\n');
        }
        if self.mid_line || !self.streamed {
            println!();
        }
        let e = ui::err();
        let mut footer = Vec::new();
        if result.status != "completed" {
            footer.push(format!("ended {}", result.status));
        }
        if let Some(usage) = &result.usage {
            let n = |v: Option<u64>| v.map_or("?".into(), |v| v.to_string());
            footer.push(format!("{} in · {} out tokens", n(usage.input_tokens), n(usage.output_tokens)));
        }
        if !footer.is_empty() {
            eprintln!("\n{}", e.dim(&footer.join(" · ")));
        }
    }
}
