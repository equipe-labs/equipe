//! `equipe design …`: the designer's tools (`equipe_core::design::{check,
//! kit}`, and two scripts run in the Mac's own Chrome through Playwright —
//! `assets/design/{audit,render}.mjs`). A member runs them in its worktree;
//! so can the person.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use clap::Subcommand;
use equipe_core::design::{check, kit, tokens};

#[derive(Subcommand)]
pub enum DesignAction {
    /// Lay out a design folder to fill in: brand.md, layered tokens.json, guidelines.md
    Init {
        /// The design folder (default: ./design)
        dir: Option<PathBuf>,
    },
    /// Check a design folder before handing it in: tokens equipe reads, layers, dark, contrast, Korean type, brand, logo
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
    /// Harvest a brand from its site: colours by area, type and webfonts, logo files, voice — one audit sheet
    Audit {
        url: String,
        /// How many of its pages to read (the home page and its navigation's)
        #[arg(long, default_value_t = 3)]
        pages: u8,
        /// Where the sheet and files go (default: ./design/audit)
        #[arg(long)]
        out: Option<PathBuf>,
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
        DesignAction::Audit { url, pages, out } => {
            let out = out.unwrap_or_else(|| PathBuf::from(equipe_core::design::DIR).join("audit"));
            run_script("audit.mjs", &[&url, "--pages", &pages.to_string(), "--out", out.to_string_lossy().as_ref()]).await
        }
    }
}

/// Where the scripts and the Playwright they import live: the person's own,
/// made once. Chrome is the Mac's own (`channel: 'chrome'`), so no browser
/// is downloaded.
fn kit_dir() -> PathBuf {
    equipe_providers::credentials::equipe_home().join("design-kit")
}

const PLAYWRIGHT: &str = "playwright@1.64.0";

/// Node and npm beside the `npx` the squad's browser uses.
fn node_tools() -> Result<(PathBuf, PathBuf), String> {
    let npx = equipe_core::browser::find_npx().ok_or("Node.js isn't installed (no npx on PATH): `brew install node`")?;
    let bin = npx.parent().map(Path::to_path_buf).unwrap_or_default();
    Ok((bin.join("node"), bin.join("npm")))
}

async fn ensure_kit() -> Result<PathBuf, String> {
    let dir = kit_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
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
    for (name, body) in [("render.mjs", include_str!("../assets/design/render.mjs")), ("audit.mjs", include_str!("../assets/design/audit.mjs"))] {
        std::fs::write(dir.join(name), body).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

async fn run_script(name: &str, args: &[&str]) -> Result<(), String> {
    let dir = ensure_kit().await?;
    let (node, _) = node_tools()?;
    let status = tokio::process::Command::new(&node).arg(dir.join(name)).args(args).stdin(Stdio::null()).status().await.map_err(|e| format!("node: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{name} stopped ({status})")) }
}
