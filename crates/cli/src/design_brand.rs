//! `equipe design brand …` and `equipe design emit`: machine-level brands a
//! project inherits its design system from (`equipe_core::design::inherit`,
//! docs/BRANDS.md) — made, released, linked to projects and squads, shown
//! with what a project inherits and what it overrides — and a design
//! folder's tokens as equipe reads them, resolved.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use equipe_core::design::inherit;
use equipe_core::journal::{Journal, squad_dir};

#[derive(Subcommand)]
pub enum BrandAction {
    /// Make a brand on this Mac (~/.equipe/brands/<slug>/): brand.json, design/brand.md, design/tokens, components, taste.md, refs.md — version 1
    New {
        /// Its name on disk: lower-case letters, digits and dashes
        slug: String,
        /// What it is called (default: the slug)
        #[arg(long)]
        name: Option<String>,
        /// Start from a design folder's tokens (or one tokens file) instead of the starter set
        #[arg(long)]
        from: Option<PathBuf>,
        /// Also list it for this squad (default: none)
        #[arg(long)]
        squad: Option<String>,
    },
    /// Release the brand as it is now as a new version; projects pinned to an earlier one keep it
    Release {
        slug: String,
        /// The version (default: the next whole number)
        #[arg(long)]
        version: Option<String>,
    },
    /// Link a project's design folder to a brand (writes design/brand.json), and list the brand for its squad
    Link {
        slug: String,
        /// The project's design folder (default: ./design)
        dir: Option<PathBuf>,
        /// The version to stay on (default: the brand's current one)
        #[arg(long, conflicts_with = "latest")]
        pin: Option<String>,
        /// Follow the brand as it is, unpinned
        #[arg(long)]
        latest: bool,
        /// The squad that lists it (default: the one this folder is in)
        #[arg(long)]
        squad: Option<String>,
        /// Only list the brand for the squad; link no project
        #[arg(long)]
        squad_only: bool,
    },
    /// Unlink a project (its design/brand.json goes; its overrides stay), or with --squad and --brand, take a brand off a squad's list
    Unlink {
        /// The project's design folder (default: ./design)
        dir: Option<PathBuf>,
        #[arg(long, requires = "brand")]
        squad: Option<String>,
        #[arg(long, requires = "squad")]
        brand: Option<String>,
    },
    /// A project's brand — which tokens it inherits and which it overrides — or, given a slug, the brand: versions and the squads using it
    Show {
        slug: Option<String>,
        /// The project's design folder (default: ./design)
        #[arg(long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

fn home() -> PathBuf {
    equipe_providers::credentials::equipe_home()
}

fn design_dir(dir: Option<PathBuf>) -> PathBuf {
    dir.unwrap_or_else(|| PathBuf::from(equipe_core::design::DIR))
}

/// The squad named, else the one `near` is in.
fn squad_of(name: Option<String>, near: &Path) -> Option<String> {
    let near = std::fs::canonicalize(near).unwrap_or_else(|_| near.to_path_buf());
    crate::home::squad_for(&home(), &near, name)
}

fn journal(squad: &str) -> Result<Journal, String> {
    let db = squad_dir(&home(), squad).join("squad.db");
    if !db.is_file() {
        return Err(format!("No squad `{squad}` on this Mac."));
    }
    Journal::open(&db).map_err(|e| e.to_string())
}

fn list_for_squad(squad: &str, slug: &str, on: bool) -> Result<bool, String> {
    let j = journal(squad)?;
    let mut brands = j.brands();
    let had = brands.iter().any(|b| b == slug);
    match (on, had) {
        (true, false) => brands.push(slug.to_owned()),
        (false, true) => brands.retain(|b| b != slug),
        _ => return Ok(false),
    }
    j.set_brands(&brands).map_err(|e| e.to_string())?;
    Ok(true)
}

pub fn brand_command(action: BrandAction) -> Result<(), String> {
    let brands = inherit::root();
    match action {
        BrandAction::New { slug, name, from, squad } => {
            let brand = inherit::create(&brands, &slug, name.as_deref().unwrap_or(&slug), from.as_deref())?;
            println!("Made brand `{}` v{} at {}", brand.slug, brand.version, inherit::brand_dir(&brands, &slug).display());
            if let Some(squad) = squad {
                list_for_squad(&squad, &slug, true)?;
                println!("Listed for squad {squad}.");
            }
            println!("Next: fill in its design/brand.md and tokens on a brand ticket; `equipe design brand link {slug}` in each project that inherits it.");
            Ok(())
        }
        BrandAction::Release { slug, version } => {
            let brand = inherit::release(&brands, &slug, version.as_deref())?;
            println!("Released brand `{}` v{}. Projects pinned to an earlier version keep it until their pin moves (`equipe design brand link {slug} --pin {}`).", brand.slug, brand.version, brand.version);
            Ok(())
        }
        BrandAction::Link { slug, dir, pin, latest, squad, squad_only } => {
            let design = design_dir(dir);
            if !squad_only {
                let pin = if latest { Some(None) } else { pin.as_deref().map(Some) };
                let link = inherit::link(&brands, &design, &slug, pin)?;
                println!("{} inherits brand `{slug}` {}.", design.display(), link.pin.map_or_else(|| "as it is (unpinned)".to_owned(), |p| format!("v{p}")));
                println!("Keep only this project's overrides in {}; `equipe design brand show` says what is inherited.", design.join("tokens").display());
            } else {
                inherit::read_brand(&brands, &slug)?;
            }
            let near = design.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
            match squad_of(squad.clone(), &near) {
                Some(s) => {
                    if list_for_squad(&s, &slug, true)? {
                        println!("Listed for squad {s}.");
                    }
                }
                None if squad_only || squad.is_some() => return Err("No squad to list it for: name one with --squad.".into()),
                None => {}
            }
            Ok(())
        }
        BrandAction::Unlink { dir, squad, brand } => {
            if let (Some(squad), Some(slug)) = (squad, brand) {
                if list_for_squad(&squad, &slug, false)? {
                    println!("Took brand `{slug}` off squad {squad}'s list.");
                } else {
                    println!("Squad {squad} doesn't list brand `{slug}`.");
                }
                return Ok(());
            }
            let design = design_dir(dir);
            if inherit::unlink(&design)? {
                println!("{} no longer inherits a brand. Its design/tokens/ (the overrides) stayed: make it a whole system, or write tokens.json.", design.display());
            } else {
                println!("{} links no brand.", design.display());
            }
            Ok(())
        }
        BrandAction::Show { slug: Some(slug), json, .. } => {
            let brand = inherit::read_brand(&brands, &slug)?;
            let versions = inherit::versions(&brands, &slug);
            let squads = inherit::squads_using(&home(), &slug);
            let dir = inherit::brand_dir(&brands, &slug);
            if json {
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "brand": brand, "dir": dir, "versions": versions, "squads": squads })).unwrap_or_default());
            } else {
                println!("{} (`{}`) v{} — {}", brand.name, brand.slug, brand.version, dir.display());
                println!("versions: {}", if versions.is_empty() { "none released".to_owned() } else { versions.join(", ") });
                println!("squads:   {}", if squads.is_empty() { "none".to_owned() } else { squads.join(", ") });
            }
            Ok(())
        }
        BrandAction::Show { slug: None, dir, json } => {
            let design = design_dir(dir);
            let Some(resolved) = inherit::resolve(&brands, &design) else {
                let all = inherit::list(&brands);
                println!("{} links no brand; its own tokens.json is its system.", design.display());
                if all.is_empty() {
                    println!("No brands on this Mac yet: `equipe design brand new <slug>`.");
                } else {
                    println!("Brands on this Mac: {}", all.iter().map(|b| format!("{} (v{})", b.slug, b.version)).collect::<Vec<_>>().join(", "));
                }
                return Ok(());
            };
            let r = resolved?;
            if json {
                println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
                return Ok(());
            }
            println!("{} inherits brand {} (`{}`) v{}{} — {}", design.display(), r.brand.name, r.brand.slug, r.version, r.link.pin.as_ref().map_or(" (unpinned)", |_| " (pinned)"), r.dir.display());
            if let Some(newer) = &r.newer {
                println!("  a newer v{newer} exists: `equipe design brand link {} --pin {newer}` to move", r.brand.slug);
            }
            let section = |title: &str, names: &[String]| {
                println!("{title} ({}):", names.len());
                for n in names {
                    println!("  {n}");
                }
            };
            section("overridden here", &r.overridden);
            if !r.added.is_empty() {
                section("this project's own (not in the brand)", &r.added);
            }
            section("inherited", &r.inherited);
            Ok(())
        }
    }
}

/// `equipe design emit`: the design folder's tokens.json as equipe reads it
/// — a brand-linked project's resolved system, else the file as it is.
pub fn emit_command(dir: Option<PathBuf>, out: Option<PathBuf>) -> Result<(), String> {
    let design = design_dir(dir);
    if let Some(Err(e)) = inherit::resolve(&inherit::root(), &design) {
        return Err(e);
    }
    let text = inherit::tokens_text(&design).ok_or_else(|| format!("No {} in {}.", equipe_core::design::folder::TOKENS, design.display()))?;
    match out {
        Some(path) => {
            std::fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
            eprintln!("wrote {}", path.display());
        }
        None => print!("{text}"),
    }
    Ok(())
}
