//! `equipe design …`: the designer's tools (`equipe_core::design::{check,
//! kit}`, and two scripts run in the Mac's own Chrome through Playwright —
//! `assets/design/{audit,render}.mjs`), and a fresh critic — the other engine
//! when it can take it (`equipe_core::design::{critique, reviewer}`). A
//! member runs them in its worktree;
//! so can the person. `equipe design flows …` keeps the UI map
//! (`equipe_core::design::flows`): every flow, its screens, each in every
//! state, pictured light and dark at each width — a web page's states shot
//! in Chrome, an app's by the `command` the map gives them, run at the
//! repository's root.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use clap::Subcommand;
use equipe_core::design::{check, critique, flows, kit, reviewer, tokens};
use equipe_core::protocol::Engine;
use equipe_core::protocol::design::{FlowMap, ScreenState};
use equipe_core::protocol::engines::Reviewed;
use equipe_core::t;

#[derive(Subcommand)]
pub enum DesignAction {
    /// Lay out a design folder to fill in: brand.md, layered tokens.json, guidelines.md
    Init {
        /// The design folder (default: ./design)
        dir: Option<PathBuf>,
    },
    /// Check a design folder before handing it in: tokens equipe reads, layers, dark, contrast, type for the product's script, brand, logo
    Check {
        dir: Option<PathBuf>,
        /// The findings as JSON
        #[arg(long)]
        json: bool,
    },
    /// Draw a system from its tokens alone — screens light and dark, palette, type, spacing — as HTML (and PNG)
    Specimen {
        dir: Option<PathBuf>,
        /// Another tokens file (a direction's, for style tiles)
        #[arg(long)]
        tokens: Option<PathBuf>,
        /// What the sheet is called
        #[arg(long)]
        title: Option<String>,
        /// Where the HTML goes (default: <dir>/components/specimen.html)
        #[arg(long)]
        out: Option<PathBuf>,
        /// Also render it to PNG beside the HTML
        #[arg(long)]
        png: bool,
    },
    /// Draw HTML or SVG files to PNG, at each width, light (and dark)
    Render {
        files: Vec<String>,
        /// Widths in px, comma-separated
        #[arg(long, default_value = "1280")]
        widths: String,
        /// Light and dark (prefers-color-scheme)
        #[arg(long)]
        dark: bool,
        /// Where the PNGs go (default: beside the first file)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Shoot running pages as people see them: each URL at each width, light and dark
    Shoot {
        /// The pages: http://localhost:3000/, http://localhost:3000/pricing …
        urls: Vec<String>,
        /// Widths in px, comma-separated
        #[arg(long, default_value = "390,768,1280")]
        widths: String,
        /// Light only (dark is shot too unless this is given)
        #[arg(long)]
        light: bool,
        /// Where the PNGs go
        #[arg(long, default_value = "shots")]
        out: PathBuf,
    },
    /// A spec or mockup and what was built, side by side, and where they differ — one sheet
    Diff {
        spec: String,
        built: String,
        /// The sheet (default: diff.png)
        #[arg(long, default_value = "diff.png")]
        out: PathBuf,
    },
    /// Have a fresh reviewer — the other engine when it's available — critique pictures: the whole first, then each region; kept in design/critiques/
    Critique {
        /// The pictures: mockups, shots of what was built
        #[arg(required = true)]
        pictures: Vec<PathBuf>,
        /// The reference it should stand beside (repeatable)
        #[arg(long = "reference")]
        references: Vec<PathBuf>,
        /// The round before, to check the revision didn't lose the whole (repeatable)
        #[arg(long)]
        before: Vec<PathBuf>,
        /// What the work is for
        #[arg(long)]
        goal: Option<String>,
        /// The brief (default: design/brand.md, if there is one)
        #[arg(long)]
        brief: Option<PathBuf>,
        /// Who critiques (default: as the squad sets it — the other engine when it has room, else a fresh session of this one)
        #[arg(long, value_parser = ["codex", "claude"])]
        engine: Option<String>,
    },
    /// Harvest a brand from its site: colours by area, type and webfonts, logo files, voice — one audit sheet
    Audit {
        url: String,
        /// How many of its pages to read (the home page and its navigation's)
        #[arg(long, default_value_t = 3)]
        pages: u8,
        /// Where the sheet and files go (default: ./design/audit)
        #[arg(long)]
        out: Option<PathBuf>,
        /// The language the site is asked for, as the people the product is for read it (default: yours)
        #[arg(long)]
        locale: Option<String>,
    },
    /// Hold a screen to the design system in code: the floor (never waived), the habits of generated screens, finishing notes — in its source, and with --render as Chrome draws it at each width
    Lint(crate::design_lint::LintArgs),
    /// A fresh reviewer's verdict on a checked screen's pictures, given only them, the brief and the system — what a hand-in waits on
    SecondLook(crate::design_lint::SecondLookArgs),
    /// The directions proposed and chosen, by layout, type, palette, density and motion — so the next one differs
    Direction {
        #[command(subcommand)]
        action: crate::design_lint::DirectionAction,
    },
    /// What a redesign must keep: a screen's links, fields, actions and headings, recorded before and checked after
    Kept {
        #[command(subcommand)]
        action: crate::design_lint::KeptAction,
    },
    /// A project's design system as code — its tokens, components.json, registry, components and docs pages — what is there and what is missing
    System(crate::design_system::SystemArgs),
    /// A registry component drawn in every variant, state and theme from its docs page, with axe-core and a keyboard pass — one contact sheet
    Matrix(crate::design_system::MatrixArgs),
    /// Check registry.json before a release — files, dependencies, sources and licences, no other brand's assets — build it, and install a sample into a scratch project
    Registry(crate::design_system::RegistryArgs),
    /// The UI map: every flow, its screens in order, each in every state, pictured light and dark at each width
    Flows {
        #[command(subcommand)]
        action: FlowsAction,
    },
    /// A brief's reference board: pages from galleries and tools, each with what to learn from it, as one contact sheet
    Refs {
        #[command(subcommand)]
        action: crate::design_taste::RefsAction,
    },
    /// Two to four palettes on a real layout, light and dark, with their contrast, as tokens
    Palette(crate::design_taste::PaletteArgs),
    /// Type pairs set side by side as a specimen, in the product's languages, with coverage, licence and weight
    Type(crate::design_taste::TypeArgs),
    /// A public component's source pulled in as a starting point, with its source and licence, to restyle to our tokens
    Component(crate::design_taste::ComponentArgs),
}

#[derive(Subcommand)]
pub enum FlowsAction {
    /// Start the map to fill in: design/flows/flows.json (never over one that's there)
    Init {
        /// The design folder (default: ./design)
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Check the map before handing it in: states missing, pictures older than their code, no dark, pictures it doesn't list
    Check {
        /// The design folder (default: ./design)
        #[arg(long)]
        dir: Option<PathBuf>,
        /// The findings as JSON
        #[arg(long)]
        json: bool,
    },
    /// Picture each state by its command, else from its URL: at each width, light and dark, into flows/<flow>/<screen>/
    Shoot {
        /// The design folder (default: ./design)
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Only this flow's screens
        #[arg(long)]
        flow: Option<String>,
        /// Only this screen
        #[arg(long)]
        screen: Option<String>,
        /// Widths in px, comma-separated
        #[arg(long, default_value = "390,1280")]
        widths: String,
        /// Light only (dark is shot too unless this is given)
        #[arg(long)]
        light: bool,
        /// How many of the map's commands run at once
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..=64))]
        jobs: u16,
        /// How long each command may run, in seconds, before it is stopped
        #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..))]
        timeout_secs: u64,
    },
    /// The map as an outline: each flow's screens in order, their states, what is missing or stale
    List {
        /// The design folder (default: ./design)
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Record a flow as a video to watch again: its walk at a person's pace, captioned, into flows/<flow>/walkthrough.<mode>.<width>.mp4
    Record {
        /// The design folder (default: ./design)
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Only this flow (default: every flow with a walk)
        #[arg(long)]
        flow: Option<String>,
        /// Widths in px, comma-separated
        #[arg(long, default_value = "390")]
        widths: String,
        /// Light only (default: light and dark)
        #[arg(long, conflicts_with = "dark")]
        light: bool,
        /// Dark only
        #[arg(long)]
        dark: bool,
        /// How many earlier recordings to keep in history/, for each mode and width
        #[arg(long, default_value_t = 5)]
        keep: usize,
        /// How long a flow's record_command may run, in seconds, before it is stopped
        #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u64).range(1..))]
        timeout_secs: u64,
    },
}

fn dir_or_default(dir: Option<PathBuf>) -> PathBuf {
    dir.unwrap_or_else(|| PathBuf::from(equipe_core::design::DIR))
}

pub async fn command(action: DesignAction) -> Result<(), String> {
    match action {
        DesignAction::Init { dir } => {
            let dir = dir_or_default(dir);
            let written = kit::init(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            if written.is_empty() {
                println!("{} has its files already; nothing written.", dir.display());
            }
            for path in written {
                println!("wrote {}", path.display());
            }
            println!("Next: fill in brand.md with the person (who it is for, this-not-that, voice), then the tokens; `equipe design check` when done.");
            Ok(())
        }
        DesignAction::Check { dir, json } => {
            let dir = dir_or_default(dir);
            let findings = check::check(&dir);
            if json {
                println!("{}", serde_json::to_string_pretty(&findings).unwrap_or_default());
            } else {
                print!("{}", check::report(&findings));
            }
            if findings.iter().any(|f| f.level == check::Level::Error) { Err("the design folder has errors to fix".into()) } else { Ok(()) }
        }
        DesignAction::Specimen { dir, tokens: file, title, out, png } => {
            let dir = dir_or_default(dir);
            let file = file.unwrap_or_else(|| dir.join(equipe_core::design::folder::TOKENS));
            let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let parsed = tokens::parse(&text);
            for p in &parsed.problems {
                eprintln!("! {p:?}");
            }
            let title = title.unwrap_or_else(|| file.file_stem().map(|s| s.to_string_lossy().replace(".tokens", "")).filter(|s| s != "tokens").unwrap_or_else(|| "Design system".into()));
            let out = out.unwrap_or_else(|| dir.join("components").join(if file.ends_with(equipe_core::design::folder::TOKENS) { "specimen.html".into() } else { format!("specimen-{}.html", file.file_stem().unwrap_or_default().to_string_lossy().replace(".tokens", "")) }));
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&out, kit::specimen(&parsed.tokens, &parsed.modes, &title, crate::i18n::lang())).map_err(|e| format!("{}: {e}", out.display()))?;
            println!("wrote {}", out.display());
            if png {
                let at = out.parent().unwrap_or(Path::new(".")).to_path_buf();
                run_script("render.mjs", &[out.to_string_lossy().as_ref(), "--widths", "1280", "--out", at.to_string_lossy().as_ref()]).await?;
            }
            Ok(())
        }
        DesignAction::Render { files, widths, dark, out } => {
            if files.is_empty() {
                return Err("name an HTML or SVG file to render".into());
            }
            let out = out.unwrap_or_else(|| Path::new(&files[0]).parent().map(Path::to_path_buf).filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| PathBuf::from(".")));
            let mut args: Vec<String> = files;
            args.extend(["--widths".into(), widths, "--out".into(), out.to_string_lossy().into_owned()]);
            if dark {
                args.push("--dark".into());
            }
            run_script("render.mjs", &args.iter().map(String::as_str).collect::<Vec<_>>()).await
        }
        DesignAction::Shoot { urls, widths, light, out } => {
            if urls.is_empty() {
                return Err("name the pages to shoot: http://localhost:<port>/…".into());
            }
            let mut args: Vec<String> = urls;
            args.extend(["--widths".into(), widths, "--out".into(), out.to_string_lossy().into_owned()]);
            if !light {
                args.push("--dark".into());
            }
            run_script("render.mjs", &args.iter().map(String::as_str).collect::<Vec<_>>()).await
        }
        DesignAction::Diff { spec, built, out } => run_script("diff.mjs", &[&spec, &built, "--out", out.to_string_lossy().as_ref()]).await,
        DesignAction::Critique { pictures, references, before, goal, brief, engine } => critique_command(critique::Ask { pictures, references, before, brief: None, goal }, brief, engine).await,
        DesignAction::Audit { url, pages, out, locale } => {
            let out = out.unwrap_or_else(|| PathBuf::from(equipe_core::design::DIR).join("audit"));
            run_script("audit.mjs", &[&url, "--pages", &pages.to_string(), "--out", out.to_string_lossy().as_ref(), "--locale", &audit_locale(locale)]).await
        }
        DesignAction::Flows { action } => flows_command(action).await,
        DesignAction::Lint(args) => crate::design_lint::lint(args).await,
        DesignAction::SecondLook(args) => crate::design_lint::second_look(args).await,
        DesignAction::Direction { action } => crate::design_lint::direction(action),
        DesignAction::Kept { action } => crate::design_lint::kept(action).await,
        DesignAction::Refs { action } => crate::design_taste::refs_command(action).await,
        DesignAction::Palette(args) => crate::design_taste::palette_command(args).await,
        DesignAction::Type(args) => crate::design_taste::type_command(args).await,
        DesignAction::Component(args) => crate::design_taste::component_command(args).await,
        DesignAction::System(args) => crate::design_system::system_command(args),
        DesignAction::Matrix(args) => crate::design_system::matrix_command(args).await,
        DesignAction::Registry(args) => crate::design_system::registry_command(args).await,
    }
}

