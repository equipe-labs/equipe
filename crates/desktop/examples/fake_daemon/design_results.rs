//! `FAKE_DESIGN=1 FAKE_DESIGN_RESULTS=1`: what design tickets made, for the
//! Designs tab, the Needs-you item and the Status line — laid out in the web
//! project's `design/directions/` the way a designer leaves them, and read
//! the daemon's way (`equipe_core::design::results`), pictures and all:
//!
//! - T-2562, accepted by the PM: a dashboard in three directions, each drawn at
//!   1440 and 390 beside the screen as it was, a `CHANGES.md` with a section
//!   for each, a decision record that chose B, preview links in a comment of
//!   a follow-up ticket.
//! - T-2563, handed in: a settings page in one direction, no notes.
//! - T-2565, at work: its first mock.
//!
//! Every picture is drawn here, from rectangles; the names, the notes and the
//! addresses are made up (`acme`, `vercel.app` previews of nobody's).

use std::path::{Path, PathBuf};

use equipe_core::design::results::{self, Where};
use equipe_core::protocol::rpc::{ImageRead, ReadImage};
use equipe_core::protocol::title::TicketLabel;
use equipe_core::protocol::{Command, Event, Ticket, TicketEntry, TicketKind};
use image::RgbaImage;
use tokio::sync::broadcast;

use super::design::base;
use super::refgallery::{bar, boxed, rrect};

type Color = [u8; 4];

const PAGE: Color = [0xF4, 0xF5, 0xF7, 255];
const CARD: Color = [0xFF, 0xFF, 0xFF, 255];
const LINE: Color = [0xDD, 0xE0, 0xE6, 255];
const INK: Color = [0x23, 0x27, 0x30, 255];
const SOFT: Color = [0xB8, 0xBD, 0xC8, 255];
const FAINT: Color = [0xE8, 0xEA, 0xEF, 255];
const BLUE: Color = [0x35, 0x62, 0xE8, 255];
const GREEN: Color = [0x2F, 0x9E, 0x6B, 255];
const AMBER: Color = [0xE3, 0xA2, 0x2B, 255];
const CREAM: Color = [0xFA, 0xF7, 0xF0, 255];
const SAND: Color = [0xE9, 0xE2, 0xD3, 255];

pub fn on() -> bool {
    super::design::mode().is_some() && std::env::var("FAKE_DESIGN_RESULTS").is_ok_and(|v| v == "1")
}

fn project() -> PathBuf {
    base().join("web")
}

fn designs() -> PathBuf {
    project().join("design")
}

/// The design tickets on the board.
pub fn tickets() -> Vec<Ticket> {
    if !on() {
        return Vec::new();
    }
    let make = |id: u32, status: &str, title: &str, note: &str| {
        let mut t = super::ticket(id, status, "web", "designer", title, "- [ ] Directions for the screen, each beside the screen as it is", &[], Some(&format!("equipe/t-{id}-design")), Some(note));
        t.label = Some(TicketLabel::Design);
        t.kind = TicketKind::Code;
        t
    };
    let mut done = make(2562, "done", "Dashboard — three directions", HAND_IN_2562);
    done.merged = Some(true);
    done.branch = None;
    let mut preview = super::ticket(2564, "todo", "web", "frontend", "Preview deployments for the dashboard directions", "", &[2562], None, None);
    preview.label = Some(TicketLabel::Chore);
    vec![done, make(2563, "review", "Settings page — a calmer layout", HAND_IN_2563), make(2565, "in_progress", "Empty states for the dashboard", "First mock drawn."), preview]
}

const HAND_IN_2562: &str = "Three directions for the dashboard, each at 1440 and 390 next to the screen as it is. B is the one I recommend: the KPIs first, the table under them.\n\nHero: design/directions/t2562-references/boards/mock-b-1440.png\nBefore: design/directions/t2562-references/boards/before-1440.png\n\nNotes per direction are in design/directions/t2562-dashboard/CHANGES.md.";
const HAND_IN_2563: &str = "A calmer settings page: groups with one heading each, switches on the right.\n\nHero: design/directions/t2563-settings/boards/mock-1440.png\nBefore: design/directions/t2563-settings/boards/before-1440.png";

