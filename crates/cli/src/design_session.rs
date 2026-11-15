//! `equipe design session …` and `equipe design tuner …`: the designer's side
//! of a design session (`equipe_core::design::session`) and of the taste
//! tuner (`equipe_core::taste::tuner`). What a session command did is
//! printed as a `design-session:` line the person's screen reads from the
//! designer's conversation — the page live beside it, a pin ticked — so it
//! needs no connection to the daemon, and works the same on any engine.

use std::path::PathBuf;

use clap::Subcommand;
use equipe_core::design::session::{self, Marker};
use equipe_core::taste::{profile, tuner};

#[derive(Subcommand)]
pub enum SessionAction {
    /// Show a page live beside the session: an HTML file, a dev server's address, or a rendered picture
    Show {
        /// file.html, http://localhost:5173/path, or render.png
        target: String,
    },
    /// Mark a pin the person dropped as addressed, with what changed in one line
    Done {
        /// The pin's number
        pin: u32,
        /// What changed, in one line
        what: Vec<String>,
    },
    /// Write the style profile at the top of design/taste.md: only the fields given change; its leans are equipe's
    Profile {
        /// Airy or dense, and how
        #[arg(long)]
        density: Option<String>,
        /// The tone: calm, playful, editorial…
        #[arg(long)]
        tone: Option<String>,
        /// The type: faces, scale, setting
        #[arg(long = "type")]
        typography: Option<String>,
        /// How colour is used
        #[arg(long)]
        color: Option<String>,
        /// References and takes the person liked, `;`-separated
        #[arg(long)]
        liked: Option<String>,
        /// References and takes the person disliked, `;`-separated
        #[arg(long)]
        disliked: Option<String>,
        /// The design folder (default: ./design)
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum TunerAction {
    /// Check design/tuner/set.json: enough pictures, every axis varied, pairs that vary one or two axes
    Check {
        /// The design folder (default: ./design)
        dir: Option<PathBuf>,
    },
}

fn design_dir(dir: Option<PathBuf>) -> PathBuf {
    dir.unwrap_or_else(|| PathBuf::from("design"))
}

pub fn session_command(action: SessionAction) -> Result<(), String> {
    match action {
        SessionAction::Show { target } => {
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            let marker = session::target(&target, &cwd)?;
            let shown = match &marker {
                Marker::Show(url) => format!("Live beside the session: {url}. It reloads each time you show it."),
                Marker::Render(path) => format!("Shown beside the session: {path}."),
                Marker::Done { .. } => unreachable!("a target is never a pin"),
            };
            println!("{shown}");
            println!("{}", marker.line());
            Ok(())
        }
        SessionAction::Done { pin, what } => {
            let what = what.join(" ");
            let what = what.split_whitespace().collect::<Vec<_>>().join(" ");
            if what.is_empty() {
                return Err("Say what changed, in one line: `equipe design session done 2 \"Quieter accent on the CTA\"`.".into());
            }
            println!("Pin {pin} marked done.");
            println!("{}", Marker::Done { n: pin, what }.line());
            Ok(())
        }
        SessionAction::Profile { density, tone, typography, color, liked, disliked, dir } => {
            let dir = design_dir(dir);
            let path = dir.join(equipe_core::taste::store::NOTES_FILE);
            let before = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => equipe_core::taste::store::TEMPLATE.to_owned(),
                Err(e) => return Err(format!("{}: {e}", path.display())),
            };
            let mut p = profile::parse(&before).unwrap_or_default();
            let fields = [(profile::Field::Density, density), (profile::Field::Tone, tone), (profile::Field::Type, typography), (profile::Field::Color, color), (profile::Field::Liked, liked), (profile::Field::Disliked, disliked)];
            if fields.iter().all(|(_, v)| v.is_none()) {
                return Err("Give the fields this round changed: --density, --tone, --type, --color, --liked, --disliked.".into());
            }
            for (field, value) in fields {
                if let Some(value) = value {
                    profile::set(&mut p, field, &value);
                }
            }
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            p.updated = Some(equipe_core::squad_memory::date(secs));
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            std::fs::write(&path, profile::write(&before, &p)).map_err(|e| format!("{}: {e}", path.display()))?;
            println!("Style profile written at the top of {}.", path.display());
            Ok(())
        }
    }
}

pub fn tuner_command(action: TunerAction) -> Result<(), String> {
    match action {
        TunerAction::Check { dir } => {
            let dir = design_dir(dir);
            let file = dir.join(tuner::SET_FILE);
            let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let items = tuner::parse_set(&text, &dir)?;
            let problems = tuner::check(&items);
            if problems.is_empty() {
                println!("{} pictures: the set is ready.", items.len());
                return Ok(());
            }
            for p in &problems {
                println!("- {p}");
            }
            Err(format!("{} problem(s) in {}", problems.len(), file.display()))
        }
    }
}
