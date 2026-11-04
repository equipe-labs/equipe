//! `equipe design lint`, `second-look`, `direction` and `kept`: a screen held
//! to the design system in code (`equipe_core::design::lint`), drawn in
//! Chrome at each width by `assets/design/lint.mjs`; a fresh reviewer's
//! verdict on those pictures (`design::second_look`); the directions'
//! log (`design::directions`); and what a redesign must keep (`design::kept`).

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use clap::{Args, Subcommand};
use equipe_core::design::lint::{self, System, Tier, record, render, waivers};
use equipe_core::design::{critique, directions, kept, second_look};
use equipe_core::protocol::Engine;

/// Exit codes of `lint`: the floor fails; the floor passes with habits left
/// to fix or keep. Clean is 0.
pub const FLOOR_FAILS: i32 = 1;
pub const HABITS_OPEN: i32 = 2;

#[derive(Args)]
pub struct LintArgs {
    /// Files or folders of UI source (HTML, CSS, TSX, JSX, Vue, Svelte), or one URL to draw
    pub paths: Vec<String>,
    /// Also draw it in Chrome at each width and check what only a drawing shows
    #[arg(long)]
    pub render: bool,
    /// The page to draw for these sources (a running app: http://localhost:5173/settings)
    #[arg(long)]
    pub url: Option<String>,
    /// Widths in px, comma-separated
    #[arg(long, default_value = "360,390,768,1024,1280,1440,1920")]
    pub widths: String,
    /// `floor` for the floor alone (what never waives), `all` for habits and finish too
    #[arg(long, default_value = "all", value_parser = ["floor", "all"])]
    pub tier: String,
    /// The report as JSON
    #[arg(long)]
    pub json: bool,
    /// The screen it is, as the UI map names it (`settings/notifications`): its check is kept under that name
    #[arg(long)]
    pub screen: Option<String>,
    /// The design folder (default: the nearest design/ with a tokens.json)
    #[arg(long)]
    pub design: Option<PathBuf>,
    /// Keep a habit on purpose: write its waiver in design/decisions/ (with --why)
    #[arg(long, requires = "why")]
    pub waive: Option<String>,
    /// Why the habit stays, in the person's words or the brief's
    #[arg(long)]
    pub why: Option<String>,
    /// The screen or file the waiver holds for (default: everywhere)
    #[arg(long)]
    pub scope: Option<String>,
    /// The ticket the waiver was decided on
    #[arg(long)]
    pub ticket: Option<String>,
    /// Run as Claude Code's edit hook: the edit on stdin, floor failures on stderr with exit code 2
    #[arg(long, hide = true)]
    pub hook: bool,
    /// With --hook: write the edit down to check before a hand-in
    #[arg(long, hide = true)]
    pub ledger: bool,
    /// List every rule: its tier, what it checks, how to fix it
    #[arg(long)]
    pub rules: bool,
}

