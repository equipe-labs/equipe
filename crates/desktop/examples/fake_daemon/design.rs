//! `FAKE_DESIGN`: the squad's design system, as a designer keeps it in
//! `design/` (`docs/DESIGN-SYSTEM.md`) — modelled on T-144's brand guideline
//! (equipe's own: warm neutrals, one signal blue, Inter and a face for each script), its
//! folders written under `$EQUIPE_HOME/design-system` and read the
//! daemon's way (`equipe_core::design`), pictures and all.
//!
//! - `1`: the squad's, and the web project's own (the site's older palette),
//!   with its UI map (`fake_daemon/flows.rs`).
//! - `draft`: the squad's as the desktop has it now (steel blue), and T-144
//!   in review with its draft (signal blue) — the "In review" pill.
//! - `working`: the squad's, its setup at the UI map, and T-378 still at work
//!   on it in its worktree — two flows partly shot, the designer mid-turn
//!   (`fake_daemon/flows.rs`, `seed_working`): the "In the works" chip and the
//!   empty map that says who is making it.
//! - `empty`: none yet.
//!
//! With `1`, the person's taste too: four notes of the squad's (from each
//! source), one of the web project's, and five decisions with their
//! pictures; a note added, rewritten or removed on the screen is kept for
//! the run (the squad's) or in the web project's `taste.md`.
//!
//! `FAKE_DESIGN_CHANGE_AFTER=<ms>`: the designer saves new tokens that long
//! after the first ask — an accent moved, a colour added — for "Changed since you last looked".
//! The folders are watched as the daemon watches them: edit them by hand and
//! the screen follows.

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::design::folder::{self, Place};
use equipe_core::i18n::Lang;
use equipe_core::protocol::design::{DesignPart, DesignVersion, DraftOf, Taste, TasteFrom, TasteNote};
use equipe_core::taste::{notes, store};
use equipe_core::protocol::rpc::{ImageRead, ReadImage};
use equipe_core::protocol::{Command, Event, Ticket};
use tokio::sync::broadcast;

pub fn mode() -> Option<String> {
    std::env::var("FAKE_DESIGN").ok().filter(|v| !v.is_empty() && v != "0")
}

/// Under the fake daemon's own home, so two of them never share it.
fn base() -> PathBuf {
    equipe_providers::credentials::equipe_home().join("design-system")
}

/// Where each fake design system lives.
fn places() -> Vec<Place> {
    let mut out = vec![Place { project: None, draft: None, dir: base().join("squad/design"), live: None, installed: None }];
    match mode().as_deref() {
        Some("draft") => out.push(Place { project: None, draft: Some(DraftOf { ticket: 144, title: "Brand guidelines".into(), working: false }), dir: base().join("t144/design"), live: None, installed: None }),
        Some("working") => out.push(Place { project: None, draft: Some(DraftOf { ticket: WORKING_TICKET, title: WORKING_TITLE.into(), working: true }), dir: base().join("t378/design"), live: None, installed: None }),
        Some("empty") | None => {}
        Some(_) => {
            out.push(Place { project: Some("web".into()), draft: None, dir: base().join("web/design"), live: None, installed: None });
            // T-170 in review: the site's refresh, its UI map shot again where it changed.
            out.push(Place { project: Some("web".into()), draft: Some(DraftOf { ticket: 170, title: REFRESH_TITLE.into(), working: false }), dir: base().join("t170/design"), live: None, installed: None });
        }
    }
    out
}

const REFRESH_TITLE: &str = "Site refresh — accent and buttons";
/// `working`: the ticket making the squad's UI map.
pub const WORKING_TICKET: u32 = 378;
const WORKING_TITLE: &str = "Make the UI map";

/// The ticket in review whose draft is shown: T-144 (`draft`), T-170 (`1`).
pub fn ticket() -> Option<Ticket> {
    if mode().as_deref() == Some("working") {
        let mut t = super::ticket(
            WORKING_TICKET,
            "in_progress",
            "app",
            "designer",
            WORKING_TITLE,
            "- [ ] The onboarding and checkout flows in flows/flows.json\n- [ ] Shoot every screen in each state, light and dark",
            &[],
            None,
            None,
        );
        t.kind = equipe_core::protocol::TicketKind::Task;
        t.project = None;
        return Some(t);
    }
    if mode().as_deref() == Some("1") {
        let mut t = super::ticket(
            170,
            "review",
            "web",
            "designer",
            REFRESH_TITLE,
            "- [ ] The accent to violet\n- [ ] A rounder primary button\n- [ ] Shoot the changed screens again",
            &[],
            Some("equipe/t-170-refresh"),
            Some("Changed the accent to #6D28D9 and refined the primary button. Shot the five changed screens again."),
        );
        t.kind = equipe_core::protocol::TicketKind::Code;
        return Some(t);
    }
    (mode().as_deref() == Some("draft")).then(|| {
        let mut t = super::ticket(
            144,
            "review",
            "app",
            "designer",
            "Brand guidelines",
            "- [ ] Logo, colour, type and voice settled\n- [ ] Light and dark contrast at AA or better\n- [ ] Tokens and guidelines in `design/`",
            &[],
            None,
            Some("Set up `design/` on the Signal blue option. Colour tokens light and dark, type, spacing, motion, three logos."),
        );
        t.kind = equipe_core::protocol::TicketKind::Task;
        t.project = None;
        t
    })
}

