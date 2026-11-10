//! `equipe design refs | palette | type | component`: the designer's tools for
//! references and taste (`equipe_core::design::{refs, palette, fonts,
//! component}`). Pages are drawn by `assets/design/refs.mjs` in the Mac's
//! own Chrome; robots.txt, a registry's or a font's facts are fetched here.
//! Every read is of a public page, a few per call, paced, as a person reads
//! them; nothing signs in, holds a key or answers a check for a person.

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, Subcommand};
use equipe_core::design::{component, fonts, palette, refs};
use equipe_core::i18n::Lang;
use equipe_core::t;
use serde::Deserialize;
use serde_json::json;

#[derive(Subcommand)]
pub enum RefsAction {
    /// The sources references come from, how each is read, and whether it can be now
    Sources,
    /// Where a query goes on a source; for 21st.dev, the components it lists
    Search {
        /// lapa, mobbin, fontpair, 21st, google-fonts
        source: String,
        /// What to look for: a category, a pattern, a font, a use
        query: String,
        /// How many results to list
        #[arg(long, default_value_t = 12)]
        limit: usize,
    },
    /// Shoot public pages as references, each with one line of what to learn from it
    Shoot(ShootArgs),
    /// Record a screenshot you took in your own browser as a reference
    Add {
        /// The picture, saved in your worktree
        picture: PathBuf,
        /// The page it shows
        #[arg(long)]
        url: String,
        /// One line: what to learn from it — a pattern, a layout, a rhythm, type or colour
        #[arg(long)]
        learn: String,
        /// pattern, layout, rhythm, type, colour (comma-separated)
        #[arg(long)]
        aspect: String,
        #[arg(long)]
        ticket: Option<String>,
        /// The design folder (default: ./design)
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Draw the board as one contact sheet, and say what it still needs
    Board {
        /// The brief, a line, shown at the top
        #[arg(long)]
        brief: Option<String>,
        #[arg(long)]
        dir: Option<PathBuf>,
        /// The HTML only, no PNG
        #[arg(long)]
        no_png: bool,
    },
    /// The board, a line each
    List {
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

#[derive(Args)]
pub struct ShootArgs {
    /// The pages (at most 8 a call)
    #[arg(required = true)]
    pub urls: Vec<String>,
    /// What to learn from each page, one line each, in the pages' order
    #[arg(long = "learn", required = true)]
    pub learn: Vec<String>,
    /// pattern, layout, rhythm, type, colour: one for every page, or one per page
    #[arg(long = "aspect", required = true)]
    pub aspect: Vec<String>,
    /// Widths in px, comma-separated (at most 2)
    #[arg(long, default_value = "1280")]
    pub widths: String,
    /// The whole page, not its first screen (cut at 6000 px)
    #[arg(long)]
    pub full: bool,
    #[arg(long)]
    pub ticket: Option<String>,
    #[arg(long)]
    pub dir: Option<PathBuf>,
}

#[derive(Args)]
pub struct PaletteArgs {
    /// One to four palettes: Name=<text>-<bg>-<primary>-<secondary>-<accent>[/<the same, dark>]
    #[arg(required = true)]
    pub palettes: Vec<String>,
    /// The type pair to set them in, Heading/Body (Google Fonts)
    #[arg(long)]
    pub fonts: Option<String>,
    /// The page being designed (a file, or a local address), each palette drawn on it light and dark (default: ./index.html when there is one; else a page of our own)
    #[arg(long)]
    pub page: Option<String>,
    #[arg(long)]
    pub dir: Option<PathBuf>,
    #[arg(long)]
    pub no_png: bool,
}

#[derive(Args)]
pub struct TypeArgs {
    /// One to four pairs, Heading/Body, as Google Fonts names them (Fraunces/Inter)
    #[arg(required = true)]
    pub pairs: Vec<String>,
    /// The product's languages, comma-separated (en,ko,ja …, or all); default: yours and the brand's
    #[arg(long)]
    pub locales: Option<String>,
    #[arg(long)]
    pub dir: Option<PathBuf>,
    #[arg(long)]
    pub no_png: bool,
}

#[derive(Args)]
pub struct ComponentArgs {
    /// The component: its 21st.dev page, a public registry item's address, or the `npx shadcn add <url>` line
    pub source: String,
    /// Where its files go (default: <design>/vendor/<name>)
    #[arg(long)]
    pub out: Option<PathBuf>,
    #[arg(long)]
    pub dir: Option<PathBuf>,
}

fn design_dir(dir: Option<PathBuf>) -> PathBuf {
    dir.unwrap_or_else(|| PathBuf::from(equipe_core::design::DIR))
}

fn today() -> String {
    equipe_core::squad_memory::date(equipe_core::design::lint::record::now())
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("equipe/", env!("CARGO_PKG_VERSION"), " (design references)"))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}

/// These tools read the web and draw in Chrome, neither of which an
/// engine's sandbox allows: said at once, rather than after a failed read.
fn outside_sandbox(env: &dyn Fn(&str) -> Option<String>) -> Result<(), String> {
    let set = |name: &str| env(name).is_some_and(|v| !v.trim().is_empty());
    if set("CODEX_SANDBOX") || set("CODEX_SANDBOX_NETWORK_DISABLED") { Err(t!("design.taste.sandboxed").to_owned()) } else { Ok(()) }
}

fn not_sandboxed() -> Result<(), String> {
    outside_sandbox(&|name| std::env::var(name).ok())
}

/// Set when a member's board server runs these tools: only the listed
/// reference sites are read.
const LISTED_ONLY: &str = "EQUIPE_DESIGN_LISTED_SITES";

fn listed_only() -> bool {
    std::env::var_os(LISTED_ONLY).is_some_and(|v| !v.is_empty())
}

/// Whether `url` may be read when only the listed sites are: a reference
/// source's own, or (for a component) shadcn/ui's public registry.
fn listed(url: &str) -> bool {
    refs::source_of(url).is_some() || refs::site(url) == "ui.shadcn.com"
}

fn listed_sites() -> String {
    refs::SOURCES.iter().filter(|s| s.via == refs::Via::Shoot).flat_map(|s| s.hosts.iter().copied()).chain(["ui.shadcn.com"]).collect::<Vec<_>>().join(", ")
}

/// The words a board call may run: `equipe design` and one of its taste
/// commands, never a folder of its own choosing (what it writes stays in the
/// member's worktree).
pub(crate) fn board_args(args: &[String]) -> Result<Vec<String>, String> {
    let first = args.first().map(String::as_str).unwrap_or_default();
    if !equipe_core::board::DESIGN_TOOL_COMMANDS.contains(&first) {
        return Err(format!("design_tools runs {}: `args` starts with one of them.", equipe_core::board::DESIGN_TOOL_COMMANDS.join(", ")));
    }
    if let Some(flag) = args.iter().find(|a| ["--dir", "--out"].contains(&a.split('=').next().unwrap_or_default())) {
        return Err(format!("{flag}: design_tools writes in your worktree's design/ folder only."));
    }
    Ok(std::iter::once("design".to_owned()).chain(args.iter().cloned()).collect())
}

/// `design_tools`, for a designer's board server: `equipe design <args>` run
/// as this same equipe, in the member's worktree (asked of the daemon),
/// reading only the listed sites; what it printed is the answer.
pub(crate) async fn from_board(socket: &Path, home: &Path, squad: &str, member: &str, arguments: &serde_json::Value) -> equipe_core::protocol::rpc::ToolResult {
    use equipe_core::protocol::rpc::{ToolCall, ToolResult};
    let words: Vec<String> = arguments["args"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_owned)).collect();
    let args = match board_args(&words) {
        Ok(args) => args,
        Err(why) => return ToolResult { text: why, is_error: true, image: None },
    };
    let ask = ToolCall { squad: squad.to_owned(), member: member.to_owned(), name: equipe_core::board::WORKDIR.to_owned(), arguments: json!({}) };
    let cwd = match equipe_core::gateway::call_tool(socket, &ask).await {
        Ok(r) if !r.is_error && Path::new(r.text.trim()).is_dir() => PathBuf::from(r.text.trim()),
        Ok(r) => return ToolResult { text: r.text, is_error: true, image: None },
        Err(e) => return ToolResult { text: format!("equipe's daemon didn't answer: {e}"), is_error: true, image: None },
    };
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("equipe"));
    let run = tokio::process::Command::new(exe)
        .args(&args)
        .current_dir(&cwd)
        .env("EQUIPE_HOME", home)
        .env(LISTED_ONLY, "1")
        .env_remove("CODEX_SANDBOX")
        .env_remove("CODEX_SANDBOX_NETWORK_DISABLED")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = match tokio::time::timeout(Duration::from_secs(15 * 60), run).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return ToolResult { text: format!("equipe design: {e}"), is_error: true, image: None },
        Err(_) => return ToolResult { text: "equipe design ran for 15 minutes and was stopped: fewer pages a call.".into(), is_error: true, image: None },
    };
    let said = [String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)].join("\n");
    let said = plain(said.trim());
    // The end is what matters: what was written, and what it still needs.
    let text = match said.char_indices().rev().nth(8000) {
        Some((at, _)) => format!("…{}", &said[at..]),
        None => said,
    };
    ToolResult { text: if text.is_empty() { "done".into() } else { text }, is_error: !output.status.success(), image: None }
}