async fn flows_command(action: FlowsAction) -> Result<(), String> {
    match action {
        FlowsAction::Init { dir } => {
            let (file, written) = init_map(&dir_or_default(dir))?;
            if written {
                println!("{}", t!("design.flows.wrote", path = file.display()));
                println!("{}", t!("design.flows.init_next"));
            } else {
                println!("{}", t!("design.flows.init_there", path = file.display()));
            }
            Ok(())
        }
        FlowsAction::Check { dir, json } => {
            let map = read_map(&dir_or_default(dir))?;
            let findings = flows::check(&map);
            if json {
                println!("{}", serde_json::to_string_pretty(&findings).unwrap_or_default());
            } else {
                print!("{}", report(&map, &findings));
            }
            if failing(&map, &findings) { Err(t!("design.flows.check_failed").to_owned()) } else { Ok(()) }
        }
        FlowsAction::Shoot { dir, flow, screen, widths: text, light, jobs, timeout_secs } => {
            let dir = dir_or_default(dir);
            if !map_file(&dir).is_file() {
                return Err(no_map(&dir));
            }
            let plan = plan(&flows::shots(&dir), flow.as_deref(), screen.as_deref(), &widths(&text)?, light)?;
            print!("{}", draw_list(&plan.draw));
            let failed = if plan.runs.is_empty() { 0 } else { shoot_by_commands(&dir, &plan.runs, usize::from(jobs), Duration::from_secs(timeout_secs)).await? };
            if !plan.takes.is_empty() {
                let kit = kit_dir();
                std::fs::create_dir_all(&kit).map_err(|e| format!("{}: {e}", kit.display()))?;
                let file = kit.join(format!("flows-plan-{}.json", std::process::id()));
                std::fs::write(&file, serde_json::to_vec(&plan.takes).unwrap_or_default()).map_err(|e| format!("{}: {e}", file.display()))?;
                eprintln!("{}", t!("design.flows.shooting", count = plan.takes.len()));
                let ran = run_script("render.mjs", &["--plan", file.to_string_lossy().as_ref()]).await;
                let _ = std::fs::remove_file(&file);
                ran?;
            }
            if failed > 0 { Err(t!("design.flows.commands_failed", count = failed)) } else { Ok(()) }
        }
        FlowsAction::List { dir } => {
            print!("{}", outline(&read_map(&dir_or_default(dir))?));
            Ok(())
        }
        FlowsAction::Record { dir, flow, widths: text, light, dark, keep, timeout_secs } => record(&dir_or_default(dir), flow.as_deref(), &widths(&text)?, record_modes(light, dark), keep, Duration::from_secs(timeout_secs)).await,
    }
}

/// The modes a flow is recorded in: both, unless one is asked for.
fn record_modes(light: bool, dark: bool) -> &'static [&'static str] {
    match (light, dark) {
        (true, false) => &["light"],
        (false, true) => &["dark"],
        _ => &["light", "dark"],
    }
}

/// `flows record`: each flow chosen walked through at each width in each
/// mode, the one before kept in history/.
async fn record(design: &Path, only: Option<&str>, widths: &[u32], modes: &[&str], keep: usize, limit: Duration) -> Result<(), String> {
    let map = read_map(design)?;
    let lang = crate::i18n::lang();
    let chosen: Vec<&equipe_core::protocol::design::Flow> = map.flows.iter().filter(|f| only.is_none_or(|id| f.id == id)).collect();
    if let (Some(id), true) = (only, chosen.is_empty()) {
        return Err(t!("design.flows.nothing_matches", what = format!("--flow {id}")));
    }
    let mut walks = Vec::new();
    for flow in chosen {
        match flows::recording(design, &flow.id, lang) {
            Ok(how) => walks.push((flow, how)),
            Err(why) if only.is_some() => return Err(why),
            Err(why) => why.lines().for_each(|line| eprintln!("! {line}")),
        }
    }
    if walks.is_empty() {
        return Err(t!("design.flows.no_walks").to_owned());
    }
    // Chrome's walks need an encoder; a record_command, the repository it runs in.
    let encoder = match walks.iter().any(|(_, how)| matches!(how, flows::Recording::Walk(..))) {
        true => Some(Encoder::find().ok_or_else(|| t!("design.flows.no_encoder").to_owned())?),
        false => None,
    };
    let repo = match walks.iter().any(|(_, how)| matches!(how, flows::Recording::Command(_))) {
        true => Some(command_root(design)?),
        false => None,
    };
    let work = std::path::absolute(kit_dir().join(format!("record-{}", std::process::id()))).map_err(|e| e.to_string())?;
    let ran = async {
        for (flow, how) in &walks {
            let titles = flow.screens.iter().map(|s| (s.id.clone(), s.title.clone())).collect();
            let at = design.join(flows::DIR).join(&flow.id);
            for &width in widths {
                for &mode in modes {
                    eprintln!("{}", t!("design.flows.recording", flow = flow.id, width = width, mode = mode_word(mode)));
                    let made = match (how, &encoder, &repo) {
                        (flows::Recording::Walk(steps, browse), Some(encoder), _) => record_one(&work, encoder, &walk_script(steps, &titles), browse, width, mode).await,
                        (flows::Recording::Command(command), _, Some(repo)) => record_by_command(&work, command, repo, &flow.id, width, mode, limit).await,
                        _ => unreachable!("an encoder for each walk, a repository for each command"),
                    };
                    let made = made.map_err(|why| t!("design.flows.record_failed", flow = flow.id, why = why))?;
                    let kept = keep_video(&at, mode, width, made, keep).map_err(|e| format!("{}: {e}", at.display()))?;
                    if let Some(before) = &kept.before {
                        println!("{}", t!("design.flows.shelved", path = before.display()));
                    }
                    println!("{}", t!("design.flows.recorded", path = kept.video.display(), length = length(kept.millis)));
                    println!("{}", t!("design.flows.wrote", path = kept.video.with_extension("json").display()));
                }
            }
        }
        Ok(())
    }
    .await;
    let _ = std::fs::remove_dir_all(&work);
    ran
}

fn mode_word(mode: &str) -> &'static str {
    if mode == "dark" { t!("design.flows.dark") } else { t!("design.flows.light") }
}

/// A video made in the work folder, and what it shows.
#[derive(Debug)]
struct Made {
    video: PathBuf,
    sidecar: Sidecar,
}

/// One walk recorded at `width` in `mode`, in the browser `browse` says, into `work`.
async fn record_one(work: &Path, encoder: &Encoder, script: &str, browse: &flows::Browse, width: u32, mode: &str) -> Result<Made, String> {
    let _ = std::fs::remove_dir_all(work);
    let frames = work.join("frames");
    std::fs::create_dir_all(&frames).map_err(|e| format!("{}: {e}", frames.display()))?;
    let walk = work.join("walk.mjs");
    std::fs::write(&walk, script).map_err(|e| format!("{}: {e}", walk.display()))?;
    let raw = work.join("raw.json");
    let plan = record_plan(&walk, &frames, &raw, browse, width, mode);
    let file = work.join("plan.json");
    std::fs::write(&file, plan.to_string()).map_err(|e| format!("{}: {e}", file.display()))?;
    run_script("record.mjs", &["--plan", file.to_string_lossy().as_ref()]).await?;
    let raw: Raw = serde_json::from_str(&std::fs::read_to_string(&raw).map_err(|e| format!("{}: {e}", raw.display()))?).map_err(|e| e.to_string())?;
    let line = timeline(raw)?;
    let video = work.join("walkthrough.mp4");
    encoder.encode(&line, work, &video).await?;
    Ok(Made { video, sidecar: Sidecar { millis: line.millis, marks: line.marks } })
}

/// What `record.mjs` is told: the walk, where its frames and what it saw
/// go, the viewport, the mode, and the browser — `en-US` unless the flow
/// says its `locale`, as `flows shoot` is, so a site that offers a
/// visitor's own language doesn't on the Mac's.
fn record_plan(walk: &Path, frames: &Path, raw: &Path, browse: &flows::Browse, width: u32, mode: &str) -> serde_json::Value {
    let (height, scale) = viewport(width);
    let locale = browse.locale.as_deref().unwrap_or("en-US");
    serde_json::json!({ "walk": walk, "frames": frames, "raw": raw, "width": width, "height": height, "scale": scale, "scheme": mode,
        "locale": locale, "reducedMotion": browse.reduced_motion.unwrap_or(false), "permissions": browse.permissions })
}

/// One flow recorded by its own `record_command` at `width` in `mode`, into
/// `work`, at the root of `repo`: the video it wrote, checked, and the note
/// it wrote beside it, if any.
async fn record_by_command(work: &Path, command: &str, repo: &Path, flow: &str, width: u32, mode: &str, limit: Duration) -> Result<Made, String> {
    let _ = std::fs::remove_dir_all(work);
    std::fs::create_dir_all(work).map_err(|e| format!("{}: {e}", work.display()))?;
    let video = work.join("walkthrough.mp4");
    let values = flows::record_values(&video, mode, width, flow);
    made_by(&flows::fill(command, &values), &video, repo, &flows::env_of("EQUIPE_RECORD_", &values), limit, Want::Mp4).await?;
    let sidecar = std::fs::read(video.with_extension("json")).ok().and_then(|b| serde_json::from_slice::<Sidecar>(&b).ok()).unwrap_or(Sidecar { millis: 0, marks: Vec::new() });
    Ok(Made { video, sidecar })
}

/// The viewport a width is recorded in: its height, and its pixel scale
/// (sharp on a phone's width, lighter on a desktop's).
fn viewport(width: u32) -> (u32, u32) {
    (if width < 600 { 844 } else { 800 }, if width <= 800 { 2 } else { 1 })
}

/// The steps as the module `record.mjs` runs: each a call on what it hands
/// over, a screen's mark and caption first where one begins.
fn walk_script(steps: &[flows::WalkStep], titles: &std::collections::BTreeMap<String, String>) -> String {
    use flows::Step;
    let quote = |text: &str| serde_json::to_string(text).unwrap_or_default();
    let mut script = String::from("// Written by `equipe design flows record` from flows.json.\nexport default async (w) => {\n");
    for walk_step in steps {
        if let Some(screen) = &walk_step.screen {
            let title = titles.get(screen).map_or(screen.as_str(), String::as_str);
            script.push_str(&format!("  await w.screen({}, {});\n", quote(screen), quote(title)));
        }
        let call = match &walk_step.step {
            Step::Go(url) => format!("go({})", quote(url)),
            Step::Click(selector) => format!("click({})", quote(selector)),
            Step::Fill(selector, text) => format!("fill({}, {})", quote(selector), quote(text)),
            Step::Press(key) => format!("press({})", quote(key)),
            Step::Wait(ms) => format!("wait({ms})"),
            Step::Say(text) => format!("say({})", quote(text)),
        };
        script.push_str(&format!("  await w.{call};\n"));
    }
    script.push_str("};\n");
    script
}