/// Write the folders. Called once the fake repositories are made.
pub fn seed() {
    let Some(mode) = mode() else { return };
    let _ = std::fs::remove_dir_all(base());
    if mode == "empty" {
        return;
    }
    let squad = base().join("squad/design");
    if mode == "draft" {
        write_system(&squad, &tokens(&STEEL), GUIDELINES_STEEL);
        write_system(&base().join("t144/design"), &tokens(&SIGNAL), GUIDELINES);
    } else if mode == "working" {
        // The worktree starts as the squad's folder; the map lands in it.
        write_system(&squad, &tokens(&SIGNAL), GUIDELINES);
        write_system(&base().join("t378/design"), &tokens(&SIGNAL), GUIDELINES);
        super::flows::seed_working(&base().join("t378/design"));
    } else {
        write_system(&squad, &tokens(&SIGNAL), GUIDELINES);
        write_system(&base().join("web/design"), &tokens(&SITE), GUIDELINES_WEB);
        super::flows::seed(&base().join("web/design"));
        write_system(&base().join("t170/design"), &tokens(&REFRESH), GUIDELINES_WEB);
        super::flows::seed_draft(&base().join("t170/design"), &base().join("web/design"));
        write_decisions(&squad, &base().join("web/design"));
    }
}

/// The squad's notes, kept for the run as its memory would keep them.
static SQUAD_NOTES: std::sync::Mutex<Vec<TasteNote>> = std::sync::Mutex::new(Vec::new());

fn seeded_notes() -> Vec<TasteNote> {
    let note = |text: &str, from: TasteFrom, date: &str, confirmed: bool, seen: &[u32]| TasteNote { text: text.into(), from, date: Some(date.into()), confirmed, seen: seen.to_vec(), ..TasteNote::default() };
    vec![
        note("Round corners — cards 12, buttons 8", TasteFrom::Person, "2026-10-21", true, &[]),
        note("Dislikes highly saturated blue", TasteFrom::SentBack { ticket: 336 }, "2026-10-28", true, &[]),
        note("Generous space, one emphasis per screen", TasteFrom::Answered { ticket: 272 }, "2026-11-01", false, &[]),
        note("Numbers in tabular figures, aligned", TasteFrom::Noticed { ticket: Some(288) }, "2026-11-03", false, &[301]),
    ]
}

/// What equipe kept of the person's send-backs, as the memory would give it.
fn sent_backs() -> Vec<equipe_core::protocol::design::Evidence> {
    let body = "## 2026-10-27 · T-336 (designer)\n- Sent back \u{201c}Pricing page\u{201d}: \u{201c}The blue is too loud. It hurts my eyes\u{201d}\n\n## 2026-11-02 · T-288 (designer)\n- Sent back \u{201c}Dashboard table\u{201d}: \u{201c}The numbers don\u{2019}t line up, so I can\u{2019}t compare them\u{201d}";
    equipe_core::taste::evidence::sent_backs(body)
}