#[derive(Args)]
pub struct SecondLookArgs {
    /// The screen, as checked with `lint --render --screen <name>`
    pub screen: String,
    /// Only its first screenful: a new screen's opening, before the rest is built
    #[arg(long)]
    pub first: bool,
    /// What it must do: the acceptance criteria, as words or a file
    #[arg(long)]
    pub criteria: Option<String>,
    /// The brief (default: design/brand.md)
    #[arg(long)]
    pub brief: Option<PathBuf>,
    /// Who looks (default: a fresh run of the engine this is run from)
    #[arg(long, value_parser = ["codex", "claude"])]
    pub engine: Option<String>,
    /// Print the reviewer's prompt and stop: for a fresh subagent of your own when no engine can run
    #[arg(long)]
    pub prompt: bool,
    /// Record the answer a fresh subagent gave (a file holding its JSON verdict)
    #[arg(long)]
    pub answer: Option<PathBuf>,
    /// The design folder (default: ./design)
    #[arg(long)]
    pub design: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum DirectionAction {
    /// Propose a direction by its key choices: checked against the recent chosen ones and the other options
    Propose {
        name: String,
        #[arg(long)]
        layout: String,
        #[arg(long = "type")]
        type_: String,
        #[arg(long)]
        palette: String,
        #[arg(long)]
        density: String,
        #[arg(long)]
        motion: String,
        /// The ticket the options are for
        #[arg(long)]
        ticket: Option<String>,
        #[arg(long)]
        design: Option<PathBuf>,
    },
    /// The person chose this one: the next directions are held against it
    Choose {
        name: String,
        #[arg(long)]
        ticket: Option<String>,
        #[arg(long)]
        design: Option<PathBuf>,
    },
    /// The directions proposed and chosen, newest last
    List {
        #[arg(long)]
        design: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum KeptAction {
    /// Before a redesign: write down the screen's links, fields, actions and headings
    Record {
        /// Its source files, or its URL
        paths: Vec<String>,
        #[arg(long)]
        screen: String,
        #[arg(long)]
        design: Option<PathBuf>,
    },
    /// After it: what went missing, unless the brief or a decision says `Remove <kind>: …`
    Check {
        paths: Vec<String>,
        #[arg(long)]
        screen: String,
        /// The redesign's brief, for the removals it asks for
        #[arg(long)]
        brief: Option<PathBuf>,
        #[arg(long)]
        design: Option<PathBuf>,
    },
}

fn is_url(text: &str) -> bool {
    ["http://", "https://", "file://", "data:"].iter().any(|p| text.starts_with(p))
}

fn widths(text: &str) -> Result<Vec<u32>, String> {
    text.split(',').map(|w| w.trim().parse::<u32>().map_err(|_| format!("{w}: a width is a number of px"))).filter(|w| w.as_ref().map_or(true, |w| *w > 0)).collect()
}

/// The design folder: given, else the nearest to the first file, else ./design.
fn design_of(given: Option<PathBuf>, near: Option<&Path>) -> PathBuf {
    given.or_else(|| near.and_then(lint::design_dir_of)).unwrap_or_else(|| PathBuf::from(equipe_core::design::DIR))
}

fn screen_name(given: Option<&str>, target: &str) -> String {
    if let Some(s) = given {
        return record::safe(s);
    }
    let stem = if is_url(target) {
        target.split("://").nth(1).unwrap_or(target).split(['?', '#']).next().unwrap_or("").trim_end_matches('/').rsplit('/').next().unwrap_or("page").to_owned()
    } else {
        Path::new(target).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "page".into())
    };
    record::safe(if stem.is_empty() || stem.contains(':') { "home" } else { &stem })
}

pub async fn lint(args: LintArgs) -> Result<(), String> {
    if args.hook {
        return hook(args.ledger);
    }
    if args.rules {
        print!("{}", rules_text());
        return Ok(());
    }
    let first = args.paths.iter().find(|p| !is_url(p)).map(PathBuf::from);
    let design = design_of(args.design.clone(), first.as_deref());
    if let Some(rule) = &args.waive {
        return waive(&design, rule, args.scope.as_deref(), args.why.as_deref().unwrap_or(""), args.ticket.as_deref());
    }
    if args.paths.is_empty() && args.url.is_none() {
        return Err("name the files or folder to lint, or a URL with --render".into());
    }
    let files: Vec<PathBuf> = args.paths.iter().filter(|p| !is_url(p)).flat_map(|p| lint::ui_files(Path::new(p))).collect();
    for p in args.paths.iter().filter(|p| !is_url(p)) {
        if !Path::new(p).exists() {
            return Err(format!("{p}: no such file or folder"));
        }
    }
    let system = System::read(&design);
    let mut report = lint::lint_sources(&files, &system);
    let target = args.url.clone().or_else(|| args.paths.iter().find(|p| is_url(p)).cloned()).or_else(|| {
        // A page that draws itself: one HTML file.
        let pages: Vec<&PathBuf> = files.iter().filter(|f| f.extension().is_some_and(|e| e == "html" || e == "htm")).collect();
        (pages.len() == 1).then(|| pages[0].to_string_lossy().into_owned())
    });
    let screen = screen_name(args.screen.as_deref(), target.as_deref().unwrap_or_else(|| args.paths.first().map_or("page", String::as_str)));
    if args.render {
        let Some(target) = &target else {
            return Err("--render draws one page: give one HTML file, or the running page with --url http://localhost:…".into());
        };
        let mut measured = draw(target, &widths(&args.widths)?, &record::pictures_dir(&design, &screen)).await?;
        // Findings name the page as it was given, not as Chrome opened it.
        measured.url = target.clone();
        render::judge(&measured, &mut report);
    }
    lint::apply_waivers(&mut report, &design, Some(&screen));
    if args.tier == "floor" {
        report.only(Tier::Floor);
    }
    let mut summary = report.summary();
    summary.screen = Some(screen.clone());
    if args.render {
        let new = record::note_check(&design, &screen, &files).unwrap_or(false);
        let before = record::read(&design, &screen);
        let was_new = new || before.as_ref().is_some_and(|b| b.new);
        let kept = record::of(&design, &screen, &report, &files, before, was_new);
        let file = record::write(&design, &kept).map_err(|e| format!("{}: {e}", design.display()))?;
        summary.record = Some(file.to_string_lossy().into_owned());
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
    } else {
        print!("{}", report.text());
        if let Some(file) = &summary.record {
            println!("kept {file}");
            if summary.passes() {
                println!("Next: `equipe design second-look {screen}` — a fresh look at these pictures before you hand in.");
            }
        }
        println!("{}", lint::mark_line(&summary));
    }
    let code = if !summary.passes() { FLOOR_FAILS } else if summary.habits > 0 && args.tier == "all" { HABITS_OPEN } else { 0 };
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/// The page drawn and measured at each width, its pictures in `out`.
async fn draw(target: &str, widths: &[u32], out: &Path) -> Result<render::Measured, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let json = out.join(format!(".measures-{}.json", std::process::id()));
    let list = widths.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    eprintln!("Drawing {target} at {list}…");
    let ran = crate::design::run_script("lint.mjs", &[target, "--widths", &list, "--out", out.to_string_lossy().as_ref(), "--json", json.to_string_lossy().as_ref()]).await;
    let text = std::fs::read_to_string(&json);
    let _ = std::fs::remove_file(&json);
    ran?;
    serde_json::from_str(&text.map_err(|e| format!("lint.mjs wrote no measures: {e}"))?).map_err(|e| format!("lint.mjs's measures: {e}"))
}

fn waive(design: &Path, rule: &str, scope: Option<&str>, why: &str, ticket: Option<&str>) -> Result<(), String> {
    let text = waivers::decision(rule, scope, why, ticket)?;
    let dir = waivers::dir(design);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let day = equipe_core::squad_memory::date(record::now());
    let file = dir.join(format!("{day}-keep-{rule}{}.md", scope.map(|s| format!("-{}", record::safe(s).replace('/', "-"))).unwrap_or_default()));
    std::fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
    println!("wrote {} — `{rule}` is kept{} with its reason.", file.display(), scope.map(|s| format!(" on {s}")).unwrap_or_default());
    Ok(())
}

/// Claude Code's edit hook: the call on stdin; the floor's failures in what
/// was just written on stderr, exit code 2 — handed back to the member.
fn hook(ledger: bool) -> Result<(), String> {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&input) else { return Ok(()) };
    if let Some(said) = lint::hook::run(&value, ledger) {
        eprint!("{said}");
        std::process::exit(2);
    }
    Ok(())
}

pub async fn second_look(args: SecondLookArgs) -> Result<(), String> {
    let design = args.design.clone().unwrap_or_else(|| PathBuf::from(equipe_core::design::DIR));
    let screen = record::safe(&args.screen);
    let mut rec = record::read(&design, &screen).ok_or_else(|| format!("{screen} has no check: `equipe design lint <file> --render --screen {screen}` first"))?;
    if !rec.rendered {
        return Err(format!("{screen} was only read: check it with --render so there are pictures to look at"));
    }
    let widths = rec.summary.widths.clone();
    let pictures: Vec<(u32, PathBuf)> = rec
        .pictures
        .iter()
        .zip(&widths)
        .map(|(p, w)| (*w, if args.first { PathBuf::from(p.replace(".png", ".first.png")) } else { PathBuf::from(p) }))
        .filter(|(_, p)| p.is_file())
        .collect();
    if pictures.is_empty() {
        return Err(format!("{screen}'s pictures are gone: check it again with --render"));
    }
    let read = |p: &Path| std::fs::read_to_string(p).ok();
    let brief = args.brief.clone().or_else(|| Some(design.join("brand.md"))).and_then(|p| read(&p));
    let criteria = args.criteria.as_ref().map(|c| if Path::new(c).is_file() { read(Path::new(c)).unwrap_or_default() } else { c.clone() });
    let ask = second_look::Ask {
        screen: screen.clone(),
        pictures,
        brief,
        criteria,
        guidelines: read(&design.join("guidelines.md")),
        tokens: Some(design.join("tokens.json")).filter(|p| p.is_file()),
        first: args.first,
        kept: rec.summary.waived.clone(),
    };
    if args.prompt {
        println!("{}", second_look::prompt(&ask));
        return Ok(());
    }
    let (answer, by) = match &args.answer {
        Some(file) => (read(file).ok_or_else(|| format!("{}: no such file", file.display()))?, "subagent".to_owned()),
        None => {
            let engine = match args.engine.as_deref() {
                Some("codex") => Engine::Codex,
                Some(_) => Engine::Claude,
                None => critique::caller_engine(|n| std::env::var(n).ok()).unwrap_or(Engine::Claude),
            };
            (look(engine, &ask).await?, if engine == Engine::Codex { "codex".to_owned() } else { "claude".to_owned() })
        }
    };
    let mut verdict = second_look::verdict_of(&answer).ok_or("the reviewer gave no verdict (a JSON object with \"verdict\")")?;
    verdict.at = record::now();
    verdict.by = by;
    verdict.widths = ask.pictures.iter().map(|(w, _)| *w).collect();
    verdict.first = args.first;
    if args.first {
        rec.first = Some(verdict.clone());
    } else {
        rec.verdict = Some(verdict.clone());
    }
    record::write(&design, &rec).map_err(|e| format!("{}: {e}", design.display()))?;
    print!("{}", second_look::text(&verdict));
    if !verdict.pass {
        println!("Fix these, check again with `equipe design lint … --render --screen {screen}`, and ask for another look.");
    }
    println!("{}", second_look::mark_line(&screen, &verdict));
    if !verdict.pass {
        std::process::exit(1);
    }
    Ok(())
}

/// One fresh, read-only run of `engine` on the ask; its answer.
async fn look(engine: Engine, ask: &second_look::Ask) -> Result<String, String> {
    let answer = std::env::temp_dir().join(format!("equipe-second-look-{}.md", std::process::id()));
    let (program, args) = second_look::command(engine, ask, &answer);
    eprintln!("A fresh {} is looking at {} picture(s) of {}…", engine.label(), ask.pictures.len(), ask.screen);
    let run = tokio::process::Command::new(&program).args(&args).stdin(Stdio::null()).stderr(Stdio::piped()).output().await.map_err(|e| format!("{program}: {e} — is {} installed and signed in? Or give your own fresh subagent the --prompt and record its --answer.", engine.label()))?;
    if !run.status.success() {
        let said = [String::from_utf8_lossy(&run.stderr), String::from_utf8_lossy(&run.stdout)].join("\n");
        let tail: Vec<&str> = said.lines().filter(|l| !l.trim().is_empty()).rev().take(6).collect();
        return Err(format!("{} couldn't look ({}): {}", engine.label(), run.status, tail.into_iter().rev().collect::<Vec<_>>().join("\n")));
    }
    let text = if engine == Engine::Claude { String::from_utf8_lossy(&run.stdout).into_owned() } else { std::fs::read_to_string(&answer).unwrap_or_default() };
    let _ = std::fs::remove_file(&answer);
    Ok(text)
}

pub fn direction(action: DirectionAction) -> Result<(), String> {
    match action {
        DirectionAction::Propose { name, layout, type_, palette, density, motion, ticket, design } => {
            let design = design.unwrap_or_else(|| PathBuf::from(equipe_core::design::DIR));
            let mut log = directions::read(&design);
            let proposal = directions::Direction { name: name.clone(), ticket, at: record::now(), chosen: false, layout, type_, palette, density, motion };
            let locked = directions::locked(&design);
            let problems = directions::check(&log, &proposal, &locked);
            if !problems.is_empty() {
                return Err(format!("“{name}” isn't a new direction yet:\n{}", problems.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n")));
            }
            directions::record(&mut log, proposal);
            directions::write(&design, &log).map_err(|e| format!("{}: {e}", design.display()))?;
            println!("“{name}” is in {}{}.", directions::path(&design).display(), if locked.is_empty() { String::new() } else { format!(" (the brand locks {})", locked.join(", ")) });
            Ok(())
        }
        DirectionAction::Choose { name, ticket, design } => {
            let design = design.unwrap_or_else(|| PathBuf::from(equipe_core::design::DIR));
            let mut log = directions::read(&design);
            if !directions::choose(&mut log, &name, ticket.as_deref()) {
                return Err(format!("no direction “{name}” proposed{}", ticket.map(|t| format!(" on {t}")).unwrap_or_default()));
            }
            directions::write(&design, &log).map_err(|e| format!("{}: {e}", design.display()))?;
            println!("“{name}” is chosen: the next directions differ from it.");
            Ok(())
        }
        DirectionAction::List { design } => {
            let design = design.unwrap_or_else(|| PathBuf::from(equipe_core::design::DIR));
            for d in directions::read(&design) {
                println!("{} {}{} — layout: {}; type: {}; palette: {}; density: {}; motion: {}", if d.chosen { "✓" } else { "·" }, d.name, d.ticket.map(|t| format!(" ({t})")).unwrap_or_default(), d.layout, d.type_, d.palette, d.density, d.motion);
            }
            Ok(())
        }
    }
}

async fn inventory(paths: &[String]) -> Result<kept::Inventory, String> {
    let mut inv = kept::Inventory::default();
    for p in paths {
        if is_url(p) {
            let json = std::env::temp_dir().join(format!("equipe-kept-{}.json", std::process::id()));
            let ran = crate::design::run_script("lint.mjs", &[p, "--inventory", "--json", json.to_string_lossy().as_ref()]).await;
            let text = std::fs::read_to_string(&json);
            let _ = std::fs::remove_file(&json);
            ran?;
            inv.merge(serde_json::from_str(&text.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?);
            continue;
        }
        for file in lint::ui_files(Path::new(p)) {
            let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            if let Some(src) = lint::source::Source::read(&file, text) {
                inv.merge(kept::Inventory::of_source(&src));
            }
        }
    }
    Ok(inv)
}

pub async fn kept(action: KeptAction) -> Result<(), String> {
    match action {
        KeptAction::Record { paths, screen, design } => {
            let design = design_of(design, paths.iter().find(|p| !is_url(p)).map(Path::new));
            let inv = inventory(&paths).await?;
            if inv.is_empty() {
                return Err("found no links, fields, actions or headings to keep".into());
            }
            let file = kept::path(&design, &screen);
            std::fs::create_dir_all(file.parent().unwrap_or(&design)).map_err(|e| e.to_string())?;
            std::fs::write(&file, format!("{}\n", serde_json::to_string_pretty(&inv).unwrap_or_default())).map_err(|e| format!("{}: {e}", file.display()))?;
            println!("wrote {}: {} links, {} fields, {} actions, {} headings to keep.", file.display(), inv.links.len(), inv.fields.len(), inv.actions.len(), inv.headings.len());
            Ok(())
        }
        KeptAction::Check { paths, screen, brief, design } => {
            let design = design_of(design, paths.iter().find(|p| !is_url(p)).map(Path::new));
            let file = kept::path(&design, &screen);
            let before: kept::Inventory = serde_json::from_str(&std::fs::read_to_string(&file).map_err(|_| format!("{screen} has nothing recorded: `equipe design kept record <its files> --screen {screen}` before the redesign"))?).map_err(|e| format!("{}: {e}", file.display()))?;
            let after = inventory(&paths).await?;
            let mut allowed = kept::allowed_by_decisions(&design);
            if let Some(brief) = brief {
                allowed.extend(kept::removals(&std::fs::read_to_string(&brief).map_err(|e| format!("{}: {e}", brief.display()))?));
            }
            let gone = kept::missing(&before, &after, &allowed);
            if gone.is_empty() {
                println!("{screen} keeps everything it offered.");
                return Ok(());
            }
            for (kind, item) in &gone {
                println!("✗ the {kind} “{item}” is gone");
            }
            Err(format!("{} thing(s) the screen offered went missing: bring them back, or have the brief say `Remove <kind>: <item>`", gone.len()))
        }
    }
}

/// Every rule by tier, what it checks and how it is fixed: `equipe design lint --rules`.
pub fn rules_text() -> String {
    let mut out = String::new();
    for tier in [Tier::Floor, Tier::Habit, Tier::Finish] {
        out.push_str(match tier {
            Tier::Floor => "Floor — never waived:\n",
            Tier::Habit => "\nHabits — fixed, or kept with a reason in design/decisions/:\n",
            Tier::Finish => "\nFinish — said, never blocking:\n",
        });
        for r in lint::RULES.iter().filter(|r| r.tier == tier) {
            let how = match (r.how.in_source(), r.how.in_render()) {
                (true, true) => "source, drawn",
                (true, false) => "source",
                _ => "drawn",
            };
            out.push_str(&format!("  {:<17} {} ({how})\n  {:<17} fix: {}\n", r.id, r.checks, "", r.fix));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screen_is_named_by_its_flag_or_its_file_or_url() {
        assert_eq!(screen_name(Some("settings/notifications"), "x"), "settings/notifications");
        assert_eq!(screen_name(None, "design/mockups/billing.html"), "billing");
        assert_eq!(screen_name(None, "http://localhost:5173/settings/team?x=1"), "team");
        assert_eq!(screen_name(None, "http://localhost:5173/"), "home");
        assert!(rules_text().contains("Floor — never waived:\n  token-colour"));
        assert_eq!(widths("360, 1280").unwrap(), vec![360, 1280]);
        assert!(widths("wide").is_err());
    }
}
