//! `equipe` — the command-line entry point.
//!
//! Two engines, both on the user's own plans:
//! - **codex**: equipe signs in to the ChatGPT (Codex) subscription itself.
//! - **claude**: the user's installed Claude Code, which signs itself in. equipe
//!   never sees Claude credentials (see `equipe_providers::claude_code`).

mod ui;

use std::io::{IsTerminal as _, Write as _};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use equipe_providers::claude_code::{self, AuthStatus};
use equipe_providers::openai::{codex, login, responses};
use equipe_providers::turn::TurnResult;
use equipe_providers::{AuthMethod, CredentialStore, Credentials};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Built-in squad roles. A project rarely needs more than these.
const ROLES: [(&str, &str); 6] = [
    ("po", "product owner — turns intent into specs and tickets"),
    ("designer", "UX/UI — flows, wireframes, design tokens"),
    ("frontend", "frontend engineer"),
    ("backend", "backend engineer"),
    ("qa", "quality — tests, review, acceptance"),
    ("devops", "build, deploy, infra"),
];

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
    /// Sign in to an engine
    Login {
        #[arg(value_enum, default_value = "codex")]
        engine: Engine,
        /// Minutes to wait for the browser (codex)
        #[arg(long, default_value_t = 10)]
        timeout: u64,
        /// Print the sign-in link instead of opening a browser (codex)
        #[arg(long)]
        no_browser: bool,
    },
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
        Command::Login { engine: Engine::Codex, timeout, no_browser } => app.login_codex(timeout, no_browser).await,
        Command::Login { engine: Engine::Claude, .. } => app.login_claude().await,
        Command::Status => app.status().await,
        Command::Try { prompt, engine, model } => app.run_try(&prompt, engine, model.as_deref()).await,
        Command::Roles => {
            let p = ui::out();
            println!("{}\n", p.kicker("squad roles"));
            for (id, what) in ROLES {
                println!("  {}  {}", p.bold(&format!("{id:<9}")), p.dim(what));
            }
            Ok(())
        }
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            let p = ui::err();
            eprintln!("\n{}", p.fail(&failure.message));
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

    async fn login_codex(&self, timeout_min: u64, no_browser: bool) -> CmdResult {
        let p = ui::out();
        println!("{}  {}\n", p.wordmark(), p.dim("sign in · codex"));
        println!("Sign in with your ChatGPT subscription. This uses {} account;", p.bold("your"));
        println!("{}\n", p.dim("equipe identifies itself to OpenAI as itself, never as Codex CLI."));

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
                    println!("{}", p.ok("Opened the sign-in page in your browser."));
                    println!("  {}", p.dim("Don't see it? Press Enter to print the link."));
                }
                Err(reason) => {
                    if !no_browser {
                        println!("{}", p.fail(&format!("Couldn't open a browser: {reason}")));
                    }
                    // Alone on its line and unindented, so it copies cleanly.
                    println!("Open this link to sign in:\n");
                    println!("{url}\n");
                }
            }
            println!("\n{}", p.dim(&format!("Waiting for the browser · up to {timeout_min} min · Ctrl-C to cancel")));
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
        println!("\n{}", p.ok(&format!("Signed in to ChatGPT{account}")));
        println!("{}", p.row("saved", &self.store.path("openai").display().to_string()));
        println!("\n  {}", p.hint("equipe try \"hello\""));
        Ok(())
    }

    async fn login_claude(&self) -> CmdResult {
        let p = ui::out();
        println!("{}  {}\n", p.wordmark(), p.dim("sign in · claude code"));
        println!("Claude Code signs itself in, through Anthropic's own page.");
        println!("{}\n", p.dim("equipe never reads, stores or forwards your Claude credentials."));

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
        println!("{}  {}", p.kicker("codex"), p.dim("ChatGPT subscription · signed in by equipe"));
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
        println!("{}  {}", p.kicker("claude code"), p.dim("your installed claude · signs itself in"));
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

    async fn run_try(&self, prompt: &str, engine: Option<Engine>, model: Option<&str>) -> CmdResult {
        if prompt.trim().is_empty() {
            return Err(Failure::new("try needs a prompt").next("equipe try \"summarize this repo\""));
        }
        let engine = match engine {
            Some(engine) => engine,
            None => self.first_ready_engine().await?,
        };

        let e = ui::err();
        let mut streamed = false;
        let mut on_delta = |delta: &str| {
            streamed = true;
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(delta.as_bytes());
            let _ = out.flush();
        };

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
                responses::run_turn(&self.client, &cred, VERSION, request, &mut on_delta).await?
            }
            Engine::Claude => {
                let label = model.map(|m| format!("{} · {m}", engine.label())).unwrap_or_else(|| engine.label().to_owned());
                eprintln!("{}\n", e.dim(&label));
                let cwd = std::env::current_dir().map_err(|err| Failure::new(err.to_string()))?;
                claude_code::run_turn(claude_code::DEFAULT_BINARY, &cwd, prompt, model, &mut on_delta)
                    .await
                    .map_err(|err| {
                        let failure = Failure::from(err);
                        if failure.message.contains("not installed") || failure.message.to_lowercase().contains("login") {
                            failure.next("equipe login claude")
                        } else {
                            failure
                        }
                    })?
            }
        };

        // A turn that sent only a terminal event streamed nothing — print it now.
        if !streamed {
            print!("{}", result.text);
        }
        println!();

        let mut footer = Vec::new();
        if result.status != "completed" {
            footer.push(format!("ended {}", result.status));
        }
        if let Some(usage) = result.usage {
            let n = |v: Option<u64>| v.map_or("?".into(), |v| v.to_string());
            footer.push(format!("{} in · {} out tokens", n(usage.input_tokens), n(usage.output_tokens)));
        }
        if !footer.is_empty() {
            eprintln!("\n{}", e.dim(&footer.join(" · ")));
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