/// Five decisions in the squad's record and one in the web project's, with their pictures.
fn write_decisions(squad: &Path, web: &Path) {
    use equipe_core::protocol::design::Decision;
    let d = |date: &str, question: &str, chosen: &str, picture: &str, passed: &[&str], why: Option<&str>, ticket: u32| Decision {
        date: date.into(),
        question: question.into(),
        chosen: chosen.into(),
        picture: Some(format!("decisions/{picture}")),
        passed: passed.iter().map(|p| (*p).to_owned()).collect(),
        why: why.map(str::to_owned),
        ticket: Some(ticket),
    };
    let pictures: [(&str, String); 5] = [
        ("mark-dot.svg", mark("#0A0A0C", "#EFEDE7", "#3F6FF5")),
        ("accent-signal.svg", button("#3F6FF5")),
        ("hero-airy.svg", card()),
        ("input-round.svg", input()),
        ("four-dots.svg", four_dots("#3F6FF5")),
    ];
    for (dir, list) in [
        (squad, vec![
            d("2026-11-03", "Logo direction", "One dot — a lowercase e and the brand dot", "mark-dot.svg", &["Four dots as they are", "Wordmark only"], Some("I'd like it to stay recognisable when small"), 144),
            d("2026-11-01", "Hero layout", "B — one column, generous space", "hero-airy.svg", &["A — two columns", "C — a full-bleed photo"], Some("Let it breathe a little more"), 272),
            d("2026-10-28", "Accent colour", "Signal blue #3F6FF5", "accent-signal.svg", &["Steel #2E5C8A"], Some("Not a loud blue — just on the one dot"), 336),
            d("2026-10-21", "Input corners", "16 — round", "input-round.svg", &["8", "Square"], None, 301),
        ]),
        (web, vec![d("2026-10-15", "Site illustration", "Four dots", "four-dots.svg", &["Photos"], Some("Our story told in dots"), 120)]),
    ] {
        std::fs::create_dir_all(dir.join("decisions")).unwrap();
        for (name, body) in &pictures {
            std::fs::write(dir.join("decisions").join(name), body).unwrap();
        }
        for (i, decision) in list.iter().enumerate() {
            std::fs::write(dir.join("decisions").join(format!("{}-{i}.md", decision.date)), equipe_core::taste::decisions::text(decision)).unwrap();
        }
    }
    std::fs::write(web.join(store::NOTES_FILE), format!("{}
- Dots and lines over photos _(the person, 2026-10-15)_
", store::TEMPLATE)).unwrap();
}

/// The taste as the daemon would send it now.
fn taste() -> Vec<Taste> {
    if mode().as_deref() != Some("1") {
        return vec![Taste { notes_at: "~/.equipe/squads/demo/home/memory/design-taste-notes.md".into(), ..Taste::default() }];
    }
    let mut notes = SQUAD_NOTES.lock().unwrap();
    if notes.is_empty() && std::env::var("FAKE_TASTE_EMPTY").is_err() {
        *notes = seeded_notes();
    }
    let web = base().join("web/design");
    let decisions = store::decisions_in(&base().join("squad/design"));
    let mut linked = notes.clone();
    equipe_core::taste::evidence::link(&mut linked, &decisions, &sent_backs());
    vec![
        Taste { project: None, notes_at: "~/.equipe/squads/demo/home/memory/design-taste-notes.md".into(), notes: linked, decisions },
        Taste {
            project: Some("web".into()),
            notes_at: web.join(store::NOTES_FILE).to_string_lossy().into_owned(),
            notes: std::fs::read_to_string(web.join(store::NOTES_FILE)).map(|t| notes::parse(&t)).unwrap_or_default(),
            decisions: store::decisions_in(&web),
        },
    ]
}

/// A note changed on the screen: the squad's for the run, the web project's in its file.
fn change_taste(project: Option<&str>, change: &equipe_core::protocol::design::TasteChange) -> Result<(), String> {
    let today = equipe_core::squad_memory::date(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()));
    match project {
        None => {
            let _ = taste();
            let mut kept = SQUAD_NOTES.lock().unwrap();
            *kept = notes::change(&kept, change, &today).map_err(|why| equipe_core::i18n::core(crate::lang()).fill(why.key(), &[("max", equipe_core::protocol::design::NOTE_MAX.to_string())]))?;
            Ok(())
        }
        Some(_) => {
            let path = base().join("web/design").join(store::NOTES_FILE);
            let before = std::fs::read_to_string(&path).unwrap_or_else(|_| store::TEMPLATE.into());
            let now = notes::change(&notes::parse(&before), change, &today).map_err(|why| why.key().to_owned())?;
            std::fs::write(&path, notes::write(&before, &now)).map_err(|e| e.to_string())
        }
    }
}

/// A palette: light and dark, by role.
struct Palette {
    bg: (&'static str, &'static str),
    surface: (&'static str, &'static str),
    sunken: (&'static str, &'static str),
    text: (&'static str, &'static str),
    muted: (&'static str, &'static str),
    accent: (&'static str, &'static str),
    on_accent: (&'static str, &'static str),
    dot: (&'static str, &'static str),
}

/// T-144's recommendation A: warm neutrals and one blue.
const SIGNAL: Palette = Palette {
    bg: ("#F6F5F1", "#0A0A0C"),
    surface: ("#FFFFFF", "#16171A"),
    sunken: ("#EAE7E0", "#111114"),
    text: ("#15161A", "#EFEDE7"),
    muted: ("#4F5159", "#A3A5AB"),
    accent: ("#1D4ED8", "#8FB0FF"),
    on_accent: ("#FFFFFF", "#0B1226"),
    dot: ("#3F6FF5", "#3F6FF5"),
};

/// B, as the desktop has it now: steel blue, a grey dot.
const STEEL: Palette = Palette {
    bg: ("#F6F5F1", "#0A0A0C"),
    surface: ("#FFFFFF", "#141417"),
    sunken: ("#ECEBE6", "#111114"),
    text: ("#15161A", "#EFEDE7"),
    muted: ("#5E6068", "#8A8C93"),
    accent: ("#2E5C8A", "#B4CDE6"),
    on_accent: ("#F6F5F1", "#0A0A0C"),
    dot: ("#7F8591", "#8A909B"),
};

/// The site's own, as it was before T-144.
const SITE: Palette = Palette {
    bg: ("#FAF8F4", "#121211"),
    surface: ("#F0EDE6", "#1B1B19"),
    sunken: ("#EAE6DD", "#262623"),
    text: ("#1A1A17", "#F2EFE8"),
    muted: ("#57544C", "#A9A59B"),
    accent: ("#1D4ED8", "#8FB0FF"),
    on_accent: ("#FFFFFF", "#0B1226"),
    dot: ("#1D4ED8", "#8FB0FF"),
};

/// T-170's: the site's, its accent violet.
const REFRESH: Palette = Palette { accent: ("#6D28D9", "#C4B5FD"), dot: ("#6D28D9", "#C4B5FD"), ..SITE };

/// `tokens.json` for a palette: colours as light and dark groups (as T-144
/// wrote them), status colours with `$extensions.mode`, aliases, type, space,
/// radius, shadow, motion.
fn tokens(p: &Palette) -> String {
    let side = |light: bool| {
        let v = |pair: (&str, &str)| if light { pair.0.to_owned() } else { pair.1.to_owned() };
        serde_json::json!({
            "bg": { "$value": v(p.bg), "$description": "Page and app background" },
            "surface": { "$value": v(p.surface), "$description": "Cards and sheets" },
            "sunken": { "$value": v(p.sunken), "$description": "Code and pressed surfaces" },
            "text": { "$value": v(p.text), "$description": "Body and headings" },
            "muted": { "$value": v(p.muted), "$description": "Secondary text" },
            "accent": { "$value": v(p.accent), "$description": "Links, selection, focus" },
            "onAccent": { "$value": v(p.on_accent), "$description": "Text on a filled accent" },
            "focus": { "$value": if light { "{color.light.accent}" } else { "{color.dark.accent}" } },
            "line": { "$value": if light { "rgba(10,10,12,.14)" } else { "rgba(233,230,223,.14)" }, "$description": "Hairline" }
        })
    };
    let tokens = serde_json::json!({
        "$description": "equipe brand tokens — from T-144 (docs/DESIGN-SYSTEM.md)",
        "color": {
            "$type": "color",
            "light": side(true),
            "dark": side(false),
            "status": {
                "ok": { "$value": "#1F6B33", "$extensions": { "mode": { "dark": "#7FCB8E" } }, "$description": "Done" },
                "work": { "$value": "#825600", "$extensions": { "mode": { "dark": "#E2B865" } }, "$description": "In progress" },
                "wait": { "$value": "#A04610", "$extensions": { "mode": { "dark": "#EC9A62" } }, "$description": "Your turn, waiting for permission" },
                "err": { "$value": "#AD2E27", "$extensions": { "mode": { "dark": "#EE7B74" } }, "$description": "Error" }
            },
            "link": { "$value": "{color.accent}" }
        },
        "brand": {
            "$type": "color",
            "dot": { "$value": p.dot.0, "$extensions": { "mode": { "dark": p.dot.1 } }, "$description": "The brand dot — a graphic for the logo only" },
            "tile": { "$value": "#0A0A0C", "$description": "Mark tile (void)" },
            "mark-fg": { "$value": "#EFEDE7", "$description": "Mark letter (bone)" }
        },
        "font": {
            "$type": "fontFamily",
            "sans": { "$value": ["Inter", "Noto Sans JP", "Pretendard Variable", "Noto Sans SC", "system-ui", "sans-serif"] },
            "mono": { "$value": ["JetBrains Mono", "ui-monospace", "Menlo", "monospace"] },
            "weight": {
                "$type": "fontWeight",
                "regular": { "$value": 400 },
                "semibold": { "$value": "semi-bold" },
                "bold": { "$value": 700 }
            }
        },
        "type": {
            "$type": "typography",
            "display": { "$value": { "fontFamily": "{font.sans}", "fontSize": "56px", "fontWeight": "{font.weight.bold}", "lineHeight": 1.05, "letterSpacing": "-0.02em" }, "$description": "Site H1 (36 on mobile)" },
            "title": { "$value": { "fontFamily": "{font.sans}", "fontSize": "32px", "fontWeight": "{font.weight.bold}", "lineHeight": 1.2 } },
            "heading": { "$value": { "fontFamily": "{font.sans}", "fontSize": "18px", "fontWeight": "{font.weight.semibold}", "lineHeight": "26px" } },
            "body": { "$value": { "fontFamily": "{font.sans}", "fontSize": "15px", "fontWeight": "{font.weight.regular}", "lineHeight": "25px" }, "$description": "Body 15/25 — 1.65 line height for CJK" },
            "small": { "$value": { "fontFamily": "{font.sans}", "fontSize": "13px", "fontWeight": "{font.weight.regular}", "lineHeight": "20px" } },
            "code": { "$value": { "fontFamily": "{font.mono}", "fontSize": "12.5px", "fontWeight": 400, "lineHeight": "19px" } }
        },
        "space": {
            "$type": "dimension",
            "1": { "$value": "4px" }, "2": { "$value": "8px" }, "3": { "$value": "12px" }, "4": { "$value": "16px" },
            "6": { "$value": "24px" }, "8": { "$value": "32px" }, "12": { "$value": "48px" }, "16": { "$value": "64px" }
        },
        "radius": {
            "$type": "dimension",
            "sm": { "$value": "6px", "$description": "Chips, small buttons" },
            "md": { "$value": "8px", "$description": "Controls, menus, rows" },
            "lg": { "$value": "12px", "$description": "Cards" },
            "xl": { "$value": "16px", "$description": "Inputs, dialogs" },
            "tile": { "$value": "14px", "$description": "Mark tile (22% of 64)" }
        },
        "shadow": {
            "$type": "shadow",
            "card": { "$value": { "color": "#0000001A", "offsetX": "0px", "offsetY": "1px", "blur": "3px", "spread": "0px" } },
            "float": { "$value": [{ "color": "#00000038", "offsetX": "0px", "offsetY": "8px", "blur": "24px", "spread": "-4px" }, { "color": "#0000001A", "offsetX": "0px", "offsetY": "2px", "blur": "6px", "spread": "0px" }], "$description": "Menus, popovers" },
            "dialog": { "$value": { "color": "#00000047", "offsetX": "0px", "offsetY": "24px", "blur": "48px", "spread": "-12px" } }
        },
        "motion": {
            "duration": {
                "$type": "duration",
                "fast": { "$value": "120ms", "$description": "Hover, tooltip, press" },
                "base": { "$value": "160ms", "$description": "Popovers, menus" },
                "slow": { "$value": "200ms", "$description": "A dialog or card arriving" },
                "slide": { "$value": "180ms", "$description": "Sidebar" }
            },
            "ease": {
                "$type": "cubicBezier",
                "out": { "$value": [0.33, 1, 0.68, 1], "$description": "The one curve — starts fast, lands softly" },
                "slide": { "$value": [0.22, 1, 0.36, 1] },
                "in-out": { "$value": "ease-in-out" }
            },
            "pop": { "$type": "transition", "$value": { "duration": "{motion.duration.base}", "delay": "0ms", "timingFunction": "{motion.ease.out}" } }
        }
    });
    serde_json::to_string_pretty(&tokens).unwrap()
}

fn write_system(dir: &Path, tokens: &str, guidelines: &str) {
    for sub in ["assets/logo", "assets/icons", "assets/illustrations", "components"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    std::fs::write(dir.join("tokens.json"), tokens).unwrap();
    std::fs::write(dir.join("guidelines.md"), guidelines).unwrap();
    let dot = if tokens.contains("#7F8591") { "#7F8591" } else { "#3F6FF5" };
    let files: [(&str, String); 11] = [
        ("assets/logo/mark.svg", mark("#0A0A0C", "#EFEDE7", dot)),
        ("assets/logo/mark-mono-on-dark.svg", mark("none", "#EFEDE7", "#EFEDE7")),
        ("assets/logo/mark-mono-on-light.svg", mark("none", "#15161A", "#15161A")),
        ("assets/icons/inbox.svg", icon(r#"<path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>"#)),
        ("assets/icons/check.svg", icon(r#"<path d="M20 6 9 17l-5-5"/>"#)),
        ("assets/icons/bell.svg", icon(r#"<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>"#)),
        ("assets/illustrations/four-dots.svg", four_dots(dot)),
        ("components/button.svg", button(dot)),
        ("components/card.svg", card()),
        ("components/input.svg", input()),
        ("assets/illustrations/README.txt", "not a picture: left out of the grid".into()),
    ];
    for (name, body) in files {
        std::fs::write(dir.join(name), body).unwrap();
    }
    // A raster one too: the social card, a PNG with the four dots on void.
    social_card(dot).save(dir.join("assets/illustrations/social-card.png")).unwrap();
}

/// The mark: a tile (or none), a lowercase `e` drawn as a stroke, the dot.
fn mark(tile: &str, ink: &str, dot: &str) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024"><rect width="1024" height="1024" rx="225" fill="{tile}"/><path d="M300 540 H650 A182 182 0 1 0 600 668" fill="none" stroke="{ink}" stroke-width="86"/><circle cx="772" cy="690" r="64" fill="{dot}"/></svg>"##
    )
}

fn icon(body: &str) -> String {
    format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="96" height="96" fill="none" stroke="#4F5159" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"##)
}

fn four_dots(dot: &str) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200"><circle cx="64" cy="64" r="34" fill="{dot}"/><circle cx="136" cy="64" r="32" fill="none" stroke="#A3A5AB" stroke-width="4"/><circle cx="64" cy="136" r="32" fill="none" stroke="#A3A5AB" stroke-width="4"/><circle cx="136" cy="136" r="32" fill="none" stroke="#A3A5AB" stroke-width="4"/></svg>"##
    )
}

fn button(dot: &str) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 120"><rect x="20" y="36" width="132" height="44" rx="8" fill="#15161A"/><rect x="48" y="54" width="76" height="8" rx="4" fill="#F6F5F1"/><rect x="168" y="36" width="132" height="44" rx="8" fill="none" stroke="#0A0A0C24" stroke-width="2"/><rect x="196" y="54" width="62" height="8" rx="4" fill="#15161A"/><circle cx="276" cy="58" r="5" fill="{dot}"/></svg>"##
    )
}

fn card() -> String {
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 200"><rect x="24" y="24" width="272" height="152" rx="12" fill="#FFFFFF" stroke="#0A0A0C24" stroke-width="2"/><rect x="44" y="48" width="120" height="12" rx="6" fill="#15161A"/><rect x="44" y="76" width="220" height="8" rx="4" fill="#4F515966"/><rect x="44" y="94" width="190" height="8" rx="4" fill="#4F515966"/><rect x="44" y="132" width="72" height="24" rx="6" fill="#1D4ED8"/></svg>"##.into()
}

fn input() -> String {
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 120"><rect x="20" y="38" width="280" height="44" rx="16" fill="#FFFFFF" stroke="#1D4ED8" stroke-width="2"/><rect x="40" y="56" width="2" height="10" fill="#15161A"/><rect x="52" y="57" width="120" height="8" rx="4" fill="#4F515955"/><circle cx="276" cy="60" r="12" fill="#15161A"/></svg>"##.into()
}

/// 1200×630: void, the four dots, one of them the brand's.
fn social_card(dot: &str) -> image::RgbaImage {
    let blue = u32::from_str_radix(dot.trim_start_matches('#'), 16).unwrap_or(0x3F6FF5);
    let (bw, bh) = (1200u32, 630u32);
    image::RgbaImage::from_fn(bw, bh, |x, y| {
        let d = |cx: f32, cy: f32| ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
        let filled = d(520., 250.) < 70.;
        let ring = [(680., 250.), (520., 410.), (680., 410.)].iter().any(|(cx, cy)| (d(*cx, *cy) - 66.).abs() < 4.);
        if filled {
            image::Rgba([(blue >> 16) as u8, (blue >> 8) as u8, blue as u8, 255])
        } else if ring {
            image::Rgba([0xA3, 0xA5, 0xAB, 255])
        } else {
            image::Rgba([0x0A, 0x0A, 0x0C, 255])
        }
    })
}

/// The design systems as the daemon would send them now.
fn event() -> Event {
    let (mut systems, folder) = folder::catalog_of(&places(), crate::lang());
    super::flows::dress(&mut systems);
    // As the daemon does while a draft is in review: what each picture holds.
    folder::fingerprint(&mut systems);
    // The web project's designer is at its map: brief, logo and system done
    // (`working`: the squad's, its map under way).
    let at_map = if mode().as_deref() == Some("working") { None } else { Some("web") };
    for web in systems.iter_mut().filter(|s| s.project.as_deref() == at_map && s.draft.is_none()) {
        use equipe_core::protocol::design::{Setup, SetupStep};
        web.setup = [Setup::Brief, Setup::Logo, Setup::System, Setup::Flows, Setup::Qa].map(|step| SetupStep { step, done: !matches!(step, Setup::Flows | Setup::Qa) }).to_vec();
    }
    Event::Design { squad: "demo".into(), systems, folder, taste: taste() }
}

/// `design`: answered at once, and the folders watched from then on.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    if let Command::Taste { project, change, .. } = command {
        match change_taste(project.as_deref(), change) {
            Ok(()) => {
                let _ = events.send(event());
            }
            Err(message) => {
                let _ = events.send(Event::Error { session: None, message, next: None });
            }
        }
        return true;
    }
    match command {
        Command::DesignVersions { project, .. } => {
            let _ = events.send(Event::DesignVersions { squad: "demo".into(), project: project.clone(), versions: versions(project.as_deref()) });
            return true;
        }
        // Asked back: a ticket for the designer, as the daemon makes one.
        Command::RestoreDesign { project, commit, .. } => {
            let version = versions(project.as_deref()).into_iter().find(|v| v.commit.starts_with(commit.as_str()));
            let event = match version {
                Some(v) => {
                    let (title, body) = equipe_core::design::versions::restore(&v, crate::lang());
                    let mut t = super::ticket(171, "todo", project.as_deref().unwrap_or("app"), "designer", &title, &body, &[], None, None);
                    t.kind = if project.is_some() { equipe_core::protocol::TicketKind::Code } else { equipe_core::protocol::TicketKind::Task };
                    Event::TicketChanged { squad: "demo".into(), ticket: t }
                }
                None => Event::Error { session: None, message: format!("Couldn't find that design version ({commit})."), next: None },
            };
            let _ = events.send(event);
            return true;
        }
        Command::Design { .. } => {}
        _ => return false,
    }
    let _ = events.send(event());
    static WATCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !WATCHING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let events = events.clone();
        tokio::spawn(async move {
            let mut last = folder::stamp(&places());
            loop {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                let now = folder::stamp(&places());
                if now != last {
                    last = now;
                    let _ = events.send(event());
                }
            }
        });
        if let Some(after) = std::env::var("FAKE_DESIGN_CHANGE_AFTER").ok().and_then(|v| v.parse::<u64>().ok()) {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(after)).await;
                let dir = base().join("squad/design");
                let Ok(text) = std::fs::read_to_string(dir.join("tokens.json")) else { return };
                // The designer moves the accent a step and adds a highlight.
                let moved = text.replacen("\"#1D4ED8\"", "\"#2251E0\"", 1).replacen("\"#2E5C8A\"", "\"#2B5F94\"", 1).replacen(
                    "\"link\": {",
                    "\"highlight\": { \"$value\": \"#FFE58A\", \"$extensions\": { \"mode\": { \"dark\": \"#5C4A12\" } }, \"$description\": \"Search match\" },\n      \"link\": {",
                    1,
                );
                let _ = std::fs::write(dir.join("tokens.json"), moved);
            });
        }
    }
    true
}

