//! `equipe design system`, `matrix` and `registry`: a project's design
//! system as code (`equipe_core::design::system`) — what it has and lacks,
//! each component drawn in every variant, state and theme with its
//! accessibility and keyboard checked (`assets/design/matrix.mjs`), and its
//! registry checked, built and installed into a scratch project. And
//! `system_tools`, the board tool that runs these for a designer or a
//! frontend engineer outside its engine's sandbox, where Chrome and the
//! network are.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use clap::Args;
use equipe_core::design::system::{self, catalog, matrix, registry};
use equipe_core::t;
use serde_json::json;

#[derive(Args)]
pub struct SystemArgs {
    /// The project (default: here)
    pub path: Option<PathBuf>,
    /// What was found as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct MatrixArgs {
    /// The components, by name (`button`, `alert-dialog`)
    pub components: Vec<String>,
    /// Every component the project has
    #[arg(long)]
    pub all: bool,
    /// Where its docs page is drawn; `{name}` is each component's name (http://localhost:3000/docs/components/{name})
    #[arg(long)]
    pub url: Option<String>,
    /// Widths in px, comma-separated
    #[arg(long, default_value = "390,1280")]
    pub widths: String,
    /// Themes, comma-separated
    #[arg(long, default_value = "light,dark")]
    pub themes: String,
    /// The project (default: here)
    #[arg(long)]
    pub project: Option<PathBuf>,
    /// The design folder its records are kept in (default: the project's design/)
    #[arg(long)]
    pub design: Option<PathBuf>,
    /// The report as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct RegistryArgs {
    /// The project whose registry.json it is (default: here)
    pub path: Option<PathBuf>,
    /// Run `shadcn build` first (the project's own shadcn, else `npx shadcn@3`)
    #[arg(long)]
    pub build: bool,
    /// The items to install into a scratch project (default: the first component)
    #[arg(long = "try")]
    pub try_items: Vec<String>,
    /// Also install them with the real `npx shadcn add` (needs the network)
    #[arg(long)]
    pub npx: bool,
    /// The problems as JSON
    #[arg(long)]
    pub json: bool,
}

fn here(path: Option<PathBuf>) -> PathBuf {
    path.unwrap_or_else(|| PathBuf::from("."))
}

pub fn system_command(args: SystemArgs) -> Result<(), String> {
    let root = here(args.path);
    let root = system::project_of(&root.join("x")).unwrap_or(root);
    let found = system::detect(&root);
    if args.json {
        let out = json!({ "found": found, "coverage": system::coverage(&found), "missing": system::missing(&found), "contrast": system::token_contrast(&found.all_tokens()) });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return Ok(());
    }
    if !found.has_system() && found.tokens.is_empty() {
        println!("{}", t!("design.system.none", path = found.root.display()));
    }
    print!("{}", system::report(&found));
    Ok(())
}

fn list(text: &str) -> Vec<String> {
    text.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()
}

fn widths(text: &str) -> Result<Vec<u32>, String> {
    text.split(',').map(|w| w.trim().parse::<u32>().map_err(|_| t!("design.flows.bad_widths", widths = text))).collect()
}

/// A component's source: its file in the components' folder, else the registry item's.
fn source_of(found: &system::Detected, name: &str) -> Option<String> {
    let dir = found.components_dir.as_ref();
    let by_dir = dir.and_then(|d| ["tsx", "jsx", "vue", "svelte", "ts"].iter().map(|e| d.join(format!("{name}.{e}"))).find(|p| p.is_file()));
    let by_item = found.registry.as_ref().and_then(|r| r.item(name)).and_then(|i| i.files.first()).map(|f| found.root.join(&f.path)).filter(|p| p.is_file());
    by_dir.or(by_item).and_then(|p| std::fs::read_to_string(p).ok())
}

/// Where axe-core's browser build is in the design kit, set up once.
async fn axe() -> Result<PathBuf, String> {
    let dir = crate::design::kit_dir();
    let file = dir.join("node_modules").join("axe-core").join("axe.min.js");
    if file.is_file() {
        return Ok(file);
    }
    let (_, npm) = crate::design::node_tools()?;
    let status = tokio::process::Command::new(&npm)
        .args(["install", "--prefix"])
        .arg(&dir)
        .args(["--no-audit", "--no-fund", "--loglevel=error", AXE])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .await
        .map_err(|e| format!("npm: {e}"))?;
    if status.success() && file.is_file() { Ok(file) } else { Err(format!("npm install {AXE} stopped ({status})")) }
}