/// What `record.mjs` saw, in Unix ms.
#[derive(Debug, serde::Deserialize)]
struct Raw {
    start: u64,
    end: u64,
    frames: Vec<RawFrame>,
    marks: Vec<RawMark>,
}

#[derive(Debug, serde::Deserialize)]
struct RawFrame {
    file: PathBuf,
    t: u64,
}

#[derive(Debug, serde::Deserialize)]
struct RawMark {
    screen: String,
    t: u64,
}

/// Where a screen begins in a video, in ms from its start.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Mark {
    screen: String,
    at: u64,
}

/// What sits beside a video: how long it is and where each screen begins.
#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Sidecar {
    #[serde(default)]
    millis: u64,
    #[serde(default)]
    marks: Vec<Mark>,
}

/// A recording on the video's own clock: its length, each frame from when
/// it shows, each screen's mark.
#[derive(Debug, PartialEq, Eq)]
struct Timeline {
    millis: u64,
    frames: Vec<(PathBuf, u64)>,
    marks: Vec<Mark>,
}

/// `raw` on the video's clock, from the moment filming began: the first
/// frame shown from 0, a frame replaced in the same ms dropped, a mark
/// before filming at 0.
fn timeline(raw: Raw) -> Result<Timeline, String> {
    let millis = raw.end.saturating_sub(raw.start);
    let on_clock = |t: u64| t.saturating_sub(raw.start).min(millis);
    let mut frames: Vec<(PathBuf, u64)> = raw.frames.into_iter().map(|f| (f.file, on_clock(f.t))).collect();
    frames.sort_by_key(|(_, at)| *at);
    let mut kept: Vec<(PathBuf, u64)> = Vec::with_capacity(frames.len());
    for (file, at) in frames {
        match kept.last_mut() {
            Some(last) if last.1 == at => last.0 = file,
            _ => kept.push((file, at)),
        }
    }
    match kept.first_mut() {
        Some(first) => first.1 = 0,
        None => return Err(t!("design.flows.no_frames").to_owned()),
    }
    let marks = raw.marks.into_iter().map(|m| Mark { screen: m.screen, at: on_clock(m.t) }).collect();
    Ok(Timeline { millis, frames: kept, marks })
}

/// The frames per second ffmpeg is given and makes.
const FPS: u64 = 30;

/// The frame showing at each tick of `FPS` from 0 to the end: Chrome paints
/// only when something changes, ffmpeg wants a frame every tick.
fn ticks(line: &Timeline) -> Vec<&Path> {
    let count = (line.millis * FPS).div_ceil(1000).max(1);
    let mut next = 0;
    (0..count)
        .map(|tick| {
            let now = tick * 1000 / FPS;
            while line.frames.get(next + 1).is_some_and(|(_, at)| *at <= now) {
                next += 1;
            }
            line.frames[next].0.as_path()
        })
        .collect()
}

/// The frames as `frames2mp4.swift` reads them: `<ms>\t<path>`, then `<end>\t`.
fn frame_list(line: &Timeline) -> String {
    let mut list: String = line.frames.iter().map(|(file, at)| format!("{at}\t{}\n", file.display())).collect();
    list.push_str(&format!("{}\t\n", line.millis.max(line.frames.last().map_or(0, |(_, at)| at + 34))));
    list
}

/// What makes the .mp4 (H.264, which QuickTime plays): ffmpeg when it is
/// installed, else AVFoundation through a Swift script — on any Mac with
/// the command line tools.
enum Encoder {
    Ffmpeg { program: PathBuf, x264: bool },
    Swift(PathBuf),
}