const CHANGES_2562: &str = "# T-2562 dashboard — changes\n\nThe three directions keep every figure, filter and export the dashboard has today.\n\n## Direction A — Calm ledger\n### Before → After → Why\n- Cards → one ledger table: the page reads as a list of what happened, as the references' reports do\n- Row height 56px → 40px, because the references' lists run 36–44px\n- One accent, used on the primary action only\n\n### Unchanged\nFeatures, figures, copy.\n\n## Direction B — KPIs first\n### Before → After → Why\n- Six equal cards → four KPI tiles with a sparkline, then the table: the first screenful answers \"how are we doing\"\n- Filters moved from a side panel to one bar above the table, as in the references' data tools\n- Type scale 14/20 → 15/24 for the table: figures are tabular now\n\n### Unchanged\nFeatures, figures, copy.\n\n## Direction C — Dense\n### Before → After → Why\n- Twelve compact tiles and a split view: for the person who lives in it all day\n- Spacing on the 4px scale, 8px between tiles\n\n### Unchanged\nFeatures, figures, copy.\n";

const DECISION_2562: &str = "# Foundation direction for the dashboard\n\nQuestion: which direction does the dashboard take?\nChose: Direction B — KPIs first\nPicture: design/directions/t2562-references/boards/mock-b-1440.png\nPassed over: A (too quiet for daily use), C (too dense)\nWhy: the first screenful answers how the team is doing, and the table stays one scroll away.\nTicket: T-2562\n";

/// What the ticket's record and its follow-up's hold: the hand-in and a comment with the previews.
fn records(id: u32) -> (Vec<TicketEntry>, Vec<Vec<TicketEntry>>) {
    let entry = |by: &str, kind: &str, text: &str| TicketEntry { at: 1, by: by.into(), kind: kind.into(), text: text.into() };
    match id {
        2562 => (
            vec![entry("designer", "handed in", "Handed in the three directions."), entry("pm", "accepted by the PM", "Looked at all three at 1440 and 390; B reads best.")],
            vec![vec![entry("frontend", "comment", "Previews are up:\nA — https://t2562-a-git-dashboard-acme.vercel.app/a\nB — https://t2562-b-git-dashboard-acme.vercel.app/b\nDirection C: https://t2562-c-git-dashboard-acme.vercel.app/c")]],
        ),
        _ => (Vec::new(), Vec::new()),
    }
}

/// `design_results`: read the folders the daemon's way, the board's tickets.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    let Command::DesignResults { .. } = command else { return false };
    let mut designs_found = Vec::new();
    if on() {
        let board = super::tickets().lock().map(|t| t.clone()).unwrap_or_default();
        let at = Where { squad_home: base().join("squad"), projects: &[project()] };
        for t in board.into_iter().filter(|t| matches!(t.id, 2562 | 2563 | 2565)) {
            // The project is where the folders are written, not the fake repository the board names.
            let ticket = Ticket { project: Some(project()), worktree: None, ..t };
            let (entries, followups) = records(ticket.id);
            designs_found.extend(results::build(&ticket, &entries, &followups, &at));
        }
        designs_found.sort_by_key(|d| std::cmp::Reverse((d.updated, d.ticket)));
    }
    let _ = events.send(Event::DesignResults { squad: "demo".into(), designs: designs_found });
    true
}

/// `read_image` for a render of the web project's `design/directions/`: inside it, links resolved.
pub fn read_image(ask: &ReadImage) -> Result<ImageRead, String> {
    let root = designs().join("directions");
    let path = Path::new(&ask.path);
    if on() && path.starts_with(&root) {
        return equipe_core::visuals::read(path, std::slice::from_ref(&root), ask.max);
    }
    Err(format!("{} isn't a picture {} shows", ask.path, ask.squad))
}

// ── the folders ──