const AXE: &str = "axe-core@4.10.3";
const SHADCN: &str = "shadcn@3";

pub async fn matrix_command(args: MatrixArgs) -> Result<(), String> {
    let root = here(args.project);
    let root = system::project_of(&root.join("x")).unwrap_or(root);
    let found = system::detect(&root);
    let names: Vec<String> = if args.all { found.components.clone() } else { args.components.clone() };
    if names.is_empty() {
        return Err(t!("design.matrix.which").to_owned());
    }
    let design = args.design.unwrap_or_else(|| found.root.join(equipe_core::design::DIR));
    let widths = widths(&args.widths)?;
    let themes = list(&args.themes);
    let axe = match axe().await {
        Ok(p) => Some(p),
        Err(why) => {
            eprintln!("{}", t!("design.matrix.no_axe", why = why));
            None
        }
    };
    let mut failed = 0;
    let mut reports = Vec::new();
    for name in &names {
        let Some(comp) = catalog::catalog().get(name) else {
            eprintln!("{}", t!("design.matrix.unknown", name = name));
            failed += 1;
            continue;
        };
        let url = match (&args.url, found.doc(&comp.name)) {
            (Some(u), _) => u.replace("{name}", &comp.name),
            (None, Some(doc)) if doc.path.extension().is_some_and(|e| e == "html" || e == "htm") => doc.path.to_string_lossy().into_owned(),
            _ => {
                eprintln!("{}", t!("design.matrix.no_url", name = comp.name));
                failed += 1;
                continue;
            }
        };
        let plan = matrix::plan(comp, source_of(&found, &comp.name).as_deref(), &url, &widths, &themes);
        let pictures = matrix::pictures_dir(&design, &comp.name);
        std::fs::create_dir_all(&pictures).map_err(|e| format!("{}: {e}", pictures.display()))?;
        let plan_file = pictures.join(format!(".plan-{}.json", std::process::id()));
        let out_file = pictures.join(format!(".measured-{}.json", std::process::id()));
        std::fs::write(&plan_file, serde_json::to_string(&plan).unwrap_or_default()).map_err(|e| e.to_string())?;
        eprintln!("{}", t!("design.matrix.drawing", name = comp.name, count = matrix::per_example(&plan)));
        let mut words: Vec<String> = vec!["--plan".into(), plan_file.to_string_lossy().into_owned(), "--out".into(), pictures.to_string_lossy().into_owned(), "--json".into(), out_file.to_string_lossy().into_owned()];
        if let Some(axe) = &axe {
            words.extend(["--axe".into(), axe.to_string_lossy().into_owned()]);
        }
        let ran = crate::design::run_script("matrix.mjs", &words.iter().map(String::as_str).collect::<Vec<_>>()).await;
        let measured = std::fs::read_to_string(&out_file);
        let _ = std::fs::remove_file(&plan_file);
        let _ = std::fs::remove_file(&out_file);
        ran?;
        let measured: matrix::Measured = serde_json::from_str(&measured.map_err(|e| format!("matrix.mjs wrote nothing: {e}"))?).map_err(|e| format!("matrix.mjs: {e}"))?;
        let report = matrix::judge(&plan, &measured, equipe_core::design::lint::record::now());
        let kept = matrix::write(&design, &report).map_err(|e| format!("{}: {e}", design.display()))?;
        if !report.passes() {
            failed += 1;
        }
        if !args.json {
            print!("{}", report.text());
            println!("{}", t!("design.matrix.kept", path = kept.display()));
            println!("{}", report.mark_line());
        }
        reports.push(report);
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&reports).unwrap_or_default());
    }
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// The project's own shadcn, else the CLI from npm (`npx`), with `words`.
async fn shadcn(root: &Path, words: &[&str], limit: Duration) -> Result<(), String> {
    let local = root.join("node_modules").join(".bin").join("shadcn");
    let mut command = if local.is_file() {
        tokio::process::Command::new(local)
    } else {
        let npx = equipe_core::browser::find_npx().ok_or_else(|| t!("design.registry.no_node").to_owned())?;
        let mut c = tokio::process::Command::new(npx);
        c.args(["--yes", SHADCN]);
        c
    };
    command.args(words).current_dir(root).stdin(Stdio::null()).kill_on_drop(true);
    let run = tokio::time::timeout(limit, command.output()).await.map_err(|_| t!("design.registry.timed_out", secs = limit.as_secs()))?.map_err(|e| e.to_string())?;
    if run.status.success() {
        return Ok(());
    }
    let said = [String::from_utf8_lossy(&run.stderr), String::from_utf8_lossy(&run.stdout)].join("\n");
    let tail: Vec<&str> = said.lines().filter(|l| !l.trim().is_empty()).rev().take(6).collect();
    Err(t!("design.registry.shadcn_failed", words = words.join(" "), status = run.status, tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n")))
}

/// A scratch project for a sample install: its components.json taking the
/// real project's aliases, a package.json and a tsconfig with `@/`.
fn scratch(aliases: &[(String, String)], tag: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("equipe-registry-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("app")).map_err(|e| e.to_string())?;
    let alias = |k: &str, d: &str| aliases.iter().find(|(a, _)| a == k).map_or_else(|| d.to_owned(), |(_, v)| v.clone());
    let components = json!({
        "$schema": "https://ui.shadcn.com/schema.json",
        "style": "new-york", "rsc": false, "tsx": true,
        "tailwind": { "config": "", "css": "app/globals.css", "baseColor": "neutral", "cssVariables": true },
        "aliases": { "components": alias("components", "@/components"), "ui": alias("ui", "@/components/ui"), "lib": alias("lib", "@/lib"), "utils": alias("utils", "@/lib/utils"), "hooks": alias("hooks", "@/hooks") },
        "iconLibrary": "lucide"
    });
    let files = [
        ("components.json", serde_json::to_string_pretty(&components).unwrap_or_default()),
        ("package.json", r#"{ "name": "equipe-registry-try", "private": true, "dependencies": {} }"#.to_owned()),
        ("tsconfig.json", r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "@/*": ["./*"] }, "jsx": "preserve" } }"#.to_owned()),
        ("app/globals.css", "@import \"tailwindcss\";\n".to_owned()),
    ];
    for (name, body) in files {
        std::fs::write(dir.join(name), body).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

pub async fn registry_command(args: RegistryArgs) -> Result<(), String> {
    let root = here(args.path);
    let root = system::project_of(&root.join("x")).unwrap_or(root);
    let file = root.join("registry.json");
    if !file.is_file() {
        return Err(t!("design.registry.none", path = file.display()));
    }
    if args.build {
        eprintln!("{}", t!("design.registry.building"));
        shadcn(&root, &["build"], Duration::from_secs(600)).await?;
    }
    let found = system::detect(&root);
    let reg = registry::Registry::read(&file)?;
    let mut problems = registry::check(&reg, found.built.as_deref());
    let aliases = found.components_json.as_ref().map(|c| c.aliases.clone()).unwrap_or_default();
    let tries: Vec<String> = if args.try_items.is_empty() {
        reg.items.iter().find(|i| i.kind == "registry:ui").map(|i| vec![i.name.clone()]).unwrap_or_default()
    } else {
        args.try_items.clone()
    };
    let mut lines = Vec::new();
    let mut broken = false;
    match &found.built {
        None if !tries.is_empty() => lines.push(t!("design.registry.not_built").to_owned()),
        None => {}
        Some(built) => {
            for item in &tries {
                let project = scratch(&aliases, "offline")?;
                let done = registry::install(&reg, built, item, &project, &aliases);
                if done.resolves() {
                    lines.push(t!("design.registry.try_ok", item = item, files = done.files.len(), packages = if done.packages.is_empty() { "—".to_owned() } else { done.packages.join(", ") }));
                } else {
                    broken = true;
                    lines.push(t!("design.registry.try_failed", item = item));
                    lines.extend(done.unresolved.iter().chain(&done.broken).map(|l| format!("  ✗ {l}")));
                }
                let _ = std::fs::remove_dir_all(&project);
                if args.npx {
                    let project = scratch(&aliases, "npx")?;
                    let item_file = std::fs::canonicalize(built.join(format!("{item}.json"))).unwrap_or_else(|_| built.join(format!("{item}.json")));
                    match shadcn(&project, &["add", &item_file.to_string_lossy(), "--yes", "--overwrite", "--silent"], Duration::from_secs(600)).await {
                        Ok(()) => lines.push(t!("design.registry.npx_ok", item = item)),
                        Err(why) => {
                            broken = true;
                            lines.push(why);
                        }
                    }
                    let _ = std::fs::remove_dir_all(&project);
                }
            }
        }
    }
    problems.sort_by_key(|p| p.level);
    let errors = problems.iter().filter(|p| p.level == registry::Level::Error).count();
    if args.json {
        println!("{}", serde_json::to_string_pretty(&json!({ "problems": problems, "install": lines })).unwrap_or_default());
    } else {
        for p in &problems {
            println!("{}", p.line());
        }
        for l in &lines {
            println!("{l}");
        }
        println!("{}", t!("design.registry.summary", errors = errors, warnings = problems.len() - errors, items = reg.items.len()));
    }
    if errors > 0 || broken {
        std::process::exit(1);
    }
    Ok(())
}

/// The board tool's name, its commands, and the folder question it asks the daemon.
pub(crate) const COMMANDS: [&str; 3] = equipe_core::board::SYSTEM_TOOL_COMMANDS;

/// The words a board call may run: `equipe design` and one of the system
/// commands, never a folder of its own choosing (it works in the member's
/// worktree).
pub(crate) fn board_args(args: &[String]) -> Result<Vec<String>, String> {
    let first = args.first().map(String::as_str).unwrap_or_default();
    if !COMMANDS.contains(&first) {
        return Err(t!("design.tools.which", commands = COMMANDS.join(", ")));
    }
    if let Some(flag) = args.iter().find(|a| ["--project", "--design", "--out"].contains(&a.split('=').next().unwrap_or_default())) {
        return Err(t!("design.tools.own_folder", flag = flag));
    }
    // A path given as the project, outside the worktree, is refused the same way.
    if let Some(path) = args.iter().skip(1).find(|a| !a.starts_with('-') && (a.starts_with('/') || a.starts_with("..") || a.starts_with('~'))) {
        return Err(t!("design.tools.own_folder", flag = path));
    }
    Ok(std::iter::once("design".to_owned()).chain(args.iter().cloned()).collect())
}

/// `system_tools`, for a designer's or a frontend engineer's board server:
/// `equipe design <args>` run as this same equipe, in the member's worktree
/// (asked of the daemon), outside its engine's sandbox; what it printed is
/// the answer.
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
        .env("EQUIPE_SNAPSHOT_HIDDEN", "1")
        .env_remove("CODEX_SANDBOX")
        .env_remove("CODEX_SANDBOX_NETWORK_DISABLED")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = match tokio::time::timeout(Duration::from_secs(20 * 60), run).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return ToolResult { text: format!("equipe design: {e}"), is_error: true, image: None },
        Err(_) => return ToolResult { text: t!("design.tools.too_long").to_owned(), is_error: true, image: None },
    };
    let said = [String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)].join("\n");
    let said = plain(said.trim());
    // The end is what matters: the findings, what was kept, what is next.
    let text = match said.char_indices().rev().nth(12000) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_board_tool_runs_only_the_system_commands_in_the_members_own_folder() {
        let words = |w: &[&str]| w.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(board_args(&words(&["matrix", "button", "--url", "http://localhost:3000/docs/{name}"])).unwrap()[..3], words(&["design", "matrix", "button"]));
        assert!(board_args(&words(&["system"])).is_ok() && board_args(&words(&["registry", "--build", "--try", "dialog"])).is_ok());
        for refused in [&["lint", "x.tsx"][..], &[], &["matrix", "button", "--design", "/etc"], &["registry", "/Users/x/other"], &["system", "../elsewhere"], &["matrix", "--project=/tmp/x"]] {
            assert!(board_args(&words(refused)).is_err(), "{refused:?}");
        }
        assert_eq!(plain("\u{1b}[1m✗\u{1b}[0m no"), "✗ no");
    }

    #[test]
    fn a_scratch_project_takes_the_real_ones_aliases() {
        let dir = scratch(&[("ui".into(), "@/ui".into())], "test").unwrap();
        let c: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("components.json")).unwrap()).unwrap();
        assert_eq!(c["aliases"]["ui"], "@/ui");
        assert_eq!(c["aliases"]["lib"], "@/lib");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