impl Encoder {
    fn find() -> Option<Encoder> {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let dirs = std::env::split_paths(&path).chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
        if let Some(program) = dirs.map(|d| d.join("ffmpeg")).find(|p| p.is_file()) {
            let encoders = std::process::Command::new(&program).args(["-hide_banner", "-encoders"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
            let x264 = String::from_utf8_lossy(&encoders.stdout).contains("libx264");
            return Some(Encoder::Ffmpeg { program, x264 });
        }
        // `/usr/bin/swift` is there on every Mac, but runs only with the tools installed.
        let swift = std::process::Command::new("xcrun").args(["--find", "swift"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok().filter(|o| o.status.success())?;
        Some(Encoder::Swift(PathBuf::from(String::from_utf8_lossy(&swift.stdout).trim())))
    }

    async fn encode(&self, line: &Timeline, work: &Path, out: &Path) -> Result<(), String> {
        let (program, args) = match self {
            Encoder::Ffmpeg { program, x264 } => {
                // The frame at each tick, linked (not copied) in order.
                let ticked = work.join("ticks");
                std::fs::create_dir_all(&ticked).map_err(|e| format!("{}: {e}", ticked.display()))?;
                for (n, frame) in ticks(line).into_iter().enumerate() {
                    let to = ticked.join(format!("{:06}.jpg", n + 1));
                    std::fs::hard_link(frame, &to).or_else(|_| std::fs::copy(frame, &to).map(drop)).map_err(|e| format!("{}: {e}", to.display()))?;
                }
                (program.clone(), ffmpeg_args(&ticked.join("%06d.jpg"), out, *x264))
            }
            Encoder::Swift(swift) => {
                let (list, script) = (work.join("frames.txt"), work.join("frames2mp4.swift"));
                std::fs::write(&list, frame_list(line)).map_err(|e| format!("{}: {e}", list.display()))?;
                std::fs::write(&script, include_str!("../assets/design/frames2mp4.swift")).map_err(|e| format!("{}: {e}", script.display()))?;
                (swift.clone(), [script.as_path(), list.as_path(), out].iter().map(|p| p.to_string_lossy().into_owned()).collect())
            }
        };
        let run = tokio::process::Command::new(&program).args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).output().await.map_err(|e| format!("{}: {e}", program.display()))?;
        if run.status.success() && out.is_file() {
            Ok(())
        } else {
            let said = String::from_utf8_lossy(&run.stderr);
            Err(t!("design.flows.encode_failed", program = program.display(), why = said.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default().trim()))
        }
    }
}

/// ffmpeg's arguments: the numbered frames at `FPS`, even-sized, in
/// limited-range yuv420p — what QuickTime plays — the index first for
/// playing as it loads.
fn ffmpeg_args(frames: &Path, out: &Path, x264: bool) -> Vec<String> {
    let mut args: Vec<String> = ["-y", "-hide_banner", "-loglevel", "error", "-framerate", &FPS.to_string(), "-i"].map(String::from).to_vec();
    args.push(frames.to_string_lossy().into_owned());
    args.extend(["-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2:out_range=tv,format=yuv420p", "-pix_fmt", "yuv420p"].map(String::from));
    let codec: &[&str] = if x264 { &["-c:v", "libx264", "-preset", "veryfast", "-crf", "22"] } else { &["-c:v", "h264_videotoolbox", "-b:v", "6M"] };
    args.extend(codec.iter().map(|s| s.to_string()));
    args.extend(["-movflags", "+faststart"].map(String::from));
    args.push(out.to_string_lossy().into_owned());
    args
}

/// A video put in place: where, how long, and where the one before went.
#[derive(Debug, PartialEq, Eq)]
struct Kept {
    video: PathBuf,
    millis: u64,
    before: Option<PathBuf>,
}

/// `made` put in place as the flow's current video at `mode` and `width`,
/// its sidecar beside it; the one before moved to history/ and history
/// pruned to `keep`.
fn keep_video(flow_dir: &Path, mode: &str, width: u32, made: Made, keep: usize) -> std::io::Result<Kept> {
    let before = shelve(flow_dir, mode, width)?;
    let video = flow_dir.join(flows::video_name(mode, width));
    std::fs::create_dir_all(flow_dir)?;
    if std::fs::rename(&made.video, &video).is_err() {
        // Another volume.
        std::fs::copy(&made.video, &video)?;
    }
    std::fs::write(video.with_extension("json"), serde_json::to_vec_pretty(&made.sidecar).unwrap_or_default())?;
    prune(flow_dir, mode, width, keep)?;
    // A history kept to none takes the one before too.
    let before = before.filter(|p| p.is_file());
    Ok(Kept { video, millis: made.sidecar.millis, before })
}

/// The current video at `mode` and `width`, and its sidecar, moved to
/// history/ under when it was made (renamed, so its time stays).
fn shelve(flow_dir: &Path, mode: &str, width: u32) -> std::io::Result<Option<PathBuf>> {
    let current = flow_dir.join(flows::video_name(mode, width));
    let Ok(meta) = std::fs::metadata(&current) else { return Ok(None) };
    let made = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    let history = flow_dir.join(flows::HISTORY);
    std::fs::create_dir_all(&history)?;
    let mut to = history.join(flows::history_name(&stamp(made), mode, width));
    // Two in the same second: the later a second on.
    let mut later = made;
    while to.exists() {
        later += 1;
        to = history.join(flows::history_name(&stamp(later), mode, width));
    }
    std::fs::rename(&current, &to)?;
    let sidecar = current.with_extension("json");
    if sidecar.is_file() {
        std::fs::rename(&sidecar, to.with_extension("json"))?;
    }
    Ok(Some(to))
}

/// The videos in history/ at `mode` and `width` past the newest `keep`
/// removed, with their sidecars; those removed.
fn prune(flow_dir: &Path, mode: &str, width: u32, keep: usize) -> std::io::Result<Vec<PathBuf>> {
    let history = flow_dir.join(flows::HISTORY);
    let Ok(entries) = std::fs::read_dir(&history) else { return Ok(Vec::new()) };
    let mut stamped: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            match flows::parse_video_name(&name) {
                Some((Some(stamp), m, w)) if m == mode && w == width => Some((stamp, e.path())),
                _ => None,
            }
        })
        .collect();
    stamped.sort_by(|a, b| b.0.cmp(&a.0));
    let gone: Vec<PathBuf> = stamped.into_iter().skip(keep).map(|(_, path)| path).collect();
    for path in &gone {
        std::fs::remove_file(path)?;
        let _ = std::fs::remove_file(path.with_extension("json"));
    }
    Ok(gone)
}

/// `secs` in local time as history names it: `20261104-051200`.
fn stamp(secs: u64) -> String {
    let Some(tm) = equipe_providers::platform::local_time(secs) else { return secs.to_string() };
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", tm.year, tm.month, tm.day, tm.hour, tm.minute, tm.second)
}

/// A video's length as a person reads it: `0:42`, `1:05`.
fn length(millis: u64) -> String {
    let secs = (millis + 500) / 1000;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// `design/flows/flows.json`.
fn map_file(design: &Path) -> PathBuf {
    design.join(flows::DIR).join(flows::FILE)
}

fn no_map(design: &Path) -> String {
    t!("design.flows.no_map", path = map_file(design).display())
}

/// The starter map written in `design`, unless there is one: its file, and
/// whether it was written.
fn init_map(design: &Path) -> Result<(PathBuf, bool), String> {
    let file = map_file(design);
    if file.exists() {
        return Ok((file, false));
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(&file, flows::starter(crate::i18n::lang())).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok((file, true))
}

/// The map in `design`, the repository it is in (if any) telling what is stale.
fn read_map(design: &Path) -> Result<FlowMap, String> {
    let repo = equipe_core::worktree::repo_root(design).ok();
    flows::read(design, repo.as_deref(), crate::i18n::lang()).ok_or_else(|| no_map(design))
}

/// `390,1280` as widths in px.
fn widths(text: &str) -> Result<Vec<u32>, String> {
    let bad = || t!("design.flows.bad_widths", widths = text);
    let widths = text.split(',').map(str::trim).filter(|w| !w.is_empty()).map(|w| w.parse::<u32>().ok().filter(|&w| w > 0).ok_or_else(bad)).collect::<Result<Vec<_>, _>>()?;
    if widths.is_empty() { Err(bad()) } else { Ok(widths) }
}

/// The modes a state is pictured in: light, and dark unless light only.
fn modes(light: bool) -> &'static [&'static str] {
    if light { &["light"] } else { &["light", "dark"] }
}

/// A picture for `render.mjs --plan` to take.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
struct Take {
    url: String,
    path: PathBuf,
    width: u32,
    scheme: &'static str,
    #[serde(flatten)]
    capture: flows::Capture,
}

/// A picture a state's command takes.
#[derive(Debug, PartialEq, Eq)]
struct Run {
    shot: flows::Shot,
    command: String,
    path: PathBuf,
    width: u32,
    mode: &'static str,
}

/// What `flows shoot` does: the pictures Chrome takes, those commands take,
/// and the states neither reaches — `flow › screen › state` and the pictures
/// to draw for each.
#[derive(Debug, Default)]
struct Plan {
    takes: Vec<Take>,
    runs: Vec<Run>,
    draw: Vec<(String, Vec<PathBuf>)>,
}

/// Each state of `all` the narrowing keeps (`flow`, `screen`), at every width
/// in every mode; nothing kept is an error.
fn plan(all: &[flows::Shot], flow: Option<&str>, screen: Option<&str>, widths: &[u32], light: bool) -> Result<Plan, String> {
    let kept: Vec<&flows::Shot> = all.iter().filter(|s| flow.is_none_or(|f| s.flow == f) && screen.is_none_or(|id| s.screen == id)).collect();
    if kept.is_empty() {
        let what = [flow.map(|f| format!("--flow {f}")), screen.map(|s| format!("--screen {s}"))].into_iter().flatten().collect::<Vec<_>>().join(" ");
        return Err(if what.is_empty() { t!("design.flows.no_states").to_owned() } else { t!("design.flows.nothing_matches", what = what) });
    }
    let mut plan = Plan::default();
    for shot in kept {
        let pictures = widths.iter().flat_map(|&width| modes(light).iter().map(move |&scheme| (width, scheme))).map(|(width, scheme)| (width, scheme, shot.dir.join(flows::picture_name(&shot.state, scheme, width))));
        match (&shot.command, &shot.url) {
            (Some(command), _) => plan.runs.extend(pictures.map(|(width, mode, path)| Run { shot: shot.clone(), command: command.clone(), path, width, mode })),
            (None, Some(url)) => plan.takes.extend(pictures.map(|(width, scheme, path)| Take { url: url.clone(), path, width, scheme, capture: shot.capture.clone() })),
            (None, None) => plan.draw.push((at(&shot.flow, Some(&shot.screen), Some(&shot.state)), pictures.map(|(_, _, path)| path).collect())),
        }
    }
    Ok(plan)
}

/// The repository root `design` is in: where the map's commands run, and
/// nowhere else.
fn command_root(design: &Path) -> Result<PathBuf, String> {
    let at = std::path::absolute(design).unwrap_or_else(|_| design.to_path_buf());
    equipe_core::worktree::repo_root(&at).map_err(|_| t!("design.flows.command_no_repo", path = at.display()))
}

/// Each of `runs` taken by its command at the root of `design`'s
/// repository, `jobs` at a time, each stopped after `limit`; what was
/// written said as it comes, what wasn't with why. How many didn't come out.
async fn shoot_by_commands(design: &Path, runs: &[Run], jobs: usize, limit: Duration) -> Result<usize, String> {
    use futures_util::StreamExt;
    let repo = command_root(design)?;
    eprintln!("{}", t!("design.flows.running", count = runs.len(), jobs = jobs.min(runs.len())));
    let failed = futures_util::stream::iter(runs)
        .map(|run| {
            let repo = &repo;
            async move {
                let taken = shoot_one(run, repo, limit).await;
                match &taken {
                    Ok(path) => println!("{}", t!("design.flows.wrote", path = path.display())),
                    Err(why) => eprintln!("✗ {} · {} · {} px: {why}", at(&run.shot.flow, Some(&run.shot.screen), Some(&run.shot.state)), mode_word(run.mode), run.width),
                }
                taken.is_err()
            }
        })
        .buffer_unordered(jobs.max(1))
        .filter(|failed| std::future::ready(*failed))
        .count()
        .await;
    Ok(failed)
}

/// One picture taken by its command, at the root of `repo`: written to a
/// hidden file beside it, checked to be a PNG, then put in its place — so
/// a failed run leaves the picture before as it was. Where it went.
async fn shoot_one(run: &Run, repo: &Path, limit: Duration) -> Result<PathBuf, String> {
    let path = std::path::absolute(&run.path).map_err(|e| format!("{}: {e}", run.path.display()))?;
    let dir = path.parent().unwrap_or(Path::new("/"));
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let fresh = dir.join(format!(".{}.{}.png", name.trim_end_matches(".png"), std::process::id()));
    let _ = std::fs::remove_file(&fresh);
    let values = flows::shot_values(&fresh, run.mode, run.width, &run.shot);
    let made = made_by(&flows::fill(&run.command, &values), &fresh, repo, &flows::env_of("EQUIPE_SHOT_", &values), limit, Want::Png).await;
    let placed = made.and_then(|()| std::fs::rename(&fresh, &path).map_err(|e| format!("{}: {e}", path.display())));
    let _ = std::fs::remove_file(&fresh);
    placed.map(|()| path)
}

/// What a command is to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Want {
    Png,
    Mp4,
}

impl Want {
    /// Whether `path` is one: `Err` says what is wrong.
    fn check(self, path: &Path) -> Result<(), String> {
        let meta = std::fs::metadata(path).ok().filter(|m| m.is_file() && m.len() > 0);
        let (missing, wrong) = match self {
            Want::Png => ("design.flows.command_no_picture", "design.flows.command_not_png"),
            Want::Mp4 => ("design.flows.command_no_video", "design.flows.command_not_mp4"),
        };
        let words = crate::i18n::catalog();
        let says = |key: &str| words.fill(key, &[("path", path.display().to_string())]);
        if meta.is_none() {
            return Err(says(missing));
        }
        let good = match self {
            Want::Png => image::ImageReader::open(path)
                .ok()
                .and_then(|r| r.with_guessed_format().ok())
                .filter(|r| r.format() == Some(image::ImageFormat::Png))
                .and_then(|r| r.into_dimensions().ok())
                .is_some_and(|(w, h)| w > 0 && h > 0),
            // An MP4 starts with its `ftyp` box.
            Want::Mp4 => std::fs::read(path).ok().is_some_and(|b| b.get(4..8) == Some(b"ftyp".as_slice())),
        };
        if good { Ok(()) } else { Err(says(wrong)) }
    }
}

/// `command` run at the root of `repo` with `env`, stopped after `limit`,
/// and what it was to write at `out` checked: `Err` says why it didn't
/// come out, with the last lines it printed to stderr.
async fn made_by(command: &str, out: &Path, repo: &Path, env: &[(String, String)], limit: Duration, want: Want) -> Result<(), String> {
    let ran = run_command(command, repo, env, limit).await?;
    let why = match ran.ended {
        Ended::TimedOut => Err(t!("design.flows.command_timed_out", secs = limit.as_secs_f32())),
        Ended::Exited(status) if !status.success() => Err(t!("design.flows.command_failed", status = status)),
        Ended::Exited(_) => want.check(out),
    };
    why.map_err(|why| if ran.said.is_empty() { why } else { format!("{why}\n{}", ran.said.lines().map(|l| format!("    | {l}")).collect::<Vec<_>>().join("\n")) })
}

/// How a command ended.
#[derive(Debug)]
enum Ended {
    Exited(std::process::ExitStatus),
    /// It ran past its limit and was stopped, with all it started.
    TimedOut,
}

/// A command run: how it ended, and the end of what it printed to stderr.
#[derive(Debug)]
struct Ran {
    ended: Ended,
    said: String,
}

/// How much of a command's stderr is kept, and how many of its last lines told.
const SAID_BYTES: usize = 16 << 10;
const SAID_LINES: usize = 12;

/// `command` run by `sh -c` at `cwd` with `env` added, in a process group
/// of its own: past `limit`, the group is sent SIGTERM, then SIGKILL two
/// seconds on — so what it started stops too.
async fn run_command(command: &str, cwd: &Path, env: &[(String, String)], limit: Duration) -> Result<Ran, String> {
    use tokio::io::AsyncReadExt;
    #[cfg(unix)]
    let mut process = {
        let mut process = tokio::process::Command::new("/bin/sh");
        process.arg("-c").arg(command);
        process.process_group(0);
        process
    };
    #[cfg(windows)]
    let mut process = {
        let mut process = tokio::process::Command::new("cmd.exe");
        process.args(["/D", "/S", "/C", command]);
        process
    };
    let mut child = process
        .current_dir(cwd)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("sh: {e}"))?;
    #[cfg(unix)]
    let group = child.id().map(|id| id as libc::pid_t);
    let mut stderr = child.stderr.take();
    let mut reader = tokio::spawn(async move {
        let mut kept: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 4096];
        while let Some(pipe) = stderr.as_mut() {
            match pipe.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    kept.extend_from_slice(&chunk[..n]);
                    if kept.len() > SAID_BYTES {
                        kept.drain(..kept.len() - SAID_BYTES);
                    }
                }
            }
        }
        kept
    });
    let ended = match tokio::time::timeout(limit, child.wait()).await {
        Ok(status) => Ended::Exited(status.map_err(|e| format!("sh: {e}"))?),
        Err(_) => {
            #[cfg(unix)]
            {
            let signal = |sig| {
                if let Some(group) = group {
                    // SAFETY: killpg only sends a signal; the group is the child's own, and the child isn't reaped yet.
                    unsafe { libc::killpg(group, sig) };
                }
            };
            signal(libc::SIGTERM);
            if tokio::time::timeout(Duration::from_secs(2), child.wait()).await.is_err() {
                signal(libc::SIGKILL);
            }
            let _ = child.wait().await;
            }
            #[cfg(windows)]
            {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            Ended::TimedOut
        }
    };
    // Whatever it left running may hold stderr open: wait for it a little only.
    let said = match tokio::time::timeout(Duration::from_secs(2), &mut reader).await {
        Ok(Ok(bytes)) => tail(&String::from_utf8_lossy(&bytes)),
        _ => {
            reader.abort();
            String::new()
        }
    };
    Ok(Ran { ended, said })
}

/// The last `SAID_LINES` lines of `text` with something on them.
fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(SAID_LINES)..].join("\n")
}

/// `onboarding › login › error`, as far as it goes.
fn at(flow: &str, screen: Option<&str>, state: Option<&str>) -> String {
    [Some(flow), screen, state].into_iter().flatten().collect::<Vec<_>>().join(" › ")
}

/// The states to draw instead of shoot, each with where its pictures go.
fn draw_list(draw: &[(String, Vec<PathBuf>)]) -> String {
    if draw.is_empty() {
        return String::new();
    }
    let mut text = format!("{}\n", t!("design.flows.draw_instead"));
    for (at, paths) in draw {
        text.push_str(&format!("  {at}\n"));
        for path in paths {
            text.push_str(&format!("    {}\n", path.display()));
        }
    }
    text
}

/// The map's states missing and screens stale, as its screens say.
fn missing_and_stale(map: &FlowMap) -> (usize, usize) {
    let screens = map.flows.iter().flat_map(|f| &f.screens);
    (screens.clone().map(|s| s.missing.len()).sum(), screens.filter(|s| s.stale).count())
}

/// Whether a state is missing (a screen with no pictures at all among them)
/// or a screen stale: `flows check` fails.
fn failing(map: &FlowMap, findings: &[flows::Finding]) -> bool {
    missing_and_stale(map) != (0, 0) || findings.iter().any(|f| matches!(f.kind, flows::FindingKind::MissingState | flows::FindingKind::Stale | flows::FindingKind::NoPictures))
}

/// A finding in a line, under its flow › screen.
fn finding_line(f: &flows::Finding) -> String {
    use flows::FindingKind::*;
    let state = f.state.as_deref().unwrap_or_default();
    match f.kind {
        MissingState => format!("✗ {}", t!("design.flows.finding.missing_state", state = state)),
        Stale => format!("✗ {}", t!("design.flows.finding.stale")),
        NoDark => format!("! {}", t!("design.flows.finding.no_dark", state = state)),
        NoPictures => format!("! {}", t!("design.flows.finding.no_pictures")),
        Unlisted if f.state.is_some() => format!("! {}", t!("design.flows.finding.unlisted_state", state = state)),
        Unlisted => format!("! {}", t!("design.flows.finding.unlisted")),
    }
}