/// Write the folders the tickets above made.
pub fn seed() {
    if !on() {
        return;
    }
    let d = designs();
    let dashboard = d.join("directions/t2562-dashboard");
    let boards = d.join("directions/t2562-references/boards");
    let _ = std::fs::create_dir_all(&dashboard);
    let _ = std::fs::create_dir_all(&boards);
    let _ = std::fs::write(dashboard.join("CHANGES.md"), CHANGES_2562);
    let _ = std::fs::create_dir_all(d.join("decisions"));
    let _ = std::fs::write(d.join("decisions/2026-11-14-foundation-direction.md"), DECISION_2562);
    save(&boards.join("before-1440.png"), dashboard_of(Look::Before, false));
    save(&boards.join("before-390.png"), dashboard_of(Look::Before, true));
    for (key, look) in [("a", Look::Ledger), ("b", Look::Kpis), ("c", Look::Dense)] {
        save(&boards.join(format!("mock-{key}-1440.png")), dashboard_of(look, false));
        save(&boards.join(format!("mock-{key}-390.png")), dashboard_of(look, true));
    }
    let settings = d.join("directions/t2563-settings/boards");
    let _ = std::fs::create_dir_all(&settings);
    save(&settings.join("before-1440.png"), dashboard_of(Look::Before, false));
    save(&settings.join("mock-1440.png"), dashboard_of(Look::Calm, false));
    let empty = d.join("directions/t2565-empty/boards");
    let _ = std::fs::create_dir_all(&empty);
    save(&empty.join("mock-a-1440.png"), dashboard_of(Look::Calm, false));
}

fn save(path: &Path, img: RgbaImage) {
    let _ = img.save(path);
}

#[derive(Clone, Copy)]
enum Look {
    Before,
    Ledger,
    Kpis,
    Dense,
    Calm,
}

fn fill(img: &mut RgbaImage, c: Color) {
    for p in img.pixels_mut() {
        *p = image::Rgba(c);
    }
}

fn spark(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, seed: i32, c: Color) {
    for i in 0..w / 6 {
        let v = ((i * 7 + seed * 5) % 11) as i32 * h / 14 + h / 8;
        bar(img, x + i * 6, y + h - v, 4, v, c);
    }
}

fn rows(img: &mut RgbaImage, x: i32, y: i32, w: i32, n: i32, row: i32, ink: Color) {
    for i in 0..n {
        let ry = y + i * row;
        rrect(img, x, ry + row - 1, w, 1, 0, FAINT);
        bar(img, x + 16, ry + row / 2 - 4, 90 + (i * 37) % 60, 8, ink);
        bar(img, x + w / 2, ry + row / 2 - 4, 60 + (i * 23) % 40, 8, SOFT);
        bar(img, x + w - 90, ry + row / 2 - 4, 54, 8, if i % 4 == 0 { AMBER } else { GREEN });
    }
}