/// A system's accepted versions, newest first, as `git log` would have them.
fn versions(project: Option<&str>) -> Vec<DesignVersion> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let v = |n: u8, days_ago: u64, subject: &str, ticket: Option<u32>, parts: &[DesignPart]| DesignVersion {
        commit: format!("{:02x}{}", n, "7c1e9a4b3d".repeat(4)).chars().take(40).collect(),
        at: now.saturating_sub(days_ago * 86_400 + u64::from(n) * 3_600),
        subject: subject.into(),
        ticket,
        parts: parts.to_vec(),
    };
    use DesignPart::{Flows, Guidelines, Logo, Other, Tokens};
    match project {
        Some(_) => vec![
            v(1, 1, "Shoot the payment screen's error state again", Some(152), &[Flows]),
            v(2, 3, "Search results empty and loading", Some(160), &[Flows]),
            v(3, 6, "Onboarding UI map", Some(142), &[Flows, Guidelines]),
            v(4, 9, "Site tokens renamed to the squad's names", Some(131), &[Tokens, Guidelines]),
            v(5, 21, "the person: a slightly warmer background", None, &[Tokens]),
            v(6, 40, "Site design tokens, first", Some(98), &[Tokens, Logo, Other]),
        ],
        None => vec![
            v(7, 2, "Brand guidelines v1", Some(144), &[Tokens, Logo, Guidelines]),
            v(8, 12, "Three logos", Some(120), &[Logo]),
            v(9, 30, "equipe: the squad's folder", None, &[Other]),
        ],
    }
}