/// `flows check`'s words: what couldn't be read, the findings by flow ›
/// screen in the order found, and the counts.
fn report(map: &FlowMap, findings: &[flows::Finding]) -> String {
    let mut text: String = map.problems.iter().map(|p| format!("! {p}\n")).collect();
    let mut groups: Vec<(String, Vec<&flows::Finding>)> = Vec::new();
    for f in findings {
        let head = at(&f.flow, f.screen.as_deref(), None);
        match groups.iter_mut().find(|(h, _)| *h == head) {
            Some((_, list)) => list.push(f),
            None => groups.push((head, vec![f])),
        }
    }
    for (head, list) in &groups {
        text.push_str(&format!("{head}\n"));
        for f in list {
            text.push_str(&format!("  {}\n", finding_line(f)));
        }
    }
    if findings.is_empty() {
        text.push_str(&format!("✓ {}\n", t!("design.flows.all_good")));
    }
    let screens = map.flows.iter().flat_map(|f| &f.screens);
    let states = screens.clone().flat_map(|s| &s.states);
    let (missing, stale) = missing_and_stale(map);
    let summary = t!(
        "design.flows.summary",
        screens = screens.count(),
        states = states.clone().count(),
        pictures = states.map(|s| s.pictures.len()).sum::<usize>(),
        missing = missing,
        stale = stale,
    );
    text.push_str(&format!("\n{summary}\n"));
    text
}

/// `flows list`'s words: each flow, its screens numbered in order with what
/// is stale or missing, each screen's states with the pictures they have.
fn outline(map: &FlowMap) -> String {
    let mut text: String = map.problems.iter().map(|p| format!("! {p}\n")).collect();
    if map.flows.is_empty() {
        text.push_str(&format!("{}\n", t!("design.flows.no_flows")));
    }
    for flow in &map.flows {
        let summary = flow.summary.as_deref().map(|s| format!(" · {s}")).unwrap_or_default();
        text.push_str(&format!("{} — {}{summary}\n", flow.id, flow.title));
        for (n, screen) in flow.screens.iter().enumerate() {
            let stale = if screen.stale { format!("  ✗ {}", t!("design.flows.stale")) } else { String::new() };
            text.push_str(&format!("  {}. {} — {}{stale}\n", n + 1, screen.id, screen.title));
            let width = screen.states.iter().map(|s| s.id.chars().count()).max().unwrap_or(0);
            for state in &screen.states {
                text.push_str(&format!("     {:<width$}  {} · {}\n", state.id, state.label, pictured(state)));
            }
            if !screen.missing.is_empty() {
                text.push_str(&format!("     ✗ {}\n", t!("design.flows.missing", what = screen.missing.join(", "))));
            }
        }
    }
    text
}

/// A state's pictures by mode: `light 390 1280 · dark 390`.
fn pictured(state: &ScreenState) -> String {
    let mut parts = Vec::new();
    for (mode, word) in [("light", t!("design.flows.light")), ("dark", t!("design.flows.dark"))] {
        let mut widths: Vec<u32> = state.pictures.iter().filter(|p| p.mode == mode).map(|p| p.width).collect();
        widths.sort_unstable();
        widths.dedup();
        if !widths.is_empty() {
            parts.push(format!("{word} {}", widths.iter().map(u32::to_string).collect::<Vec<_>>().join(" ")));
        }
    }
    if parts.is_empty() { t!("design.flows.no_pictures").to_owned() } else { parts.join(" · ") }
}

async fn critique_command(mut ask: critique::Ask, brief: Option<PathBuf>, engine: Option<String>) -> Result<(), String> {
    for p in ask.images() {
        if !p.is_file() {
            return Err(format!("{}: no such picture", p.display()));
        }
    }
    let design = PathBuf::from(equipe_core::design::DIR);
    let brief = brief.or_else(|| Some(design.join("brand.md")).filter(|p| p.is_file()));
    if let Some(file) = &brief {
        ask.brief = Some(std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?);
    }
    let first = reviewer_for(engine.as_deref(), "critique");
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default().to_string();
    let kept = critique::kept_at(&design, &stamp);
    if let Some(parent) = kept.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let (answer, by) = reviewed(first, |engine| critique_once(engine, &ask, &kept)).await?;
    // Kept with who wrote it and why that engine, for the person's screens.
    std::fs::write(&kept, format!("{}\n{}\n", reviewer::mark(&by), answer.trim_end())).map_err(|e| format!("{}: {e}", kept.display()))?;
    println!("{}\n", reviewer::said(&by));
    println!("{}", answer.trim_end());
    println!("\nkept {}", kept.display());
    Ok(())
}

/// One critique by `engine`: its answer.
async fn critique_once(engine: Engine, ask: &critique::Ask, kept: &Path) -> Result<String, NoReview> {
    let (program, args) = critique::command(engine, ask, kept);
    eprintln!("{} is critiquing {} picture(s)…", engine.label(), ask.pictures.len());
    let run = tokio::process::Command::new(&program).args(&args).stdin(Stdio::null()).stderr(Stdio::piped()).output().await.map_err(|e| NoReview::spawn(&program, engine, &e, ""))?;
    if !run.status.success() {
        return Err(NoReview::failed(engine, "critique", &run, ""));
    }
    // Codex wrote its answer to the file; Claude printed it.
    let answer = if engine == Engine::Claude { String::from_utf8_lossy(&run.stdout).into_owned() } else { std::fs::read_to_string(kept).unwrap_or_default() };
    if answer.trim().is_empty() {
        return Err(NoReview::Failed { told: format!("{} answered nothing", engine.label()), said: String::new() });
    }
    Ok(answer)
}

/// Why a reviewer gave no answer.
pub(crate) enum NoReview {
    /// Its program isn't here.
    Missing(String),
    /// It ran and failed: what to tell, and what it printed.
    Failed { told: String, said: String },
}

impl NoReview {
    /// Its program couldn't start (`then`: what to add to the words).
    pub(crate) fn spawn(program: &str, engine: Engine, err: &std::io::Error, then: &str) -> Self {
        let told = format!("{program}: {err} — is {} installed and signed in?{then}", engine.label());
        if err.kind() == std::io::ErrorKind::NotFound { NoReview::Missing(told) } else { NoReview::Failed { told, said: String::new() } }
    }

