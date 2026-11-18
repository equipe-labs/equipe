//! `equipe design session …` and `equipe design tuner …`: the designer's side
//! of a design session (`equipe_core::design::session`) and of the taste
//! tuner (`equipe_core::taste::tuner`). What a session command did is
//! printed as a `design-session:` line the person's screen reads from the
//! designer's conversation — the page live beside it, a pin ticked — so it
//! needs no connection to the daemon, and works the same on any engine.

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Subcommand;
use equipe_core::design::page_check::{self, Scope, Seen};
use equipe_core::design::session::{self, Marker};
use equipe_core::taste::{profile, tuner};

#[derive(Subcommand)]
pub enum SessionAction {
    /// Show a page live beside the session: an HTML file, a dev server's address, or a rendered picture
    Show {
        /// file.html, http://localhost:5173/path, or render.png
        target: String,
        /// One of the prototype's variants (A, B or C): the person's screen puts the variants side by side
        #[arg(long)]
        variant: Option<String>,
        /// Send it without opening and photographing it first
        #[arg(long)]
        no_check: bool,
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

/// A page shown as one of the prototype's variants: a live page only, by
/// its letter.
fn with_variant(marker: Marker, variant: Option<&str>) -> Result<Marker, String> {
    let Some(raw) = variant else { return Ok(marker) };
    let label = session::variant_label(raw).ok_or_else(|| format!("A variant is one letter, A to {}: `--variant B`.", (b'A' + session::VARIANTS_MAX as u8 - 1) as char))?;
    match marker {
        Marker::Show(url) => Ok(Marker::Variant { label, url }),
        _ => Err("Only a live page (an HTML file or a dev server's address) can be a variant; show a render on its own.".into()),
    }
}

/// What `show` says it did. Sent, not seen: equipe reads the line below and
/// brings the page to the person's screen — a saved file served by the
/// daemon, so it opens on another Mac too — and tells the member at once
/// when no screen of theirs can show it (docs/REMOTE.md).
fn shown_line(marker: &Marker) -> String {
    let served = |url: &str| if url.starts_with("file://") { " (equipe serves the file to their screen, on this Mac or another — don't hand them this address)" } else { "" };
    let after = "It reloads each time you show it; if no screen of the person's can show it, equipe tells you right away.";
    match marker {
        Marker::Show(url) => format!("Sent beside the session: {url}{}. {after}", served(url)),
        Marker::Variant { label, url } => format!("Variant {label} sent beside the session, next to the others: {url}{}. {after}", served(url)),
        Marker::Render(path) => format!("Sent beside the session: {path}. If no screen of the person's can show it, equipe tells you right away."),
        Marker::Done { .. } => unreachable!("a target is never a pin"),
    }
}

/// How long the look at a page may take, Chrome's start included.
const LOOK_WAIT: Duration = Duration::from_secs(90);

/// A page looked at before it is sent: a saved page's files checked where
/// the page server serves them from (`page_check::missing_assets`), then the
/// page opened at the address the person's screen gets — the same page
/// server, here in this command — and photographed (`pagecheck.mjs`). `Err`
/// says what is broken, with the picture; `Ok` the picture, or `None` when
/// it couldn't be looked at (no Node or Chrome here): it goes unchecked.
async fn look_before_sending(url: &str) -> Result<Option<PathBuf>, String> {
    let (address, page) = match session::file_path(url) {
        Some(page) => {
            let scope = Scope::of_page(&page);
            let missing = page_check::missing_assets(&page, &scope);
            if !missing.is_empty() {
                return Err(page_check::not_sent(&page, &missing.iter().map(page_check::Missing::words).collect::<Vec<_>>(), None));
            }
            let Some(server) = equipe_core::prototype::Server::now() else { return Ok(None) };
            (server.put_session(&page, &scope.served)?, Some(page))
        }
        None => (url.to_owned(), None),
    };
    let picture = picture_path(page.as_deref().unwrap_or(Path::new("page")));
    if let Some(dir) = picture.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let output = match crate::design::run_script_output("pagecheck.mjs", &[&address, "--out", &picture.to_string_lossy()], LOOK_WAIT).await {
        Ok(output) => output,
        Err(why) => {
            eprintln!("Couldn't look at the page before sending it ({why}); sent unchecked.");
            return Ok(None);
        }
    };
    let Some(seen) = Seen::read(&output) else { return Ok(None) };
    let problems = seen.problems();
    let shot = picture.is_file().then_some(picture.as_path());
    if !problems.is_empty() {
        return Err(page_check::not_sent(page.as_deref().unwrap_or(Path::new(url)), &problems, shot));
    }
    Ok(shot.map(Path::to_path_buf))
}

/// Where the picture of a page looked at goes: out of every repository, one per page.
fn picture_path(page: &Path) -> PathBuf {
    let stem = page.file_stem().map_or_else(|| "page".into(), |s| s.to_string_lossy().into_owned());
    let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    std::env::temp_dir().join("equipe-shown").join(format!("{stem}-{millis}.png"))
}

fn design_dir(dir: Option<PathBuf>) -> PathBuf {
    dir.unwrap_or_else(|| PathBuf::from("design"))
}

pub async fn session_command(action: SessionAction) -> Result<(), String> {
    match action {
        SessionAction::Show { target, variant, no_check } => {
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            let marker = with_variant(session::target(&target, &cwd)?, variant.as_deref())?;
            let url = match &marker {
                Marker::Show(url) | Marker::Variant { url, .. } => Some(url.clone()),
                _ => None,
            };
            // Looked at first, as the person's screen will open it: a broken page isn't sent.
            let picture = match url {
                Some(url) if !no_check => look_before_sending(&url).await?,
                _ => None,
            };
            println!("{}", shown_line(&marker));
            if let Some(picture) = picture {
                println!("What the person sees: {} (open it to look).", picture.display());
            }
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
            // In the project's main checkout it is committed on main at once: never left to block a merge.
            crate::design::keep_on_main(&dir, &[path], "equipe: the style profile, from a design session");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_page_naming_files_the_screen_wont_get_is_not_sent_and_says_which() {
        let root = std::env::temp_dir().join(format!("equipe-show-check-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let session = root.join("design/sessions/pricing");
        std::fs::create_dir_all(&session).unwrap();
        let page = std::fs::canonicalize(&session).unwrap().join("index-a.html");
        std::fs::write(&page, r#"<link rel="stylesheet" href="../../src/tokens.css"><img src="/assets/logo/mark.svg">"#).unwrap();
        let url = format!("file://{}", page.display());
        let said = look_before_sending(&url).await.unwrap_err();
        assert!(said.starts_with("Not sent:"), "{said}");
        assert!(said.contains("`../../src/tokens.css`") && said.contains("`/assets/logo/mark.svg`"), "{said}");
        assert!(said.contains("show it again"), "{said}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_pictures_place_is_out_of_the_repository_and_named_after_its_page() {
        let at = picture_path(Path::new("/w/design/sessions/pricing/index-a.html"));
        assert!(at.starts_with(std::env::temp_dir()) && at.file_name().unwrap().to_string_lossy().starts_with("index-a-"), "{}", at.display());
    }
}