/// `read_image` for a design system's picture, as the daemon checks it:
/// only one an asset names, inside its folder.
pub fn read_image(ask: &ReadImage) -> Result<ImageRead, String> {
    let places = places();
    let (mut systems, _) = folder::catalog_of(&places, Lang::En);
    super::flows::dress(&mut systems);
    let decided = places.iter().flat_map(|p| store::pictures(&p.dir)).any(|p| p == ask.path);
    if !decided && !systems.iter().any(|s| s.assets.iter().any(|a| a.path == ask.path)) && !super::flows::pictures(&systems).contains(&ask.path) {
        return Err(format!("{} isn't a picture {} shows", ask.path, ask.squad));
    }
    let roots: Vec<PathBuf> = places.into_iter().map(|p| p.dir).collect();
    equipe_core::visuals::read(Path::new(&ask.path), &roots, ask.max)
}

const GUIDELINES: &str = r#"# equipe brand guidelines v1

Status: **settled v1** — the person picked the recommendation in all four (D1 ① · D2 A · D3 1 · D4 one register per language). Tokens in `tokens.json`, pictures in `assets/`.

## 1. Logo

### 1.1 Parts
- **Mark `e.`**: a void `#0A0A0C` tile + a bone `#EFEDE7` lowercase `e` + the brand dot. App icon, favicon, avatar. Corner radius = 22% of a side.
- **What the dot means**: the site's four dots ("the filled dot = the one who leads") brought down to one. The four dots stay as an illustration.