    /// It ran and failed: what it said, the end of it (`what`: the review it was asked for).
    pub(crate) fn failed(engine: Engine, what: &str, run: &std::process::Output, then: &str) -> Self {
        // Claude says why on stdout, Codex on stderr.
        let said = [String::from_utf8_lossy(&run.stderr), String::from_utf8_lossy(&run.stdout)].join("\n");
        if let Some(next) = critique::no_network(&said, &|n| std::env::var(n).ok()) {
            return NoReview::Failed { told: format!("{} couldn't {what}: {next}", engine.label()), said };
        }
        let tail: Vec<&str> = said.lines().filter(|l| !l.trim().is_empty()).rev().take(6).collect();
        let told = format!("{} couldn't {what} ({}): {}{then}", engine.label(), run.status, tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
        NoReview::Failed { told, said }
    }

    fn told(self) -> String {
        match self {
            NoReview::Missing(told) | NoReview::Failed { told, .. } => told,
        }
    }
}

/// Whether `engine`'s program is on this machine.
pub(crate) fn installed(engine: Engine) -> bool {
    let binary = match engine {
        Engine::Codex => equipe_core::engines::codex_binary(),
        Engine::Claude => equipe_providers::claude_code::DEFAULT_BINARY.to_owned(),
    };
    equipe_providers::codex_app::find(&binary).is_some()
}

/// Who reviews this command's design work (`what`): the engine named, the
/// daemon's word for this turn, or — nobody's word — decided here.
pub(crate) fn reviewer_for(named: Option<&str>, what: &str) -> Reviewed {
    let named = named.map(|n| if n == "claude" { Engine::Claude } else { Engine::Codex });
    let env = |name: &str| std::env::var(name).ok();
    let first = reviewer::resolve(named, env, installed);
    if named.is_none() && env(reviewer::REVIEWER_ENV).is_none() && critique::caller_engine(env).is_none() {
        eprintln!("{}", critique::unknown_caller(first.engine, what));
    }
    first
}

/// The review `run` gives on `first`'s engine; when that engine can't run
/// and the member's own may take over, on its own, fresh. The answer, and
/// who gave it.
pub(crate) async fn reviewed<F, Fut>(first: Reviewed, run: F) -> Result<(String, Reviewed), String>
where
    F: Fn(Engine) -> Fut,
    Fut: std::future::Future<Output = Result<String, NoReview>>,
{
    let failed = match run(first.engine).await {
        Ok(answer) => return Ok((answer, first)),
        Err(failed) => failed,
    };
    let missing = matches!(failed, NoReview::Missing(_));
    // A sandbox with no network stops every engine alike: said as it is.
    let sandboxed = matches!(&failed, NoReview::Failed { said, .. } if critique::no_network(said, &|n| std::env::var(n).ok()).is_some());
    let Some(next) = reviewer::fall_back(&first, missing).filter(|_| !sandboxed) else { return Err(failed.told()) };
    eprintln!("{}\n{} takes it instead, in a fresh session.", failed.told(), next.engine.label());
    run(next.engine).await.map(|answer| (answer, next)).map_err(NoReview::told)
}

/// Where the scripts and the Playwright they import live: the person's own,
/// made once. Chrome is the Mac's own (`channel: 'chrome'`), so no browser
/// is downloaded.
pub(crate) fn kit_dir() -> PathBuf {
    equipe_providers::credentials::equipe_home().join("design-kit")
}

const PLAYWRIGHT: &str = "playwright@1.64.0";

/// Node and npm beside the `npx` the squad's browser uses.
pub(crate) fn node_tools() -> Result<(PathBuf, PathBuf), String> {
    let npx = equipe_core::browser::find_npx().ok_or("Node.js isn't installed (no npx on PATH): `brew install node`")?;
    let bin = npx.parent().map(Path::to_path_buf).unwrap_or_default();
    Ok((bin.join("node"), bin.join("npm")))
}

async fn ensure_kit() -> Result<PathBuf, String> {
    let dir = kit_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Two first runs at once (a member's parallel commands) installed over each other.
    let _installing = crate::design_taste::Lock::take(dir.join(".install.lock"), Duration::from_secs(300)).await;
    if !dir.join("node_modules").join("playwright").join("package.json").is_file() {
        let (_, npm) = node_tools()?;
        eprintln!("Setting up the design tools once (Playwright, using this Mac's Chrome)…");
        let status = tokio::process::Command::new(&npm)
            .args(["install", "--prefix"])
            .arg(&dir)
            .args(["--no-audit", "--no-fund", "--loglevel=error", PLAYWRIGHT])
            .env("PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD", "1")
            .stdin(Stdio::null())
            .status()
            .await
            .map_err(|e| format!("npm: {e}"))?;
        if !status.success() {
            return Err("couldn't install Playwright for the design tools".into());
        }
    }
    for (name, body) in [
        ("render.mjs", include_str!("../assets/design/render.mjs")),
        ("audit.mjs", include_str!("../assets/design/audit.mjs")),
        ("diff.mjs", include_str!("../assets/design/diff.mjs")),
        ("record.mjs", include_str!("../assets/design/record.mjs")),
        ("lint.mjs", include_str!("../assets/design/lint.mjs")),
        ("refs.mjs", include_str!("../assets/design/refs.mjs")),
        ("matrix.mjs", include_str!("../assets/design/matrix.mjs")),
    ] {
        std::fs::write(dir.join(name), body).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

/// The language a brand's site is read in: the one asked for, else the
/// person's — a language tag Chrome sends as `Accept-Language`.
fn audit_locale(asked: Option<String>) -> String {
    asked.unwrap_or_else(|| crate::i18n::lang().code().to_owned())
}

pub(crate) async fn run_script(name: &str, args: &[&str]) -> Result<(), String> {
    let dir = ensure_kit().await?;
    let (node, _) = node_tools()?;
    let status = tokio::process::Command::new(&node).arg(dir.join(name)).args(args).stdin(Stdio::null()).status().await.map_err(|e| format!("node: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{name} stopped ({status})")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use equipe_core::protocol::design::Picture;
    use flows::{Finding, FindingKind};

    #[derive(Parser)]
    struct Args {
        #[command(subcommand)]
        action: DesignAction,
    }

    #[test]
    fn a_brand_is_audited_in_the_persons_language_unless_told_another() {
        let DesignAction::Audit { locale, .. } = Args::parse_from(["x", "audit", "acme.com"]).action else { panic!() };
        assert_eq!(audit_locale(locale), crate::i18n::lang().code(), "the language in use, none built in");
        let DesignAction::Audit { locale, .. } = Args::parse_from(["x", "audit", "acme.com", "--locale", "pt-BR"]).action else { panic!() };
        assert_eq!(audit_locale(locale), "pt-BR");
        let script = include_str!("../assets/design/audit.mjs");
        assert!(script.contains("opt('--locale', 'en-US')") && script.contains("locale })"), "the page is read in the locale asked for");
    }

    fn flows_args(words: &[&str]) -> FlowsAction {
        let words = ["design", "flows"].into_iter().chain(words.iter().copied());
        match Args::try_parse_from(words).unwrap_or_else(|e| panic!("{e}")).action {
            DesignAction::Flows { action } => action,
            _ => panic!("not flows"),
        }
    }

    /// A 1×1 PNG.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1F, 0x15, 0xC4, 0x89, 0, 0, 0, 0x0A, 0x49, 0x44,
        0x41, 0x54, 0x78, 0x9C, 0x63, 0, 1, 0, 0, 5, 0, 1, 0x0D, 0x0A, 0x2D, 0xB4, 0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    const MAP: &str = r#"{ "flows": [
      { "id": "onboarding", "title": "Onboarding", "summary": "From the first run to the first project",
        "screens": [
          { "id": "login", "title": "Log in", "url": "http://localhost:3000/login", "expects": ["empty", "error"],
            "states": [
              { "id": "default", "label": "Default", "kind": "default" },
              { "id": "error", "label": "Error", "kind": "error", "url": "http://localhost:3000/login?fixture=wrong-password" } ] },
          { "id": "welcome", "title": "Welcome",
            "states": [ { "id": "first", "label": "First time", "kind": "first" } ] } ] } ] }"#;

    /// A design folder with `MAP` and `pictures` (relative to `flows/`).
    fn design(pictures: &[&str]) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("equipe-cli-flows-{}-{n}", std::process::id())).join("design");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(flows::DIR)).unwrap();
        std::fs::write(map_file(&dir), MAP).unwrap();
        for picture in pictures {
            let path = dir.join(flows::DIR).join(picture);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, PNG).unwrap();
        }
        dir
    }

    /// The map as `flows::read` gives it, made here from the folder so the
    /// words are tested on their own: its pictures found, `missing` and
    /// `stale` as given.
    fn map_of(design: &Path, missing: &[&str], stale: bool) -> FlowMap {
        let mut map: FlowMap = serde_json::from_str(&std::fs::read_to_string(map_file(design)).unwrap()).unwrap();
        for flow in &mut map.flows {
            for screen in &mut flow.screens {
                let folder = design.join(flows::DIR).join(&flow.id).join(&screen.id);
                let mut names: Vec<String> = std::fs::read_dir(&folder).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
                names.sort();
                for name in names {
                    let Some((state, mode, width)) = flows::parse_picture_name(&name) else { continue };
                    if let Some(s) = screen.states.iter_mut().find(|s| s.id == state) {
                        s.pictures.push(Picture { path: folder.join(&name).to_string_lossy().into_owned(), mode, width, taken: 0, ..Default::default() });
                    }
                }
                if screen.id == "login" {
                    screen.missing = missing.iter().map(|m| m.to_string()).collect();
                    screen.stale = stale;
                }
            }
        }
        map
    }

    fn finding(screen: Option<&str>, state: Option<&str>, kind: FindingKind) -> Finding {
        Finding { flow: "onboarding".into(), screen: screen.map(Into::into), state: state.map(Into::into), kind }
    }

    #[test]
    fn the_flows_commands_take_their_flags() {
        assert!(matches!(flows_args(&["init"]), FlowsAction::Init { dir: None }));
        assert!(matches!(flows_args(&["init", "--dir", "web/design"]), FlowsAction::Init { dir: Some(d) } if d == Path::new("web/design")));
        assert!(matches!(flows_args(&["check", "--json"]), FlowsAction::Check { dir: None, json: true }));
        assert!(matches!(flows_args(&["list", "--dir", "d"]), FlowsAction::List { dir: Some(_) }));
        match flows_args(&["shoot"]) {
            FlowsAction::Shoot { dir, flow, screen, widths, light, jobs, timeout_secs } => {
                assert_eq!((dir, flow, screen, widths.as_str(), light, jobs, timeout_secs), (None, None, None, "390,1280", false, 1, 120));
            }
            _ => panic!("shoot"),
        }
        match flows_args(&["shoot", "--flow", "onboarding", "--screen", "login", "--widths", "768", "--light", "--dir", "d", "--jobs", "4", "--timeout-secs", "30"]) {
            FlowsAction::Shoot { dir, flow, screen, widths, light, jobs, timeout_secs } => {
                assert_eq!((dir.as_deref(), flow.as_deref(), screen.as_deref(), widths.as_str(), light, jobs, timeout_secs), (Some(Path::new("d")), Some("onboarding"), Some("login"), "768", true, 4, 30));
            }
            _ => panic!("shoot"),
        }
        assert!(Args::try_parse_from(["design", "flows", "shoot", "onboarding"]).is_err(), "the flow is a flag");
        assert!(Args::try_parse_from(["design", "flows", "shoot", "--jobs", "0"]).is_err(), "at least one at a time");
        assert!(Args::try_parse_from(["design", "flows", "shoot", "--timeout-secs", "0"]).is_err(), "some time to run");
    }

    #[test]
    fn widths_are_px_comma_separated() {
        assert_eq!(widths("390,1280"), Ok(vec![390, 1280]));
        assert_eq!(widths(" 390, 768 ,"), Ok(vec![390, 768]));
        assert!(widths("390,wide").unwrap_err().contains("390,wide"));
        assert!(widths("0").is_err());
        assert!(widths("").is_err());
    }

    #[test]
    fn each_state_is_shot_at_each_width_light_and_dark_into_its_screens_folder() {
        let dir = design(&[]);
        let login = dir.join("flows/onboarding/login");
        let shot = |screen: &str, state: &str, url: Option<&str>| flows::Shot { flow: "onboarding".into(), screen: screen.into(), state: state.into(), url: url.map(Into::into), command: None, dir: dir.join("flows/onboarding").join(screen), capture: flows::Capture::default() };
        let all = [shot("login", "default", Some("http://localhost:3000/login")), shot("login", "error", Some("http://localhost:3000/login?fixture=wrong-password")), shot("welcome", "first", None)];

        let plan = plan(&all, None, None, &[390, 1280], false).unwrap();
        let taken: Vec<(&str, PathBuf, u32, &str)> = plan.takes.iter().map(|t| (t.url.as_str(), t.path.clone(), t.width, t.scheme)).collect();
        assert_eq!(taken.len(), 8);
        assert_eq!(taken[0], ("http://localhost:3000/login", login.join("default.light.390.png"), 390, "light"));
        assert_eq!(taken[1], ("http://localhost:3000/login", login.join("default.dark.390.png"), 390, "dark"));
        assert_eq!(taken[3], ("http://localhost:3000/login", login.join("default.dark.1280.png"), 1280, "dark"));
        assert_eq!(taken[4].0, "http://localhost:3000/login?fixture=wrong-password");
        assert_eq!(taken[4].1, login.join("error.light.390.png"));
        let wire = serde_json::to_value(&plan.takes[0]).unwrap();
        assert_eq!(wire["scheme"], "light");
        assert_eq!(wire["width"], 390);
        assert!(wire.get("locale").is_none() && wire.get("actions").is_none(), "unset, left to render.mjs: {wire}");
        let mut asked = all[0].clone();
        asked.capture = flows::Capture { browse: flows::Browse { locale: Some("ko-KR".into()), ..Default::default() }, js: Some(false), full_page: Some(false), selector: Some("#faq".into()), actions: vec![serde_json::json!({ "click": "summary" })], ..Default::default() };
        let wire = serde_json::to_value(&super::plan(&[asked], None, None, &[390], true).unwrap().takes[0]).unwrap();
        assert_eq!((&wire["locale"], &wire["js"], &wire["fullPage"], &wire["selector"]), (&serde_json::json!("ko-KR"), &serde_json::json!(false), &serde_json::json!(false), &serde_json::json!("#faq")));
        assert_eq!(wire["actions"], serde_json::json!([{ "click": "summary" }]));

        // A state with no URL is drawn instead, to the paths it would have.
        let welcome = dir.join("flows/onboarding/welcome");
        assert_eq!(plan.draw, vec![("onboarding › welcome › first".to_owned(), vec![welcome.join("first.light.390.png"), welcome.join("first.dark.390.png"), welcome.join("first.light.1280.png"), welcome.join("first.dark.1280.png")])]);
        let words = draw_list(&plan.draw);
        assert!(words.starts_with("Neither a URL nor a command reaches these — draw them instead"), "{words}");
        assert!(words.contains("\n  onboarding › welcome › first\n"), "{words}");
        assert!(words.contains(&format!("\n    {}\n", welcome.join("first.dark.1280.png").display())), "{words}");
        assert_eq!(draw_list(&[]), "");

        // Narrowed, and light only.
        let narrowed = super::plan(&all, Some("onboarding"), Some("login"), &[768], true).unwrap();
        assert_eq!(narrowed.takes.iter().map(|t| t.path.clone()).collect::<Vec<_>>(), vec![login.join("default.light.768.png"), login.join("error.light.768.png")]);
        assert!(narrowed.draw.is_empty() && narrowed.runs.is_empty());
        assert_eq!(super::plan(&all, None, Some("welcome"), &[390], true).unwrap().draw.len(), 1);
        assert_eq!(super::plan(&all, Some("checkout"), None, &[390], false).unwrap_err(), "Nothing in the map matches --flow checkout.");
        assert_eq!(super::plan(&all, Some("onboarding"), Some("nope"), &[390], false).unwrap_err(), "Nothing in the map matches --flow onboarding --screen nope.");
        assert_eq!(super::plan(&[], None, None, &[390], false).unwrap_err(), "The map lists no states to picture yet.");
    }

    #[test]
    fn init_writes_the_starter_once_and_never_over_a_map() {
        let dir = design(&[]).parent().unwrap().join("fresh");
        let (file, written) = init_map(&dir).unwrap();
        assert!(written);
        assert_eq!(file, dir.join("flows/flows.json"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), flows::starter(equipe_core::i18n::Lang::En));
        std::fs::write(&file, MAP).unwrap();
        assert_eq!(init_map(&dir).unwrap(), (file.clone(), false));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), MAP, "the person's map is kept");
        assert_eq!(t!("design.flows.init_there", path = file.display()), format!("{} is there already; nothing written.", file.display()));
    }

    #[test]
    fn a_folder_with_no_map_says_how_to_start_one() {
        let dir = design(&[]).parent().unwrap().join("empty");
        std::fs::create_dir_all(&dir).unwrap();
        let said = read_map(&dir).unwrap_err();
        assert_eq!(said, format!("No UI map at {} — `equipe design flows init` starts one.", dir.join("flows/flows.json").display()));
    }

    #[test]
    fn check_lists_findings_by_flow_and_screen_then_counts() {
        let dir = design(&["onboarding/login/default.light.390.png", "onboarding/login/default.dark.390.png", "onboarding/login/default.light.1280.png", "onboarding/login/eror.dark.390.png"]);
        let map = map_of(&dir, &["empty", "error"], true);
        let findings = vec![
            finding(Some("login"), Some("empty"), FindingKind::MissingState),
            finding(Some("welcome"), None, FindingKind::NoPictures),
            finding(Some("login"), Some("error"), FindingKind::MissingState),
            finding(Some("login"), None, FindingKind::Stale),
            finding(Some("login"), Some("default"), FindingKind::NoDark),
            finding(Some("login"), Some("eror"), FindingKind::Unlisted),
            finding(Some("signup"), None, FindingKind::Unlisted),
        ];
        assert_eq!(
            report(&map, &findings),
            "onboarding › login\n  ✗ empty: not pictured\n  ✗ error: not pictured\n  ✗ its code changed after its pictures were taken\n  ! default: no dark picture beside a light one\n  ! eror: pictured, but not in flows.json\nonboarding › welcome\n  ! no pictures yet\nonboarding › signup\n  ! a folder of pictures not in flows.json\n\nscreens 2 · states 3 · pictures 3 · missing 2 · stale 1\n"
        );
        assert!(failing(&map, &findings));
        let pictured = map_of(&dir, &[], false);
        assert!(!failing(&pictured, &[findings[4].clone(), findings[5].clone(), findings[6].clone()]), "no dark and unlisted pictures don't fail the check");
        assert!(failing(&pictured, &findings[1..2]), "nor does a screen with no pictures pass");
        assert!(failing(&map, &[]), "the map's own missing and stale fail it");
        let json = serde_json::to_value(&findings[0]).unwrap();
        assert_eq!(json, serde_json::json!({ "flow": "onboarding", "screen": "login", "state": "empty", "kind": "missing_state" }));
    }

    #[test]
    fn the_folder_read_and_checked_as_flows_check_says_it() {
        let dir = design(&["onboarding/login/default.light.390.png", "onboarding/login/default.dark.390.png", "onboarding/login/default.light.1280.png", "onboarding/login/eror.dark.390.png"]);
        let map = read_map(&dir).unwrap();
        let findings = flows::check(&map);
        let said = report(&map, &findings);
        for line in ["onboarding › login\n", "  ✗ empty: not pictured\n", "  ✗ error: not pictured\n", "  ! default: no dark picture beside a light one\n", "  ! eror: pictured, but not in flows.json\n", "onboarding › welcome\n", "  ! no pictures yet\n"] {
            assert!(said.contains(line), "{line:?} in\n{said}");
        }
        assert!(said.ends_with("\nscreens 2 · states 3 · pictures 3 · missing 3 · stale 0\n"), "the welcome screen's state too\n{said}");
        assert!(failing(&map, &findings));
        let listed = outline(&map);
        assert!(listed.contains("     default  Default · light 390 1280 · dark 390\n"), "{listed}");
    }

    #[test]
    fn a_map_all_pictured_says_so() {
        let mut map = map_of(&design(&["onboarding/login/default.light.390.png"]), &[], false);
        map.problems.push("onboarding › login: an id with a space".into());
        assert_eq!(report(&map, &[]), "! onboarding › login: an id with a space\n✓ Every state is pictured, light and dark, and up to date.\n\nscreens 2 · states 3 · pictures 1 · missing 0 · stale 0\n");
    }

    #[test]
    fn list_outlines_each_flow_its_screens_in_order_and_their_states() {
        let dir = design(&["onboarding/login/default.light.390.png", "onboarding/login/default.dark.390.png", "onboarding/login/default.light.1280.png", "onboarding/welcome/first.dark.390.png"]);
        let map = map_of(&dir, &["empty", "error"], true);
        assert_eq!(
            outline(&map),
            "onboarding — Onboarding · From the first run to the first project\n  1. login — Log in  ✗ stale\n     default  Default · light 390 1280 · dark 390\n     error    Error · no pictures\n     ✗ missing: empty, error\n  2. welcome — Welcome\n     first  First time · dark 390\n"
        );
        assert_eq!(outline(&FlowMap::default()), "The map has no flows yet.\n");
    }

    #[test]
    fn record_takes_a_flow_widths_one_mode_and_how_many_to_keep() {
        match flows_args(&["record"]) {
            FlowsAction::Record { dir, flow, widths, light, dark, keep, timeout_secs } => {
                assert_eq!((dir, flow, widths.as_str(), light, dark, keep, timeout_secs), (None, None, "390", false, false, 5, 600));
                assert_eq!(record_modes(light, dark), ["light", "dark"]);
            }
            _ => panic!("record"),
        }
        match flows_args(&["record", "--flow", "onboarding", "--widths", "390,1280", "--dark", "--keep", "2", "--dir", "d", "--timeout-secs", "90"]) {
            FlowsAction::Record { dir, flow, widths, light, dark, keep, timeout_secs: 90 } => {
                assert_eq!((dir.as_deref(), flow.as_deref(), widths.as_str(), keep), (Some(Path::new("d")), Some("onboarding"), "390,1280", 2));
                assert_eq!(record_modes(light, dark), ["dark"]);
            }
            _ => panic!("record"),
        }
        assert_eq!(record_modes(true, false), ["light"]);
        assert!(Args::try_parse_from(["design", "flows", "record", "--light", "--dark"]).is_err(), "one mode or both");
    }

    #[test]
    fn a_walk_becomes_calls_with_each_screen_marked_and_captioned_where_it_begins() {
        use flows::{Step, WalkStep};
        let step = |screen: Option<&str>, step: Step| WalkStep { screen: screen.map(Into::into), step };
        let steps = [
            step(Some("login"), Step::Go("http://localhost:3000/login".into())),
            step(None, Step::Fill("input[name=\"email\"]".into(), "kim@example.com".into())),
            step(None, Step::Say("A wrong password: \"it's\" shown under the field".into())),
            step(None, Step::Press("Enter".into())),
            step(None, Step::Wait(800)),
            step(Some("welcome"), Step::Click("text=Start".into())),
            step(Some("gone"), Step::Wait(1)),
        ];
        let titles = [("login".to_owned(), "Log in".to_owned()), ("welcome".to_owned(), "Welcome \u{2028}".to_owned())].into_iter().collect();
        assert_eq!(
            walk_script(&steps, &titles),
            "// Written by `equipe design flows record` from flows.json.\nexport default async (w) => {\n  await w.screen(\"login\", \"Log in\");\n  await w.go(\"http://localhost:3000/login\");\n  await w.fill(\"input[name=\\\"email\\\"]\", \"kim@example.com\");\n  await w.say(\"A wrong password: \\\"it's\\\" shown under the field\");\n  await w.press(\"Enter\");\n  await w.wait(800);\n  await w.screen(\"welcome\", \"Welcome \u{2028}\");\n  await w.click(\"text=Start\");\n  await w.screen(\"gone\", \"gone\");\n  await w.wait(1);\n};\n"
        );
    }

    #[test]
    fn a_walk_is_recorded_in_en_us_unless_its_flow_says_its_locale() {
        let (walk, frames, raw) = (Path::new("/w/walk.mjs"), Path::new("/w/frames"), Path::new("/w/raw.json"));
        let plan = record_plan(walk, frames, raw, &flows::Browse::default(), 390, "dark");
        assert_eq!((&plan["locale"], &plan["reducedMotion"], &plan["permissions"]), (&serde_json::json!("en-US"), &serde_json::json!(false), &serde_json::json!([])));
        assert_eq!((&plan["width"], &plan["height"], &plan["scale"], &plan["scheme"]), (&serde_json::json!(390), &serde_json::json!(844), &serde_json::json!(2), &serde_json::json!("dark")));
        assert_eq!(plan["walk"], "/w/walk.mjs");
        let ko = flows::Browse { locale: Some("ko-KR".into()), reduced_motion: Some(true), permissions: vec!["clipboard-write".into()] };
        let plan = record_plan(walk, frames, raw, &ko, 1280, "light");
        assert_eq!((&plan["locale"], &plan["reducedMotion"], &plan["permissions"]), (&serde_json::json!("ko-KR"), &serde_json::json!(true), &serde_json::json!(["clipboard-write"])));
    }

    fn raw(start: u64, end: u64, frames: &[(&str, u64)], marks: &[(&str, u64)]) -> Raw {
        Raw { start, end, frames: frames.iter().map(|(f, t)| RawFrame { file: PathBuf::from(f), t: *t }).collect(), marks: marks.iter().map(|(s, t)| RawMark { screen: s.to_string(), t: *t }).collect() }
    }

    #[test]
    fn marks_and_frames_are_timed_from_when_filming_began() {
        let line = timeline(raw(10_000, 14_500, &[("/f/3.jpg", 11_200), ("/f/1.jpg", 10_040), ("/f/2.jpg", 11_200), ("/f/4.jpg", 20_000)], &[("login", 9_000), ("login", 10_000), ("welcome", 12_345), ("late", 99_999)])).unwrap();
        assert_eq!(line.millis, 4_500);
        assert_eq!(line.frames, vec![(PathBuf::from("/f/1.jpg"), 0), (PathBuf::from("/f/2.jpg"), 1_200), (PathBuf::from("/f/4.jpg"), 4_500)], "the first from 0; the later of two in one ms; none past the end");
        let at: Vec<(&str, u64)> = line.marks.iter().map(|m| (m.screen.as_str(), m.at)).collect();
        assert_eq!(at, [("login", 0), ("login", 0), ("welcome", 2_345), ("late", 4_500)]);
        assert_eq!(serde_json::to_value(Sidecar { millis: line.millis, marks: line.marks.clone() }).unwrap(), serde_json::json!({ "millis": 4500, "marks": [{ "screen": "login", "at": 0 }, { "screen": "login", "at": 0 }, { "screen": "welcome", "at": 2345 }, { "screen": "late", "at": 4500 }] }));
        assert_eq!(timeline(raw(1, 2, &[], &[])).unwrap_err(), "Chrome painted nothing to record");

        let line = timeline(raw(0, 2_000, &[("/f/1.jpg", 0), ("/f/2.jpg", 505), ("/f/3.jpg", 510), ("/f/4.jpg", 1_960)], &[])).unwrap();
        let ticked = ticks(&line);
        assert_eq!(ticked.len(), 60, "2 s at 30 fps");
        let shown = |file: &str| ticked.iter().filter(|p| **p == Path::new(file)).count();
        assert_eq!((shown("/f/1.jpg"), shown("/f/2.jpg"), shown("/f/3.jpg"), shown("/f/4.jpg")), (16, 0, 43, 1), "each shown from its tick on; one replaced between ticks never");
        assert_eq!((ticked[15], ticked[16]), (Path::new("/f/1.jpg"), Path::new("/f/3.jpg")));
        assert_eq!(ticks(&timeline(raw(0, 0, &[("/f/1.jpg", 0)], &[])).unwrap()), [Path::new("/f/1.jpg")], "a video of no time still has its frame");
        let line = timeline(raw(0, 2_000, &[("/f/it's.jpg", 0), ("/f/2.jpg", 500)], &[])).unwrap();
        assert_eq!(frame_list(&line), "0\t/f/it's.jpg\n500\t/f/2.jpg\n2000\t\n");
        let args = ffmpeg_args(Path::new("/w/ticks/%06d.jpg"), Path::new("/w/out.mp4"), true);
        assert_eq!(args.join(" "), "-y -hide_banner -loglevel error -framerate 30 -i /w/ticks/%06d.jpg -vf scale=trunc(iw/2)*2:trunc(ih/2)*2:out_range=tv,format=yuv420p -pix_fmt yuv420p -c:v libx264 -preset veryfast -crf 22 -movflags +faststart /w/out.mp4");
        assert!(ffmpeg_args(Path::new("l"), Path::new("o"), false).join(" ").contains("-c:v h264_videotoolbox"));
        assert_eq!((length(0), length(41_600), length(65_000)), ("0:00".to_owned(), "0:42".to_owned(), "1:05".to_owned()));
        assert_eq!((viewport(390), viewport(1280)), ((844, 2), (800, 1)));
    }

    #[test]
    fn a_new_video_moves_the_one_before_into_history_which_keeps_the_newest() {
        let flow = design(&[]).join("flows/onboarding");
        let history = flow.join(flows::HISTORY);
        std::fs::create_dir_all(&history).unwrap();
        let stamps = ["20261101-090000", "20261102-090000", "20261103-090000"];
        for stamp in stamps {
            std::fs::write(history.join(flows::history_name(stamp, "light", 390)), b"old").unwrap();
            std::fs::write(history.join(flows::history_name(stamp, "light", 390)).with_extension("json"), b"{}").unwrap();
        }
        std::fs::write(history.join(flows::history_name("20261101-080000", "dark", 390)), b"dark").unwrap();
        std::fs::write(flow.join("walkthrough.light.390.mp4"), b"current").unwrap();
        std::fs::write(flow.join("walkthrough.light.390.json"), br#"{"millis":1,"marks":[]}"#).unwrap();
        let work = flow.join("work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("walkthrough.mp4"), b"new").unwrap();
        let made = Made { video: work.join("walkthrough.mp4"), sidecar: Sidecar { millis: 42_000, marks: vec![Mark { screen: "login".into(), at: 0 }] } };

        let kept = keep_video(&flow, "light", 390, made, 2).unwrap();
        assert_eq!(kept.video, flow.join("walkthrough.light.390.mp4"));
        assert_eq!(kept.millis, 42_000);
        assert_eq!(std::fs::read(&kept.video).unwrap(), b"new");
        assert_eq!(std::fs::read_to_string(flow.join("walkthrough.light.390.json")).unwrap().split_whitespace().collect::<String>(), r#"{"millis":42000,"marks":[{"screen":"login","at":0}]}"#);
        let before = kept.before.unwrap();
        let name = before.file_name().unwrap().to_string_lossy().into_owned();
        let (stamp, mode, width) = flows::parse_video_name(&name).unwrap();
        assert_eq!((mode.as_str(), width), ("light", 390));
        let stamp = stamp.unwrap();
        assert!(stamp.len() == 15 && stamp.as_bytes()[8] == b'-' && stamp.starts_with("20"), "{stamp}");
        assert_eq!(std::fs::read(&before).unwrap(), b"current");
        assert!(before.with_extension("json").is_file(), "its marks went with it");

        let mut left: Vec<String> = std::fs::read_dir(&history).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        let mut want = vec![flows::history_name("20261101-080000", "dark", 390), flows::history_name("20261103-090000", "light", 390), "walkthrough.20261103-090000.light.390.json".to_owned(), name.clone(), name.replace(".mp4", ".json")];
        want.sort();
        assert_eq!(left, want, "the newest two light 390s, the dark one untouched");

        // The same second twice: the later a second on.
        std::fs::write(flow.join("walkthrough.light.390.mp4"), b"again").unwrap();
        let next = shelve(&flow, "light", 390).unwrap().unwrap();
        assert_ne!(next, before);
        assert!(prune(&flow, "light", 390, 0).unwrap().len() >= 2);
        assert_eq!(shelve(&flow, "light", 390).unwrap(), None, "nothing current to move");
        assert_eq!(super::stamp(1_793_000_000).len(), 15);
    }

    #[test]
    fn a_states_command_is_run_for_each_picture_and_wins_over_its_url() {
        let dir = design(&[]);
        let login = dir.join("flows/onboarding/login");
        let shot = |state: &str, url: Option<&str>, command: Option<&str>| flows::Shot { flow: "onboarding".into(), screen: "login".into(), state: state.into(), url: url.map(Into::into), command: command.map(Into::into), dir: login.clone(), capture: flows::Capture::default() };
        let all = [shot("default", Some("http://localhost:3000/login"), Some("scripts/shot.sh {mode} {width} {out}")), shot("error", Some("http://localhost:3000/login?e=1"), None), shot("long", None, Some("scripts/long.sh {out}"))];
        let plan = plan(&all, None, None, &[1280], false).unwrap();
        let runs: Vec<(&str, &str, PathBuf, u32, &str)> = plan.runs.iter().map(|r| (r.shot.state.as_str(), r.command.as_str(), r.path.clone(), r.width, r.mode)).collect();
        assert_eq!(
            runs,
            [
                ("default", "scripts/shot.sh {mode} {width} {out}", login.join("default.light.1280.png"), 1280, "light"),
                ("default", "scripts/shot.sh {mode} {width} {out}", login.join("default.dark.1280.png"), 1280, "dark"),
                ("long", "scripts/long.sh {out}", login.join("long.light.1280.png"), 1280, "light"),
                ("long", "scripts/long.sh {out}", login.join("long.dark.1280.png"), 1280, "dark"),
            ]
        );
        assert_eq!(plan.takes.iter().map(|t| t.url.as_str()).collect::<Vec<_>>(), ["http://localhost:3000/login?e=1", "http://localhost:3000/login?e=1"], "only the state with no command opens its URL");
        assert!(plan.draw.is_empty());
    }

    /// A repository in a folder of its own, with a design folder holding `MAP`.
    fn repo() -> (PathBuf, PathBuf) {
        let design = design(&[]);
        let root = design.parent().unwrap().canonicalize().unwrap();
        let git = std::process::Command::new("git").args(["init", "-q"]).current_dir(&root).status().unwrap();
        assert!(git.success());
        std::fs::write(root.join("fixture.png"), PNG).unwrap();
        (root.clone(), root.join("design"))
    }

    fn run_of(design: &Path, command: &str, mode: &'static str) -> Run {
        let dir = design.join("flows/onboarding/login");
        let shot = flows::Shot { flow: "onboarding".into(), screen: "login".into(), state: "default".into(), url: None, command: Some(command.into()), dir: dir.clone(), capture: flows::Capture::default() };
        Run { shot, command: command.into(), path: dir.join(flows::picture_name("default", mode, 1280)), width: 1280, mode }
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with('.')).collect()
    }

    #[tokio::test]
    async fn a_command_runs_at_the_repositorys_root_with_its_placeholders_and_its_png_is_put_in_place() {
        let (root, design) = repo();
        let limit = Duration::from_secs(20);
        let command = r#"[ "$(pwd -P)" = "$PWD_WANT" ] && [ "$EQUIPE_SHOT_STATE/$EQUIPE_SHOT_MODE/$EQUIPE_SHOT_WIDTH" = "default/{mode}/{width}" ] && [ "$EQUIPE_SHOT_OUT" = {out} ] && cp fixture.png {out}"#;
        let run = run_of(&design, &command.replace("$PWD_WANT", &root.display().to_string()), "dark");
        let path = shoot_one(&run, &root, limit).await.unwrap();
        assert_eq!(path, design.join("flows/onboarding/login/default.dark.1280.png"));
        assert_eq!(std::fs::read(&path).unwrap(), PNG);
        assert!(leftovers(path.parent().unwrap()).is_empty(), "the hidden file it was written to is gone");
        let map = read_map(&design).unwrap();
        assert_eq!(map.flows[0].screens[0].states[0].pictures.len(), 1, "and the map finds it");

        // Run again and failing, the picture before stays as it was.
        let failed = shoot_one(&run_of(&design, "echo 'no window' >&2; exit 3", "dark"), &root, limit).await.unwrap_err();
        assert!(failed.starts_with("the command stopped (exit status: 3)\n    | no window"), "{failed}");
        assert_eq!(std::fs::read(&path).unwrap(), PNG);
    }

    #[tokio::test]
    async fn a_command_that_writes_no_png_says_so_with_the_end_of_what_it_printed() {
        let (root, design) = repo();
        let limit = Duration::from_secs(20);
        let chatty = (1..=30).map(|n| format!("echo line {n} >&2")).collect::<Vec<_>>().join("; ");
        let none = shoot_one(&run_of(&design, &chatty, "light"), &root, limit).await.unwrap_err();
        let lines: Vec<&str> = none.lines().collect();
        assert!(lines[0].starts_with("the command wrote no picture at ") && lines[0].contains("/flows/onboarding/login/"), "{none}");
        assert_eq!(lines[1..], (19..=30).map(|n| format!("    | line {n}")).collect::<Vec<_>>(), "its last lines, no more");
        let fake = shoot_one(&run_of(&design, "echo 'not a picture' > {out}", "light"), &root, limit).await.unwrap_err();
        assert!(fake.starts_with("what the command wrote at ") && fake.ends_with(" isn't a PNG"), "{fake}");
        let quiet = shoot_one(&run_of(&design, "true", "light"), &root, limit).await.unwrap_err();
        assert!(!quiet.contains('\n'), "nothing printed, nothing told: {quiet}");
        let dir = design.join("flows/onboarding/login");
        assert!(leftovers(&dir).is_empty() && !dir.join("default.light.1280.png").exists(), "nothing left behind");
        assert!(!dir.join("default.light.1280.png").exists());
    }

    #[tokio::test]
    async fn a_command_past_its_limit_is_stopped_with_what_it_started() {
        let (root, design) = repo();
        let marker = root.join("still-running");
        let command = format!("echo starting >&2; (sleep 3; touch {}) & sleep 30", marker.display());
        let began = std::time::Instant::now();
        let late = shoot_one(&run_of(&design, &command, "light"), &root, Duration::from_millis(400)).await.unwrap_err();
        assert!(began.elapsed() < Duration::from_secs(3), "stopped at its limit, not when it ended: {:?}", began.elapsed());
        assert_eq!(late, "the command was still running after 0.4 s, so it was stopped\n    | starting");
        tokio::time::sleep(Duration::from_millis(3500)).await;
        assert!(!marker.exists(), "what it started in the background was stopped too");
        let ran = run_command("sleep 0.1", &root, &[], Duration::from_secs(5)).await.unwrap();
        assert!(matches!(ran.ended, Ended::Exited(status) if status.success()), "{ran:?}");
    }

    #[tokio::test]
    async fn commands_run_some_at_once_and_how_many_failed_is_counted() {
        let (root, design) = repo();
        let runs = [run_of(&design, "sleep 0.5; cp fixture.png {out}", "light"), run_of(&design, "sleep 0.5; cp fixture.png {out}", "dark"), run_of(&design, "sleep 0.5; exit 1", "light")];
        let mut runs = runs.into_iter().collect::<Vec<_>>();
        runs[2].path = runs[2].path.with_file_name("other.light.1280.png");
        let began = std::time::Instant::now();
        assert_eq!(shoot_by_commands(&design, &runs, 3, Duration::from_secs(20)).await, Ok(1));
        assert!(began.elapsed() < Duration::from_millis(1400), "three at once: {:?}", began.elapsed());
        assert!(design.join("flows/onboarding/login/default.dark.1280.png").is_file());
        let _ = root;
        let away = std::env::temp_dir().join(format!("equipe-cli-norepo-{}", std::process::id())).join("design");
        std::fs::create_dir_all(&away).unwrap();
        if equipe_core::worktree::repo_root(&away).is_err() {
            let said = shoot_by_commands(&away, &runs, 1, Duration::from_secs(1)).await.unwrap_err();
            assert!(said.starts_with("The map's commands run only at the root of the repository") && said.contains("equipe-cli-norepo"), "{said}");
        }
    }

    #[tokio::test]
    async fn a_record_command_writes_the_video_and_maybe_its_marks() {
        let (root, _) = repo();
        let work = root.join("work");
        let write_mp4 = r#"printf '\000\000\000\030ftypisom0000' > {out}; printf '{"millis": 4200, "marks": [{"screen": "board", "at": 0}]}' > "${EQUIPE_RECORD_OUT%.mp4}.json""#;
        let made = record_by_command(&work, write_mp4, &root, "desktop", 1280, "dark", Duration::from_secs(20)).await.unwrap();
        assert_eq!(made.video, work.join("walkthrough.mp4"));
        assert_eq!(made.sidecar, Sidecar { millis: 4200, marks: vec![Mark { screen: "board".into(), at: 0 }] });
        let bare = record_by_command(&work, r#"printf '\000\000\000\030ftypisom' > {out}"#, &root, "desktop", 1280, "dark", Duration::from_secs(20)).await.unwrap();
        assert_eq!(bare.sidecar, Sidecar { millis: 0, marks: vec![] }, "no note beside it, none kept");
        let not = record_by_command(&work, "echo hi > {out}", &root, "desktop", 1280, "dark", Duration::from_secs(20)).await.unwrap_err();
        assert!(not.ends_with("isn't an MP4"), "{not}");
        let none = record_by_command(&work, "echo 'no camera' >&2", &root, "desktop", 1280, "dark", Duration::from_secs(20)).await.unwrap_err();
        assert!(none.starts_with("the command wrote no video at ") && none.ends_with("    | no camera"), "{none}");
    }
}