/// Text without the terminal's colour codes.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A lock on a folder while it is changed: another call of these tools on
/// it waits its turn (two `refs shoot` at once wrote the same reference id,
/// two first runs installed the drawing kit over each other). One left
/// behind by a call that was stopped is taken over after the wait.
pub(crate) struct Lock(PathBuf);

impl Lock {
    pub(crate) async fn take(path: PathBuf, wait: Duration) -> Lock {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let until = std::time::Instant::now() + wait;
        while std::fs::OpenOptions::new().write(true).create_new(true).open(&path).is_err() && std::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Lock(path)
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn board_lock(design: &Path) -> PathBuf {
    design.join(refs::DIR).join(".lock")
}

async fn get(client: &reqwest::Client, url: &str) -> Result<(u16, String), String> {
    let response = client.get(url).send().await.map_err(|e| format!("{url}: {e}"))?;
    let status = response.status().as_u16();
    Ok((status, response.text().await.unwrap_or_default()))
}

/// What robots.txt says of a page: allowed, refused, or unread (and why).
enum Robots {
    Allowed,
    Refused,
    Unread(String),
}

async fn robots(client: &reqwest::Client, url: &str) -> Robots {
    let Ok(parsed) = reqwest::Url::parse(url) else { return Robots::Unread("address".into()) };
    let origin = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or_default());
    let path = match parsed.query() {
        Some(q) => format!("{}?{q}", parsed.path()),
        None => parsed.path().to_owned(),
    };
    match get(client, &format!("{origin}/robots.txt")).await {
        Ok((200, text)) => if refs::robots_allow(&text, &path) { Robots::Allowed } else { Robots::Refused },
        // None there: nothing asked.
        Ok((404 | 410, _)) => Robots::Allowed,
        Ok((status, _)) => Robots::Unread(status.to_string()),
        Err(e) => Robots::Unread(e),
    }
}

/// One shot the drawing script takes.
fn shot(url: &str, path: &Path, width: u32, scheme: &str, full: bool) -> serde_json::Value {
    json!({ "url": url, "path": path, "width": width, "height": if width <= 480 { 844 } else { 800 }, "scheme": scheme, "full": full, "maxHeight": 6000 })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Taken {
    url: String,
    path: String,
    ok: bool,
    status: u16,
    title: String,
    final_url: String,
    error: String,
    css: Vec<String>,
    fonts: Vec<FontFile>,
    vars: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FontFile {
    url: String,
    bytes: u64,
}

/// The shots drawn by `refs.mjs`, `pace` seconds between two of one site.
async fn draw(shots: Vec<serde_json::Value>, pace: u64) -> Result<Vec<Taken>, String> {
    let work = std::env::temp_dir().join(format!("equipe-refs-{}-{}", std::process::id(), equipe_core::design::lint::record::now()));
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let (plan, result) = (work.join("plan.json"), work.join("result.json"));
    std::fs::write(&plan, json!({ "pace": pace, "shots": shots }).to_string()).map_err(|e| e.to_string())?;
    let ran = crate::design::run_script("refs.mjs", &["--plan", plan.to_string_lossy().as_ref(), "--result", result.to_string_lossy().as_ref()]).await;
    let taken = std::fs::read_to_string(&result).ok().and_then(|t| serde_json::from_str(&t).ok());
    let _ = std::fs::remove_dir_all(&work);
    match (taken, ran) {
        (Some(taken), _) => Ok(taken),
        (None, Err(e)) => Err(e),
        (None, Ok(())) => Ok(Vec::new()),
    }
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// A local HTML page drawn to PNG beside it, `width` wide, its whole height.
async fn picture(html: &Path, width: u32, tall: u32) -> Result<PathBuf, String> {
    let png = html.with_extension("png");
    let mut one = shot(&absolute(html).to_string_lossy(), &absolute(&png), width, "light", true);
    one["maxHeight"] = json!(tall);
    let taken = draw(vec![one], 0).await?;
    match taken.first() {
        Some(t) if t.ok => Ok(png),
        Some(t) => Err(t.error.clone()),
        None => Err(html.display().to_string()),
    }
}

fn learn_says(problem: &refs::LearnProblem) -> String {
    match problem {
        refs::LearnProblem::Empty => t!("design.refs.learn_empty").to_owned(),
        refs::LearnProblem::NotOneLine => t!("design.refs.learn_long", most = refs::LEARN_CHARS),
        refs::LearnProblem::TakesBrand(words) => t!("design.refs.learn_brand", words = words),
    }
}

fn aspects_of(text: &str) -> Result<Vec<refs::Aspect>, String> {
    let aspects = refs::Aspect::parse_list(text).map_err(|word| t!("design.refs.bad_aspect", word = word))?;
    if aspects.is_empty() {
        return Err(t!("design.refs.bad_aspect", word = text));
    }
    Ok(aspects)
}

fn check_learn(learn: &str) -> Result<(), String> {
    match refs::learn_problems(learn).first() {
        Some(p) => Err(learn_says(p)),
        None => Ok(()),
    }
}

fn aspect_words(aspects: &[refs::Aspect]) -> String {
    aspects.iter().map(|a| a.id()).collect::<Vec<_>>().join(", ")
}

fn squad_sites() -> Vec<String> {
    let state = equipe_core::browser::state_file(&equipe_providers::credentials::equipe_home());
    std::fs::read_to_string(state).map(|s| equipe_core::browser::sites(&s)).unwrap_or_default()
}

fn find_source(id: &str) -> Result<&'static refs::Source, String> {
    let id = id.trim().to_ascii_lowercase();
    refs::SOURCES
        .iter()
        .find(|s| s.id == id || s.name.eq_ignore_ascii_case(&id) || s.hosts.contains(&id.as_str()))
        .ok_or_else(|| t!("design.refs.unknown_source", source = id, sources = refs::SOURCES.iter().map(|s| s.id).collect::<Vec<_>>().join(", ")))
}

/// The pages and their lines, checked before anything is read: the cap, a
/// line and aspects for each, no brand taken.
fn plan_shoot(args: &ShootArgs) -> Result<(Vec<u32>, Vec<Vec<refs::Aspect>>), String> {
    if args.urls.len() > refs::MOST_PAGES {
        return Err(t!("design.refs.too_many", most = refs::MOST_PAGES, count = args.urls.len()));
    }
    if args.learn.len() != args.urls.len() {
        return Err(t!("design.refs.learn_each", pages = args.urls.len(), learns = args.learn.len()));
    }
    for line in &args.learn {
        check_learn(line)?;
    }
    let aspects: Vec<Vec<refs::Aspect>> = match args.aspect.len() {
        1 => vec![aspects_of(&args.aspect[0])?; args.urls.len()],
        n if n == args.urls.len() => args.aspect.iter().map(String::as_str).map(aspects_of).collect::<Result<_, _>>()?,
        _ => return Err(t!("design.refs.aspect_each").to_owned()),
    };
    let widths: Vec<u32> = args.widths.split(',').filter_map(|w| w.trim().parse().ok()).filter(|w| (320..=2560).contains(w)).collect();
    if widths.is_empty() || widths.len() > refs::MOST_WIDTHS {
        return Err(t!("design.refs.widths", most = refs::MOST_WIDTHS));
    }
    Ok((widths, aspects))
}

pub async fn refs_command(action: RefsAction) -> Result<(), String> {
    match action {
        RefsAction::Sources => {
            let sites = squad_sites();
            for s in &refs::SOURCES {
                let how = match s.via {
                    refs::Via::Shoot => t!("design.refs.via_shoot"),
                    refs::Via::Browser => t!("design.refs.via_browser"),
                };
                let sign = match (s.needs_sign_in, refs::signed_in(s, &sites)) {
                    (false, _) => String::new(),
                    (true, true) => format!(" · {}", t!("design.refs.signed_in")),
                    (true, false) => format!(" · {}", t!("design.refs.not_signed_in")),
                };
                println!("{} ({}) — {} · {}: {how}{sign}", s.id, s.hosts.join(", "), s.name, aspect_words(s.aspects));
            }
            Ok(())
        }
        RefsAction::Search { source, query, limit } => {
            let source = find_source(&source)?;
            if source.needs_sign_in && !refs::signed_in(source, &squad_sites()) {
                return Err(format!("{}: {}", source.name, t!("design.refs.not_signed_in")));
            }
            let urls = refs::search_urls(source, &query);
            if urls.is_empty() {
                return Err(t!("design.refs.no_search", source = source.name));
            }
            if source.via == refs::Via::Browser {
                println!("{}", t!("design.refs.open_these"));
                urls.iter().for_each(|u| println!("  {u}"));
                return Ok(());
            }
            not_sandboxed()?;
            let client = client()?;
            let mut found = 0;
            let mut refused: Vec<String> = Vec::new();
            for url in urls {
                if found >= 3 {
                    break;
                }
                if refused.contains(&refs::site(&url)) {
                    continue;
                }
                match robots(&client, &url).await {
                    Robots::Allowed => {}
                    Robots::Unread(why) if why.contains("error sending request") => {
                        // The web isn't there at all: one line, not one per page.
                        return Err(t!("design.refs.robots_unread", url = url, status = why));
                    }
                    Robots::Refused => {
                        eprintln!("{}", t!("design.refs.robots_no", url = url));
                        continue;
                    }
                    Robots::Unread(why) => {
                        eprintln!("{}", t!("design.refs.robots_unread", url = url, status = why));
                        refused.push(refs::site(&url));
                        continue;
                    }
                }
                let (status, body) = get(&client, &url).await?;
                if status != 200 {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
                let links = refs::markdown_links(&body);
                found += 1;
                if links.is_empty() {
                    println!("{}", t!("design.refs.page_to_shoot", url = url));
                } else {
                    println!("{}", t!("design.refs.found", count = links.len().min(limit), site = refs::site(&url)));
                    for (title, link) in links.into_iter().take(limit) {
                        println!("  {title} — {link}");
                    }
                }
                tokio::time::sleep(Duration::from_secs(refs::PACE_SECS)).await;
            }
            if found == 0 { Err(t!("design.refs.none_found", query = query)) } else { Ok(()) }
        }
        RefsAction::Shoot(args) => shoot(args).await,
        RefsAction::Add { picture, url, learn, aspect, ticket, dir } => {
            let design = design_dir(dir);
            check_learn(&learn)?;
            let aspects = aspects_of(&aspect)?;
            if !picture.is_file() {
                return Err(t!("design.refs.no_picture", path = picture.display()));
            }
            let _lock = Lock::take(board_lock(&design), Duration::from_secs(600)).await;
            let mut board = refs::read(&design);
            let mut r = refs::new_reference(&board, &url, &learn, aspects, &today(), refs::Via::Browser);
            r.ticket = ticket;
            let ext = picture.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
            let rel = format!("{}/{}/browser.{ext}", refs::DIR, r.id);
            std::fs::create_dir_all(design.join(refs::DIR).join(&r.id)).map_err(|e| e.to_string())?;
            std::fs::copy(&picture, design.join(&rel)).map_err(|e| format!("{}: {e}", picture.display()))?;
            r.shots.push(rel);
            println!("{}", t!("design.refs.recorded", id = r.id, site = r.site, aspects = aspect_words(&r.aspects), learn = r.learn));
            refs::upsert(&mut board, r);
            refs::write(&design, &board).map_err(|e| format!("{}: {e}", design.display()))
        }
        RefsAction::Board { brief, dir, no_png } => {
            let design = design_dir(dir);
            let board = refs::read(&design);
            if board.is_empty() {
                return Err(t!("design.refs.empty").to_owned());
            }
            if !no_png {
                not_sandboxed()?;
            }
            let html = design.join(refs::DIR).join("board.html");
            std::fs::write(&html, refs::board_html(&board, brief.as_deref(), crate::i18n::lang())).map_err(|e| format!("{}: {e}", html.display()))?;
            println!("{}", t!("design.taste.wrote", path = html.display()));
            if !no_png {
                let rows = board.len().div_ceil(3) as u32;
                let png = picture(&html, 1440, 400 + rows * 520).await?;
                println!("{}", t!("design.taste.wrote", path = png.display()));
            }
            let gaps = refs::board_gaps(&design, &board);
            for gap in &gaps {
                println!("! {}", gap_says(gap));
            }
            if gaps.is_empty() {
                println!("{}", t!("design.refs.board_ready", count = board.len(), sources = refs::source_count(&board)));
            }
            Ok(())
        }
        RefsAction::List { dir } => {
            let board = refs::read(&design_dir(dir));
            if board.is_empty() {
                println!("{}", t!("design.refs.empty"));
            }
            for r in board {
                println!("{} · {} · {} · {} — {}\n    {}", r.id, r.site, r.date, aspect_words(&r.aspects), r.learn, r.url);
            }
            Ok(())
        }
    }
}

fn gap_says(gap: &refs::BoardGap) -> String {
    match gap {
        refs::BoardGap::FewRefs(n) => t!("design.refs.gap_refs", count = n, need = refs::BOARD_REFS),
        refs::BoardGap::FewSources(n) => t!("design.refs.gap_sources", count = n, need = refs::BOARD_SOURCES),
        refs::BoardGap::NoAspect(id) => t!("design.refs.gap_aspect", id = id),
        refs::BoardGap::NoShot(id) => t!("design.refs.gap_shot", id = id),
        refs::BoardGap::Learn(id, p) => format!("{id}: {}", learn_says(p)),
    }
}

async fn shoot(args: ShootArgs) -> Result<(), String> {
    let (widths, aspects) = plan_shoot(&args)?;
    not_sandboxed()?;
    let design = design_dir(args.dir.clone());
    let client = client()?;
    // One shoot at a time per design folder: each numbers its references from the board.
    let _lock = Lock::take(board_lock(&design), Duration::from_secs(600)).await;
    let mut board = refs::read(&design);
    let mut planned: Vec<(refs::Reference, Vec<serde_json::Value>)> = Vec::new();
    for ((url, learn), aspects) in args.urls.iter().zip(&args.learn).zip(aspects) {
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(t!("design.refs.not_a_page", url = url));
        }
        if let Some(source) = refs::source_of(url).filter(|s| s.via == refs::Via::Browser) {
            eprintln!("{}", t!("design.refs.browser_only", url = url, site = source.name));
            continue;
        }
        if listed_only() && !listed(url) {
            eprintln!("{}", t!("design.refs.not_listed", url = url, sites = listed_sites()));
            continue;
        }
        match robots(&client, url).await {
            Robots::Allowed => {}
            Robots::Refused => {
                eprintln!("{}", t!("design.refs.robots_no", url = url));
                continue;
            }
            Robots::Unread(why) => {
                eprintln!("{}", t!("design.refs.robots_unread", url = url, status = why));
                continue;
            }
        }
        let so_far: Vec<refs::Reference> = board.iter().cloned().chain(planned.iter().map(|(r, _)| r.clone())).collect();
        let mut r = refs::new_reference(&so_far, url, learn, aspects, &today(), refs::Via::Shoot);
        r.ticket = args.ticket.clone();
        let shots = widths.iter().map(|&w| shot(url, &absolute(&design.join(refs::shot_path(&r.id, w))), w, "light", args.full)).collect();
        planned.push((r, shots));
    }
    if planned.is_empty() {
        return Err(t!("design.refs.nothing_shot").to_owned());
    }
    let taken = draw(planned.iter().flat_map(|(_, s)| s.clone()).collect(), refs::PACE_SECS).await?;
    for (mut r, _) in planned {
        let mine: Vec<&Taken> = taken.iter().filter(|t| t.url == r.url).collect();
        let wall = mine.iter().find(|t| equipe_core::browser::human_check(&t.final_url, &t.title, None).is_some() || matches!(t.status, 401 | 403 | 429));
        if let Some(t) = wall {
            for t in &mine {
                let _ = std::fs::remove_file(&t.path);
            }
            eprintln!("{}", t!("design.refs.check", url = t.url));
            continue;
        }
        for t in &mine {
            if t.ok {
                let rel = Path::new(&t.path).strip_prefix(absolute(&design)).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| t.path.clone());
                r.shots.push(rel);
            } else {
                eprintln!("{}", t!("design.refs.missed", url = t.url, why = if t.error.is_empty() { t.status.to_string() } else { t.error.clone() }));
            }
        }
        if r.shots.is_empty() {
            continue;
        }
        println!("{}", t!("design.refs.recorded", id = r.id, site = r.site, aspects = aspect_words(&r.aspects), learn = r.learn));
        refs::upsert(&mut board, r);
    }
    refs::write(&design, &board).map_err(|e| format!("{}: {e}", design.display()))?;
    Ok(())
}

fn fonts_of(text: Option<&str>) -> Result<Option<fonts::Pair>, String> {
    text.map(|f| fonts::Pair::parse(f).ok_or_else(|| t!("design.palette.bad_fonts", fonts = f))).transpose()
}

pub async fn palette_command(args: PaletteArgs) -> Result<(), String> {
    if !(1..=palette::MOST).contains(&args.palettes.len()) {
        return Err(t!("design.palette.how_many", most = palette::MOST));
    }
    let palettes: Vec<palette::Palette> = args.palettes.iter().map(|p| palette::parse(p).map_err(|_| t!("design.palette.bad", spec = p))).collect::<Result<_, _>>()?;
    let pair = fonts_of(args.fonts.as_deref())?;
    if !args.no_png {
        not_sandboxed()?;
    }
    let fonts = pair.as_ref().map(|p| (p.heading.as_str(), p.body.as_str()));
    let out = design_dir(args.dir).join("palettes");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let lang = crate::i18n::lang();
    let words = equipe_core::i18n::core(lang);
    let mut failing = 0;
    for p in &palettes {
        let file = out.join(format!("{}.tokens.json", refs::slug(&p.name)));
        std::fs::write(&file, format!("{}\n", serde_json::to_string_pretty(&palette::tokens(p)).unwrap_or_default())).map_err(|e| format!("{}: {e}", file.display()))?;
        println!("{} — {}", p.name, t!("design.taste.wrote", path = file.display()));
        for (l, d) in palette::checks(&p.light).iter().zip(palette::checks(&p.dark)) {
            let mark = |c: &palette::Check| format!("{:.2}:1 {}", (c.ratio * 100.).floor() / 100., if c.passes() { "✓" } else { "✗" });
            failing += usize::from(!l.passes()) + usize::from(!d.passes());
            println!("  {}: {} {} · {} {}", words.get(&format!("design.palette.pair_{}", l.pair)), words.get("design.palette.light"), mark(l), words.get("design.palette.dark"), mark(&d));
        }
    }
    // The page being designed, when there is one: each palette drawn on its own content.
    let page = args.page.clone().or_else(|| Path::new("index.html").is_file().then(|| "index.html".to_owned()));
    let mut drawn: Vec<(String, bool, String)> = Vec::new();
    if let (Some(page), false) = (page, args.no_png) {
        let url = if page.contains("://") { page.clone() } else { absolute(Path::new(&page)).to_string_lossy().into_owned() };
        let mut look = shot(&url, &absolute(&out.join("page/.inspect.png")), 1280, "light", false);
        look["inspect"] = json!(true);
        let seen = draw(vec![look], 0).await?;
        let _ = std::fs::remove_file(out.join("page/.inspect.png"));
        match seen.first() {
            Some(t) if t.ok => {
                let mut shots = Vec::new();
                for p in &palettes {
                    for (dark, roles) in [(false, &p.light), (true, &p.dark)] {
                        let rel = format!("page/{}-{}.png", refs::slug(&p.name), if dark { "dark" } else { "light" });
                        let mut one = shot(&url, &absolute(&out.join(&rel)), 1280, if dark { "dark" } else { "light" }, true);
                        one["maxHeight"] = json!(2400);
                        one["css"] = json!(palette::page_css(&t.vars, roles));
                        shots.push(one);
                        drawn.push((p.name.clone(), dark, rel));
                    }
                }
                let taken = draw(shots, 0).await?;
                drawn.retain(|(_, _, rel)| taken.iter().any(|t| t.ok && t.path.ends_with(rel.as_str())));
            }
            Some(t) => eprintln!("{}", t!("design.palette.page_missed", page = page, why = t.error)),
            None => eprintln!("{}", t!("design.palette.page_missed", page = page, why = "?")),
        }
    }
    let shown: Vec<palette::Picture<'_>> = drawn.iter().map(|(name, dark, path)| palette::Picture { palette: name, dark: *dark, path: path.clone() }).collect();
    let html = out.join("palettes.html");
    std::fs::write(&html, palette::sheet_html(&palettes, fonts, &shown, lang)).map_err(|e| format!("{}: {e}", html.display()))?;
    println!("{}", t!("design.taste.wrote", path = html.display()));
    if !args.no_png {
        let tall = 160 + palettes.len() as u32 * if shown.is_empty() { 760 } else { 1400 };
        println!("{}", t!("design.taste.wrote", path = picture(&html, 1440, tall).await?.display()));
    }
    if failing > 0 {
        println!("! {}", t!("design.palette.fails", count = failing));
    }
    Ok(())
}

/// The languages a specimen is set in: those asked for, else the person's,
/// English, and the language the brand's own words are in.
fn locales(asked: Option<&str>, design: &Path) -> Result<Vec<Lang>, String> {
    let mut out: Vec<Lang> = Vec::new();
    let mut add = |l: Lang| {
        if !out.contains(&l) {
            out.push(l);
        }
    };
    match asked.map(str::trim) {
        Some("all") => Lang::ALL.iter().for_each(|l| add(*l)),
        Some(list) => {
            for code in list.split(',').map(str::trim).filter(|c| !c.is_empty()) {
                add(Lang::parse(code).ok_or_else(|| t!("design.type.bad_locale", code = code, codes = Lang::ALL.iter().map(|l| l.code()).collect::<Vec<_>>().join(", ")))?);
            }
        }
        None => {
            add(crate::i18n::lang());
            add(Lang::En);
            let brand = std::fs::read_to_string(design.join("brand.md")).unwrap_or_default();
            if let Some(lang) = equipe_core::language::detect(&brand).and_then(Lang::from_english_name) {
                add(lang);
            }
        }
    }
    Ok(out)
}

pub async fn type_command(args: TypeArgs) -> Result<(), String> {
    if !(1..=fonts::MOST).contains(&args.pairs.len()) {
        return Err(t!("design.type.how_many", most = fonts::MOST));
    }
    let pairs: Vec<fonts::Pair> = args.pairs.iter().map(|p| fonts::Pair::parse(p).ok_or_else(|| t!("design.type.bad_pair", pair = p))).collect::<Result<_, _>>()?;
    let design = design_dir(args.dir);
    let langs = locales(args.locales.as_deref(), &design)?;
    not_sandboxed()?;
    let lang = crate::i18n::lang();
    let out = design.join("type");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let html = out.join("specimen.html");
    let mut families: Vec<&str> = pairs.iter().flat_map(fonts::Pair::families).collect();
    families.sort_unstable();
    families.dedup();
    // A first drawing loads the fonts as a browser would: their CSS gives coverage, their files the weight.
    std::fs::write(&html, fonts::specimen_html(&pairs, &langs, &[], &|_| fonts::Facts::default(), lang)).map_err(|e| format!("{}: {e}", html.display()))?;
    let mut first = shot(&absolute(&html).to_string_lossy(), &absolute(&out.join("specimen.png")), 1440, "light", true);
    first["capture"] = json!(true);
    first["maxHeight"] = json!(9000);
    let taken = draw(vec![first], 0).await?;
    let loaded = taken.first().ok_or_else(|| html.display().to_string())?;
    let css = loaded.css.join("\n");
    let faces = fonts::faces(&css);
    let covered = fonts::coverage(&css, &pairs, &langs);
    let client = client()?;
    let mut facts: Vec<(String, fonts::Facts)> = Vec::new();
    for family in &families {
        let served: Vec<&fonts::Face> = faces.iter().filter(|f| f.family.eq_ignore_ascii_case(family)).collect();
        if served.is_empty() {
            eprintln!("! {}", t!("design.type.not_served", family = family));
        }
        let bytes: u64 = loaded.fonts.iter().filter(|f| served.iter().any(|s| s.url == f.url)).map(|f| f.bytes).sum();
        let mut licence = None;
        for (name, url) in fonts::licence_urls(family) {
            if matches!(get(&client, &url).await, Ok((200, _))) {
                licence = Some(name.to_owned());
                break;
            }
        }
        facts.push((family.to_string(), fonts::Facts { licence, bytes: (bytes > 0).then_some(bytes) }));
    }
    let fact = |family: &str| facts.iter().find(|(f, _)| f == family).map(|(_, f)| f.clone()).unwrap_or_default();
    std::fs::write(&html, fonts::specimen_html(&pairs, &langs, &covered, &fact, lang)).map_err(|e| format!("{}: {e}", html.display()))?;
    println!("{}", t!("design.taste.wrote", path = html.display()));
    if !args.no_png {
        let png = picture(&html, 1440, 9000).await?;
        println!("{}", t!("design.taste.wrote", path = png.display()));
    }
    for (family, f) in &facts {
        let kb = f.bytes.map(|b| b.div_ceil(1024).to_string()).unwrap_or_else(|| "?".into());
        println!("{}", t!("design.type.fact", family = family, licence = f.licence.as_deref().unwrap_or("?"), kb = kb));
    }
    for c in covered.iter().filter(|c| !c.missing.is_empty() && faces.iter().any(|f| f.family.eq_ignore_ascii_case(&c.family))) {
        println!("! {}", t!("design.type.gap", family = c.family, count = c.missing.len(), lang = c.lang.code()));
    }
    Ok(())
}

fn origin_says(p: component::OriginProblem) -> String {
    match p {
        component::OriginProblem::NoAddress => t!("design.component.no_address").to_owned(),
        component::OriginProblem::Key => t!("design.component.key").to_owned(),
        component::OriginProblem::NotAComponent => t!("design.component.not_component").to_owned(),
        component::OriginProblem::NotHttps => t!("design.component.not_https").to_owned(),
    }
}

pub async fn component_command(args: ComponentArgs) -> Result<(), String> {
    let origin = component::origin(&args.source).map_err(origin_says)?;
    if listed_only() && !listed(&origin.page()) {
        return Err(t!("design.refs.not_listed", url = origin.page(), sites = listed_sites()));
    }
    let design = design_dir(args.dir);
    let out = args.out.unwrap_or_else(|| design.join("vendor").join(refs::slug(&origin.name())));
    not_sandboxed()?;
    let client = client()?;
    let registry = origin.registry();
    // Its page's twin: where it came from, its licence and what it needs.
    let meta = match origin.twin() {
        Some(twin) => match robots(&client, &twin).await {
            Robots::Allowed => match get(&client, &twin).await? {
                (200, md) => component::meta(&md),
                _ => component::Meta::default(),
            },
            _ => component::Meta::default(),
        },
        None => component::Meta::default(),
    };
    let meta = match (&origin, meta.licence.is_none()) {
        // shadcn/ui's own registry is MIT-licensed.
        (component::Origin::Registry(url), true) if refs::site(url) == "ui.shadcn.com" => component::Meta { licence: Some("MIT (shadcn/ui)".into()), ..meta },
        _ => meta,
    };
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let source = out.join("SOURCE.md");
    // What can't be fetched is recorded, and said: its page, licence and needs.
    let record_only = |says: String| -> Result<(), String> {
        std::fs::write(&source, component::source_md(&origin, &meta, None, &today(), &[])).map_err(|e| format!("{}: {e}", source.display()))?;
        println!("{says}");
        Ok(())
    };
    // The registry, only where its site lets tools read it, and without a key.
    match robots(&client, &registry).await {
        Robots::Allowed => {}
        Robots::Refused => return record_only(t!("design.component.closed", site = refs::site(&registry), path = source.display())),
        Robots::Unread(why) => return Err(t!("design.refs.robots_unread", url = registry, status = why)),
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let (status, body) = get(&client, &registry).await?;
    if component::wants_key(status, &body) {
        return record_only(t!("design.component.needs_key", site = refs::site(&registry), path = source.display()));
    }
    let item = (status == 200).then(|| component::item(&body)).flatten().ok_or_else(|| t!("design.component.not_item", url = registry, status = status))?;
    let mut todo = Vec::new();
    for f in &item.files {
        let name = component::file_name(&f.path);
        std::fs::write(out.join(&name), &f.content).map_err(|e| format!("{name}: {e}"))?;
        let lines = component::restyle(&f.content);
        if !lines.is_empty() {
            todo.push((name, lines));
        }
    }
    std::fs::write(&source, component::source_md(&origin, &meta, Some(&item), &today(), &todo)).map_err(|e| format!("{}: {e}", source.display()))?;
    println!("{}", t!("design.component.wrote", count = item.files.len(), path = out.display()));
    let n: usize = todo.iter().map(|(_, l)| l.len()).sum();
    if n > 0 {
        println!("{}", t!("design.component.restyle", count = n));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        action: crate::design::DesignAction,
    }

    fn parse(words: &[&str]) -> Result<crate::design::DesignAction, String> {
        Cli::try_parse_from(std::iter::once("design").chain(words.iter().copied())).map(|c| c.action).map_err(|e| e.to_string())
    }

    fn shoot_args(words: &[&str]) -> ShootArgs {
        match parse(&[&["refs", "shoot"], words].concat()).unwrap() {
            crate::design::DesignAction::Refs { action: RefsAction::Shoot(a) } => a,
            _ => panic!("not refs shoot"),
        }
    }

    #[test]
    fn a_shoot_is_capped_and_every_page_says_what_to_learn() {
        let a = shoot_args(&["https://a.com", "https://b.com", "--learn", "Hero left, form right", "--learn", "Pricing as two cards", "--aspect", "layout"]);
        let (widths, aspects) = plan_shoot(&a).unwrap();
        assert_eq!(widths, [1280]);
        assert_eq!(aspects, vec![vec![refs::Aspect::Layout]; 2], "one aspect for every page");
        let a = shoot_args(&["https://a.com", "--learn", "x", "--aspect", "type", "--widths", "390,1280"]);
        assert_eq!(plan_shoot(&a).unwrap().0, [390, 1280]);
        let a = shoot_args(&["https://a.com", "--learn", "x", "--aspect", "type", "--widths", "390,768,1280"]);
        assert!(plan_shoot(&a).is_err(), "two widths at most");
        let a = shoot_args(&["https://a.com", "https://b.com", "--learn", "only one", "--aspect", "layout"]);
        assert!(plan_shoot(&a).unwrap_err().contains('2'), "a line for each page");
        let a = shoot_args(&["https://a.com", "https://b.com", "--learn", "a", "--learn", "b", "--aspect", "layout", "--aspect", "type", "--aspect", "rhythm"]);
        assert!(plan_shoot(&a).is_err(), "one aspect, or one per page");
        let many: Vec<String> = (0..=refs::MOST_PAGES).map(|i| format!("https://s{i}.com")).collect();
        let mut words: Vec<&str> = many.iter().map(String::as_str).collect();
        words.extend(["--learn", "x", "--aspect", "layout"]);
        assert!(plan_shoot(&shoot_args(&words)).unwrap_err().contains(&refs::MOST_PAGES.to_string()));
        let a = shoot_args(&["https://a.com", "--learn", "Use their logo up top", "--aspect", "layout"]);
        assert!(plan_shoot(&a).unwrap_err().contains("logo"), "the brand rule, before anything is read");
        let a = shoot_args(&["https://a.com", "--learn", "x", "--aspect", "vibes"]);
        assert!(plan_shoot(&a).unwrap_err().contains("vibes"));
        assert!(parse(&["refs", "shoot", "https://a.com", "--aspect", "layout"]).is_err(), "--learn is required");
    }

    #[test]
    fn the_other_tools_parse_as_a_designer_writes_them() {
        let crate::design::DesignAction::Palette(p) = parse(&["palette", "Ink=050315-fbfbfe-2f27ce-dedcff-433bff", "Warm=2b1d14-fbf7f2-b4532a-f1e4d6-2a7f62", "--fonts", "Fraunces/Inter", "--page", "index.html"]).unwrap() else { panic!() };
        assert!(p.page.as_deref() == Some("index.html") && p.palettes.len() == 2 && p.fonts.as_deref() == Some("Fraunces/Inter"));
        let crate::design::DesignAction::Type(t) = parse(&["type", "Fraunces/Inter", "Space Grotesk/IBM Plex Sans", "--locales", "en,ko,ja"]).unwrap() else { panic!() };
        assert_eq!(t.pairs.len(), 2);
        assert_eq!(locales(t.locales.as_deref(), Path::new("/nowhere")).unwrap(), [Lang::En, Lang::Ko, Lang::Ja]);
        assert_eq!(locales(Some("all"), Path::new("/nowhere")).unwrap().len(), 8);
        assert!(locales(Some("en,xx"), Path::new("/nowhere")).unwrap_err().contains("xx"));
        let mine = locales(None, Path::new("/nowhere")).unwrap();
        assert!(mine.contains(&Lang::En) && mine.contains(&crate::i18n::lang()));
        let crate::design::DesignAction::Component(c) = parse(&["component", "npx shadcn@latest add https://ui.shadcn.com/r/styles/new-york-v4/button.json"]).unwrap() else { panic!() };
        assert!(component::origin(&c.source).is_ok());
        assert!(matches!(parse(&["refs", "add", "shot.png", "--url", "https://lapa.ninja/post/x/", "--learn", "Bento grid", "--aspect", "layout"]).unwrap(), crate::design::DesignAction::Refs { action: RefsAction::Add { .. } }));
        assert!(matches!(parse(&["refs", "search", "21st", "hero"]).unwrap(), crate::design::DesignAction::Refs { action: RefsAction::Search { .. } }));
    }

    #[test]
    fn a_palette_count_and_its_colours_are_checked_before_anything_is_drawn() {
        let run = |words: &[&str]| {
            let crate::design::DesignAction::Palette(p) = parse(&[&["palette"], words].concat()).unwrap() else { panic!() };
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(palette_command(p))
        };
        let one = "One=050315-fbfbfe-2f27ce-dedcff-433bff";
        assert!(run(&[one, one, one, one, one]).unwrap_err().contains('4'), "four at most");
        assert!(run(&["A=050315-fbfbfe-2f27ce-dedcff-433bff", "B=nope"]).unwrap_err().contains("B=nope"));
        assert!(run(&["A=050315-fbfbfe-2f27ce-dedcff-433bff", "B=050315-fbfbfe-2f27ce-dedcff-433bff", "--fonts", "A/B/C"]).unwrap_err().contains("A/B/C"));
    }

    #[test]
    fn a_page_is_drawn_from_its_own_absolute_path() {
        let s = shot("https://a.com/", Path::new("/tmp/x/R1/1280.png"), 390, "dark", true);
        assert_eq!((s["height"].as_u64(), s["scheme"].as_str(), s["full"].as_bool()), (Some(844), Some("dark"), Some(true)));
        let script = include_str!("../assets/design/refs.mjs");
        assert!(script.contains("lastAt") && script.contains("pace"), "pages of one site are paced");
        assert!(script.contains("stopped.has(host)"), "a site that asked for a check isn't opened again");
        assert!(!script.contains("userAgent") && !script.contains("AutomationControlled"), "it never hides that it is automated");
        let lint = include_str!("../assets/design/lint.mjs");
        assert!(lint.contains("open(browser, width, 'dark')") && lint.contains("(dark)"), "the floor reads text contrast in a page's dark scheme too");
    }

    #[test]
    fn inside_an_engines_sandbox_it_says_so_before_reading_anything() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |n: &str| pairs.iter().find(|(k, _)| *k == n).map(|(_, v)| (*v).to_owned());
        assert!(outside_sandbox(&env(&[])).is_ok());
        assert!(outside_sandbox(&env(&[("CODEX_SANDBOX", "")])).is_ok(), "set but empty: not in one");
        let said = outside_sandbox(&env(&[("CODEX_SANDBOX", "seatbelt")])).unwrap_err();
        assert!(said.contains("outside the sandbox") && said.contains("palette"), "{said}");
        assert!(outside_sandbox(&env(&[("CODEX_SANDBOX_NETWORK_DISABLED", "1")])).is_err());
    }

    #[test]
    fn the_board_tool_runs_only_the_taste_commands_in_the_members_own_folder() {
        let words = |w: &[&str]| w.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(board_args(&words(&["refs", "shoot", "https://fontpair.co/", "--learn", "x", "--aspect", "type"])).unwrap()[..3], words(&["design", "refs", "shoot"]));
        assert!(board_args(&words(&["palette", "A=050315-fbfbfe-2f27ce-dedcff-433bff"])).is_ok());
        for refused in [&["lint", "index.html"][..], &["second-look", "x"], &[], &["refs", "board", "--dir", "/etc"], &["palette", "A=x", "--out=/tmp/x"]] {
            assert!(board_args(&words(refused)).is_err(), "{refused:?}");
        }
        // Each board call parses as the command would.
        for ok in [&["refs", "sources"][..], &["refs", "search", "21st", "hero"], &["type", "Fraunces/Inter", "--locales", "en,ja"], &["component", "https://ui.shadcn.com/r/styles/new-york-v4/card.json"]] {
            assert!(parse(&board_args(&words(ok)).unwrap()[1..].iter().map(String::as_str).collect::<Vec<_>>()).is_ok(), "{ok:?}");
        }
        assert!(listed("https://fontpair.co/for/saas") && listed("https://21st.dev/@a/components/b") && listed("https://ui.shadcn.com/r/x.json"));
        assert!(!listed("https://example.com/") && !listed("https://notfontpair.co/"));
        assert!(listed_sites().contains("fontpair.co") && !listed_sites().contains("lapa.ninja"), "Lapa Ninja is browsed, not read");
        assert!(!listed_sites().contains("realtimecolors"), "not a source");
        assert_eq!(plain("\u{1b}[1m✗\u{1b}[0m no"), "✗ no");
    }

    #[tokio::test]
    async fn one_call_at_a_time_holds_the_board() {
        let dir = std::env::temp_dir().join(format!("equipe-lock-{}", std::process::id()));
        let lock = dir.join(".lock");
        let first = Lock::take(lock.clone(), Duration::from_secs(5)).await;
        assert!(lock.is_file());
        let started = std::time::Instant::now();
        let waiter = tokio::spawn({
            let lock = lock.clone();
            async move { Lock::take(lock, Duration::from_secs(5)).await }
        });
        tokio::time::sleep(Duration::from_millis(600)).await;
        drop(first);
        let second = waiter.await.unwrap();
        assert!(started.elapsed() >= Duration::from_millis(600), "the second waited for the first");
        drop(second);
        assert!(!lock.exists(), "let go when done");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