### 1.2 How to use it
- **Clear space**: ¼ of the mark's side — nothing comes inside it.
- **Smallest size**: the mark at 16px. At 16px the dot stays at least 2px.
- **Background**: on light, letters `#15161A`; on dark, bone `#EFEDE7`. The dot is `#3F6FF5` on both.
- **Don't**: recolour the dot or the letter, stretch or slant it, add shadows or gradients, or set it on a photo.

## 2. Colour

### 2.1 Principles
1. **Only two kinds of colour: the brand blue (just one) and the status colours.** Everything else is a warm neutral (bone, void).
2. Light and dark use the same **role names**; only the values differ by mode.
3. Text pairs meet WCAG 2.x AA (4.5:1), graphics 3:1 or more. Colour alone never carries a state.

### 2.2 Role tokens

| Role | Light | Dark | Used for |
|---|---|---|---|
| `bg` | `#F6F5F1` | `#0A0A0C` | Page and app background |
| `surface` | `#FFFFFF` | `#16171A` | Cards and sheets |
| `text` | `#15161A` | `#EFEDE7` | Body and headings |
| `muted` | `#4F5159` | `#A3A5AB` | Secondary text |
| `accent` | `#1D4ED8` | `#8FB0FF` | Links, selection, focus |
| `dot` | `#3F6FF5` | `#3F6FF5` | The brand dot (logo only) |