/// A dashboard, as a wireframe: the screen as it was, or one of three directions; 1440 or 390 wide.
fn dashboard_of(look: Look, mobile: bool) -> RgbaImage {
    let (w, h) = if mobile { (390, 844) } else { (1440, 900) };
    let mut img = RgbaImage::new(w as u32, h as u32);
    let (w, h) = (w as i32, h as i32);
    fill(&mut img, match look { Look::Ledger | Look::Calm => CREAM, _ => PAGE });
    // The top bar.
    rrect(&mut img, 0, 0, w, 56, 0, CARD);
    rrect(&mut img, 0, 55, w, 1, 0, LINE);
    bar(&mut img, 24, 20, 96, 16, INK);
    if !mobile {
        for i in 0..4 {
            bar(&mut img, 220 + i * 90, 24, 60, 8, SOFT);
        }
    }
    bar(&mut img, w - 56, 16, 24, 24, BLUE);
    let pad = if mobile { 16 } else { 40 };
    let inner = w - 2 * pad;
    match look {
        Look::Before => {
            // Six equal cards, each with a chart and a number.
            let cols = if mobile { 1 } else { 3 };
            let (cw, ch) = ((inner - (cols - 1) * 16) / cols, if mobile { 110 } else { 250 });
            for i in 0..6 {
                let (cx, cy) = (pad + (i % cols) * (cw + 16), 84 + (i / cols) * (ch + 16));
                if cy + ch > h {
                    break;
                }
                boxed(&mut img, cx, cy, cw, ch, 10, CARD, LINE);
                bar(&mut img, cx + 16, cy + 16, 80, 8, SOFT);
                bar(&mut img, cx + 16, cy + 36, 120, 20, INK);
                spark(&mut img, cx + 16, cy + 70, cw - 32, ch - 90, i, if i % 2 == 0 { BLUE } else { AMBER });
            }
        }
        Look::Ledger => {
            bar(&mut img, pad, 92, 220, 28, INK);
            bar(&mut img, pad, 130, 360.min(inner), 10, SOFT);
            boxed(&mut img, pad, 168, inner, h - 168 - 40, 6, CARD, SAND);
            rows(&mut img, pad, 176, inner, ((h - 168 - 60) / 40).min(16), 40, INK);
        }
        Look::Kpis => {
            let cols = if mobile { 2 } else { 4 };
            let (cw, ch) = ((inner - (cols - 1) * 16) / cols, if mobile { 96 } else { 128 });
            for i in 0..4 {
                let (cx, cy) = (pad + (i % cols) * (cw + 16), 84 + (i / cols) * (ch + 16));
                boxed(&mut img, cx, cy, cw, ch, 10, CARD, LINE);
                bar(&mut img, cx + 16, cy + 16, 70, 8, SOFT);
                bar(&mut img, cx + 16, cy + 36, 90, 26, INK);
                spark(&mut img, cx + 16, cy + ch - 40, cw - 32, 28, i * 3, BLUE);
            }
            let top = 84 + if mobile { 2 * 112 } else { 144 };
            bar(&mut img, pad, top + 8, inner.min(520), 28, FAINT);
            boxed(&mut img, pad, top + 52, inner, h - top - 52 - 32, 8, CARD, LINE);
            rows(&mut img, pad, top + 56, inner, ((h - top - 100) / 48).min(12), 48, INK);
        }
        Look::Dense => {
            let cols = if mobile { 3 } else { 6 };
            let (cw, ch) = ((inner - (cols - 1) * 8) / cols, 84);
            for i in 0..(cols * 2) {
                let (cx, cy) = (pad + (i % cols) * (cw + 8), 76 + (i / cols) * (ch + 8));
                boxed(&mut img, cx, cy, cw, ch, 6, CARD, LINE);
                bar(&mut img, cx + 10, cy + 12, 40, 6, SOFT);
                bar(&mut img, cx + 10, cy + 28, 56, 16, INK);
                spark(&mut img, cx + 10, cy + 52, cw - 20, 24, i, if i % 3 == 0 { AMBER } else { BLUE });
            }
            let top = 76 + 2 * (ch + 8) + 8;
            let half = if mobile { inner } else { (inner - 8) / 2 };
            boxed(&mut img, pad, top, half, h - top - 24, 6, CARD, LINE);
            rows(&mut img, pad, top + 4, half, ((h - top - 40) / 32).min(18), 32, INK);
            if !mobile {
                boxed(&mut img, pad + half + 8, top, half, h - top - 24, 6, CARD, LINE);
                rows(&mut img, pad + half + 8, top + 4, half, ((h - top - 40) / 32).min(18), 32, INK);
            }
        }
        Look::Calm => {
            bar(&mut img, pad, 92, 180, 26, INK);
            for g in 0..3 {
                let gy = 150 + g * 220;
                bar(&mut img, pad, gy, 120, 10, SOFT);
                boxed(&mut img, pad, gy + 24, inner.min(760), 168, 8, CARD, SAND);
                for r in 0..3 {
                    bar(&mut img, pad + 20, gy + 48 + r * 48, 160, 8, INK);
                    rrect(&mut img, pad + inner.min(760) - 64, gy + 42 + r * 48, 40, 22, 11, if r == 1 { SAND } else { BLUE });
                }
            }
        }
    }
    img
}