> The dot's "3:1 against the background" is the graphics threshold. Never use the dot as a **text colour** (4.01:1 on the light bg).

### 2.3 Status colours
- amber = at work, orange = waiting on the person, red = error, green = done. Never a project's colour.

## 3. Type

### 3.1 Stack
- **Web**: Inter, with a face made for each script the product speaks (Noto Sans JP for Japanese, Pretendard for Korean, Noto Sans SC/TC for Chinese). **Apps**: the platform's system font.
- **Mono**: JetBrains Mono, falling back to `ui-monospace, Menlo`.
- Weights: headings 700 (tracking −0.02em, 0 for CJK), body 400, buttons and labels 600.

### 3.2 Scripts
- Line breaking by script: Korean `word-break: keep-all`; Japanese and Chinese between characters. CJK body line height 1.65. Never below 12px.

## 4. Space and corners
- 4 · 8 · 12 · 16 · 24 · 32 · 48 · 64. Nothing in between.
- Radii: chip 6, control 8, card 12, input and dialog 16.

## 5. Motion
- Three durations (120 · 160 · 200ms) and one curve (ease-out). Only the sidebar is 180ms quint.
- With "Reduce motion" on, the end state shows at once.

## 6. Voice
Short, active, concrete. What the person does comes first. No hype words, exclamation marks, emoji or ALL CAPS decoration. One register throughout in every language.
"#;

const GUIDELINES_STEEL: &str = r#"# equipe design system (current)

The values the desktop app uses today. Once T-144's brand guidelines are accepted, this folder becomes them.

## Colour
- The accent is steel blue `#2E5C8A` (dark `#B4CDE6`). The dot is chrome grey.
- The product UI's filled primary button is a `text`-coloured surface.

## Type
- The app uses the system font (Helvetica Neue), mono is Menlo.
"#;

const GUIDELINES_WEB: &str = r#"# Site design tokens

The values the equipe.im site uses. The same names as the squad's design system, only different values.

## Colour
- The background is a slightly warmer `#FAF8F4`. Links and buttons are `#1D4ED8`.

## Type
- Inter (self-hosted), with a face for each script. H1 56/1.05, body 18/1.6.
"#;
