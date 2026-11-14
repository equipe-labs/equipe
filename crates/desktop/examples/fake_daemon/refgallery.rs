//! `FAKE_DESIGN`'s references: what a designer keeps in `design/references/`
//! (`skills/design-references`) — a folder for each interface pattern with
//! its pictures and a `notes.md`, and `sources.md` — for the References tab.
//! The squad's own hold a KPI tile, a data table, an empty state and a toast;
//! the web project's hold a filter bar and a data table of its own, so its
//! tab shows the squad's for the rest ("Squad's" on them).
//!
//! Every picture is drawn here, from rectangles: wireframes in the manner of
//! each pattern, never a screenshot of anyone's product. The names and the
//! addresses in the notes are made up (`example.com` and kin).

use std::path::{Path, PathBuf};

use equipe_core::design::refgallery::{self, Found};
use equipe_core::protocol::{Command, Event};
use image::{Rgba, RgbaImage};
use tokio::sync::broadcast;

use super::design::base;

const W: u32 = 960;
const H: u32 = 600;

type Color = [u8; 4];

const PAGE: Color = [0xF4, 0xF5, 0xF7, 255];
const CARD: Color = [0xFF, 0xFF, 0xFF, 255];
const LINE: Color = [0xDD, 0xE0, 0xE6, 255];
const INK: Color = [0x23, 0x27, 0x30, 255];
const SOFT: Color = [0xB8, 0xBD, 0xC8, 255];
const FAINT: Color = [0xE8, 0xEA, 0xEF, 255];
const GOOD: Color = [0x2F, 0x9E, 0x6B, 255];
const BAD: Color = [0xD9, 0x4B, 0x4B, 255];
const BLUE: Color = [0x35, 0x62, 0xE8, 255];
const AMBER: Color = [0xE3, 0xA2, 0x2B, 255];

/// A rounded rectangle, as a wireframe is drawn.
fn rrect(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, r: i32, c: Color) {
    for py in y.max(0)..(y + h).min(img.height() as i32) {
        for px in x.max(0)..(x + w).min(img.width() as i32) {
            let (dx, dy) = ((x + r - px).max(px - (x + w - 1 - r)).max(0), (y + r - py).max(py - (y + h - 1 - r)).max(0));
            if dx * dx + dy * dy <= r * r {
                img.put_pixel(px as u32, py as u32, Rgba(c));
            }
        }
    }
}

/// A rounded rectangle with a one-pixel edge.
fn boxed(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, r: i32, fill: Color, edge: Color) {
    rrect(img, x, y, w, h, r, edge);
    rrect(img, x + 1, y + 1, w - 2, h - 2, (r - 1).max(0), fill);
}

fn bar(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, c: Color) {
    rrect(img, x, y, w, h, h / 2, c);
}

/// A line of small steps: a sparkline.
fn spark(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, seed: i32, c: Color) {
    let n = 24;
    let mut last = h / 2;
    for i in 0..n {
        let wobble = ((i * 7 + seed * 13) % 9) - 4;
        last = (last + wobble * h / 40 + (i * h) / (n * 6)).clamp(2, h - 2);
        let top = y + h - last;
        rrect(img, x + i * w / n, top, (w / n - 1).max(2), 3, 1, c);
    }
}

fn canvas() -> RgbaImage {
    RgbaImage::from_pixel(W, H, Rgba(PAGE))
}

// ── KPI tile ──

fn kpi_row() -> RgbaImage {
    let mut img = canvas();
    for i in 0..4 {
        let x = 40 + i * 225;
        boxed(&mut img, x, 150, 205, 130, 12, CARD, LINE);
        bar(&mut img, x + 18, 172, 70, 8, SOFT);
        bar(&mut img, x + 18, 198, 110, 26, INK);
        bar(&mut img, x + 18, 244, 52, 14, if i == 2 { BAD } else { GOOD });
        bar(&mut img, x + 80, 247, 56, 8, FAINT);
    }
    boxed(&mut img, 40, 320, 880, 230, 12, CARD, LINE);
    bar(&mut img, 62, 342, 120, 10, SOFT);
    spark(&mut img, 62, 380, 836, 140, 3, BLUE);
    img
}

fn kpi_accent() -> RgbaImage {
    let mut img = canvas();
    for i in 0..3 {
        let x = 40 + i * 300;
        boxed(&mut img, x, 120, 280, 160, 12, CARD, LINE);
        rrect(&mut img, x, 120, 6, 160, 3, [BLUE, GOOD, AMBER][i as usize]);
        bar(&mut img, x + 28, 148, 90, 8, SOFT);
        bar(&mut img, x + 28, 176, 150, 34, INK);
        spark(&mut img, x + 28, 226, 224, 36, i, [BLUE, GOOD, AMBER][i as usize]);
    }
    for i in 0..3 {
        boxed(&mut img, 40 + i * 300, 310, 280, 80, 12, CARD, LINE);
        bar(&mut img, 68 + i * 300, 336, 80, 8, SOFT);
        bar(&mut img, 68 + i * 300, 356, 120, 16, INK);
    }
    img
}

fn kpi_single() -> RgbaImage {
    let mut img = canvas();
    boxed(&mut img, 250, 110, 460, 380, 16, CARD, LINE);
    bar(&mut img, 286, 148, 110, 10, SOFT);
    bar(&mut img, 286, 182, 220, 56, INK);
    bar(&mut img, 286, 262, 70, 18, GOOD);
    bar(&mut img, 370, 266, 140, 10, FAINT);
    spark(&mut img, 286, 320, 388, 120, 5, BLUE);
    img
}

fn kpi_compact() -> RgbaImage {
    let mut img = canvas();
    boxed(&mut img, 40, 150, 880, 84, 12, CARD, LINE);
    for i in 0..5 {
        let x = 70 + i * 170;
        bar(&mut img, x, 172, 64, 8, SOFT);
        bar(&mut img, x, 194, 84, 20, INK);
        bar(&mut img, x + 94, 198, 36, 12, if i == 1 { BAD } else { GOOD });
        if i < 4 {
            rrect(&mut img, x + 150, 166, 1, 52, 0, LINE);
        }
    }
    img
}

// ── data table ──

fn table(rows: i32, checks: bool, chips: bool, dense: bool) -> RgbaImage {
    let mut img = canvas();
    boxed(&mut img, 40, 60, 880, 480, 12, CARD, LINE);
    let row_h = if dense { 34 } else { 52 };
    rrect(&mut img, 41, 61, 878, 40, 11, FAINT);
    let cols = [if checks { 90 } else { 64 }, 280, 520, 700];
    for (i, x) in cols.iter().enumerate() {
        bar(&mut img, *x, 77, [90, 70, 60, 50][i], 8, SOFT);
    }
    for r in 0..rows {
        let y = 104 + r * row_h;
        if y + row_h > 536 {
            break;
        }
        rrect(&mut img, 41, y + row_h - 1, 878, 1, 0, LINE);
        if checks {
            boxed(&mut img, 62, y + row_h / 2 - 8, 16, 16, 4, CARD, SOFT);
        }
        bar(&mut img, cols[0], y + row_h / 2 - 5, 110 + (r * 37) % 60, 10, INK);
        bar(&mut img, cols[1], y + row_h / 2 - 4, 90 + (r * 53) % 80, 8, SOFT);
        if chips {
            let c = [GOOD, AMBER, BAD][(r % 3) as usize];
            rrect(&mut img, cols[2], y + row_h / 2 - 11, 78, 22, 11, [c[0], c[1], c[2], 70]);
            rrect(&mut img, cols[2] + 10, y + row_h / 2 - 4, 8, 8, 4, c);
            bar(&mut img, cols[2] + 26, y + row_h / 2 - 3, 36, 6, c);
        } else {
            bar(&mut img, cols[2], y + row_h / 2 - 4, 60 + (r * 29) % 50, 8, SOFT);
        }
        bar(&mut img, cols[3], y + row_h / 2 - 5, 80, 10, INK);
    }
    img
}

// ── filter bar ──

fn filters(chips: i32, search: bool, segmented: bool) -> RgbaImage {
    let mut img = canvas();
    boxed(&mut img, 40, 140, 880, 64, 12, CARD, LINE);
    let mut x = 62;
    if search {
        boxed(&mut img, x, 158, 220, 28, 8, PAGE, LINE);
        rrect(&mut img, x + 12, 166, 12, 12, 6, SOFT);
        bar(&mut img, x + 34, 169, 90, 6, SOFT);
        x += 240;
    }
    if segmented {
        boxed(&mut img, x, 158, 210, 28, 8, FAINT, LINE);
        boxed(&mut img, x + 3, 161, 66, 22, 6, CARD, LINE);
        bar(&mut img, x + 18, 169, 36, 6, INK);
        bar(&mut img, x + 92, 169, 36, 6, SOFT);
        bar(&mut img, x + 150, 169, 36, 6, SOFT);
        x += 230;
    }
    for i in 0..chips {
        let w = 84 + (i * 23) % 30;
        let on = i == 1;
        boxed(&mut img, x, 158, w, 28, 14, if on { [0xE6, 0xEC, 0xFD, 255] } else { CARD }, if on { BLUE } else { LINE });
        bar(&mut img, x + 14, 169, w - 40, 6, if on { BLUE } else { SOFT });
        rrect(&mut img, x + w - 20, 168, 8, 8, 4, if on { BLUE } else { SOFT });
        x += w + 10;
    }
    bar(&mut img, 800, 169, 96, 6, BLUE);
    for r in 0..4 {
        boxed(&mut img, 40, 230 + r * 76, 880, 64, 12, CARD, LINE);
        bar(&mut img, 66, 254 + r * 76, 200 + (r * 41) % 120, 10, INK);
        bar(&mut img, 66, 274 + r * 76, 320, 8, SOFT);
    }
    img
}

// ── empty state ──

fn empty_state(action: bool) -> RgbaImage {
    let mut img = canvas();
    boxed(&mut img, 40, 60, 880, 480, 16, CARD, LINE);
    rrect(&mut img, 432, 160, 96, 96, 48, FAINT);
    rrect(&mut img, 456, 184, 48, 48, 24, SOFT);
    bar(&mut img, 380, 292, 200, 16, INK);
    bar(&mut img, 340, 324, 280, 8, SOFT);
    bar(&mut img, 370, 342, 220, 8, SOFT);
    if action {
        rrect(&mut img, 400, 380, 160, 40, 10, BLUE);
        bar(&mut img, 440, 396, 80, 8, CARD);
    }
    img
}

// ── toast ──

fn toast(kind: Color, undo: bool) -> RgbaImage {
    let mut img = canvas();
    for r in 0..5 {
        bar(&mut img, 60, 70 + r * 40, 300 + (r * 61) % 220, 10, FAINT);
    }
    boxed(&mut img, 560, 470, 340, 72, 12, INK, INK);
    rrect(&mut img, 584, 494, 24, 24, 12, kind);
    bar(&mut img, 624, 496, 130, 8, CARD);
    bar(&mut img, 624, 512, 90, 6, SOFT);
    if undo {
        bar(&mut img, 836, 502, 40, 8, [0x9D, 0xB4, 0xFF, 255]);
    }
    img
}

// ── the folders ──

fn write(dir: &Path, pattern: &str, pictures: &[(&str, RgbaImage)], notes: &str) {
    let folder = dir.join(pattern);
    std::fs::create_dir_all(&folder).expect("a pattern's folder");
    for (name, image) in pictures {
        image.save(folder.join(name)).expect("a picture");
    }
    std::fs::write(folder.join("notes.md"), notes).expect("a pattern's notes");
}

/// The squad's references and, with the web project, its own.
pub fn seed(base: &Path) {
    let squad = base.join("squad/design/references");
    write(
        &squad,
        "kpi-tile",
        &[("northwind-1440.png", kpi_row()), ("harbor-1440.png", kpi_accent()), ("juniper-1280.png", kpi_single()), ("lantern-1440.png", kpi_compact())],
        "# kpi-tile\n\n\
## Northwind — Sales overview\n\
- Source: https://gallery.example.com/patterns/kpi-tile-1 (Pattern Gallery), 2026-10-28, `northwind-1440.png`\n\
- Take: label above value, one accent for the delta, tabular figures; four tiles share one row\n\
- Leave: the drop shadow, the icon on every tile\n\
- Fit: high — same density and one signal colour as ours\n\n\
## Harbor — Team dashboard\n\
- Source: https://docs.example.org/design/dashboards (Design system docs), 2026-10-28, `harbor-1440.png`\n\
- Take: a colour strip at the tile's edge says which metric it is; the sparkline sits under the number\n\
- Leave: three accent colours at once\n\
- Fit: medium — we keep one accent\n\n\
## Juniper — Single metric\n\
- Source: https://gallery.example.com/patterns/kpi-tile-7 (Pattern Gallery), 2026-10-30, `juniper-1280.png`\n\
- Take: when one number matters, give it the whole tile and a long trend\n\
- Leave: the large empty margins\n\
- Fit: medium — for a detail page, not the dashboard\n\n\
## Lantern — Compact strip\n\
- Source: https://notes.example.net/kpi-strips (Interaction notes), 2026-11-02, `lantern-1440.png`\n\
- Take: five metrics in one strip with hairline dividers when the screen is mostly a table\n\
- Leave: nothing to leave\n\
- Fit: high — for the orders screen\n\n\
They agree on one number, one delta and at most one accent colour; the label is small and quiet.\n",
    );
    write(
        &squad,
        "data-table",
        &[("northwind-1440.png", table(9, true, true, false)), ("harbor-1440.png", table(14, false, false, true)), ("juniper-1440.png", table(8, true, false, false))],
        "## Northwind — Orders\n\
- Source: https://gallery.example.com/patterns/table-4 (Pattern Gallery), 2026-10-27, `northwind-1440.png`\n\
- Take: 52px rows, a checkbox column, status as a quiet chip, numbers aligned right\n\
- Leave: zebra stripes\n\
- Fit: high\n\n\
## Harbor — Audit log\n\
- Source: https://docs.example.org/design/tables (Design system docs), 2026-10-27, `harbor-1440.png`\n\
- Take: 34px rows when the person scans hundreds of lines; hairlines only\n\
- Leave: the sticky footer\n\
- Fit: medium — for logs, not for orders\n\n\
## Juniper — Customers\n\
- Source: https://notes.example.net/tables (Interaction notes), 2026-10-31, `juniper-1440.png`\n\
- Take: the header row quieter than the data; bulk actions replace it on selection\n\
- Leave: the avatar column\n\
- Fit: high\n",
    );
    write(&squad, "empty-state", &[("northwind-1280.png", empty_state(true)), ("harbor-1280.png", empty_state(false))], "## Northwind — No orders yet\n- Source: https://gallery.example.com/patterns/empty-2 (Pattern Gallery), 2026-10-29, `northwind-1280.png`\n- Take: one sentence on what goes here, one action\n- Leave: the illustration\n- Fit: high\n");
    write(
        &squad,
        "toast",
        &[("northwind-1280.png", toast(GOOD, true)), ("harbor-1280.png", toast(AMBER, false))],
        "## Northwind — Saved\n- Source: https://gallery.example.com/patterns/toast-3 (Pattern Gallery), 2026-10-29, `northwind-1280.png`\n- Take: bottom right, dark surface, an Undo for five seconds\n- Leave: the progress bar\n- Fit: high\n\n## Harbor — Warning\n- Source: https://docs.example.org/design/feedback (Design system docs), 2026-10-29, `harbor-1280.png`\n- Take: one icon colour carries the kind\n- Leave: the close button\n- Fit: medium\n",
    );
    std::fs::write(
        squad.join("sources.md"),
        "# Sources\n\n\
- Pattern Gallery — https://gallery.example.com — good for: tables, KPI tiles, empty states — fits: dense tools — access: public\n\
- Design system docs — https://docs.example.org/design — good for: states, proportions — fits: dense tools, consumer apps — access: public\n\
- Interaction notes — https://notes.example.net — good for: interaction craft — fits: dense tools — access: public\n",
    )
    .expect("sources");
    // The web project: a filter bar and its own data table.
    let web = base.join("web/design/references");
    write(
        &web,
        "filter-bar",
        &[("northwind-1440.png", filters(4, true, false)), ("harbor-1440.png", filters(5, false, true)), ("juniper-1440.png", filters(3, true, true))],
        "## Northwind — Orders filters\n\
- Source: https://gallery.example.com/patterns/filters-2 (Pattern Gallery), 2026-11-03, `northwind-1440.png`\n\
- Take: search first, then chips that show their value; one text button clears all\n\
- Leave: the saved-views menu\n\
- Fit: high\n\n\
## Harbor — Segmented\n\
- Source: https://docs.example.org/design/filtering (Design system docs), 2026-11-03, `harbor-1440.png`\n\
- Take: a segmented control for the one filter everyone uses\n\
- Leave: five chips in a row\n\
- Fit: medium\n\n\
## Juniper — Search and segments\n\
- Source: https://notes.example.net/filter-bars (Interaction notes), 2026-11-04, `juniper-1440.png`\n\
- Take: the active chip is tinted, never filled\n\
- Leave: nothing\n\
- Fit: high\n\n\
They agree on search first, then filters that read their own value, and one way to clear.\n",
    );
    write(
        &web,
        "data-table",
        &[("northwind-1280.png", table(10, false, true, false)), ("harbor-1280.png", table(11, true, false, true))],
        "## Northwind — Posts\n- Source: https://gallery.example.com/patterns/table-9 (Pattern Gallery), 2026-11-03, `northwind-1280.png`\n- Take: a status chip column and generous rows for a content list\n- Leave: the row hover actions\n- Fit: high\n\n## Harbor — Redirects\n- Source: https://docs.example.org/design/tables (Design system docs), 2026-11-03, `harbor-1280.png`\n- Take: dense rows with a checkbox when an editor works through many\n- Leave: the footer\n- Fit: medium\n",
    );
}

/// The references folders a screen of `project` reads, the squad's after the project's.
fn folders(project: Option<&str>) -> Vec<(PathBuf, bool)> {
    let squad = base().join("squad/design/references");
    match project {
        Some(p) => vec![(base().join(p).join("design/references"), false), (squad, true)],
        None => vec![(squad, false)],
    }
}

/// `read_image` for a picture of a references folder: inside it, links resolved.
pub fn read_image(ask: &equipe_core::protocol::rpc::ReadImage) -> Result<equipe_core::protocol::rpc::ImageRead, String> {
    let roots: Vec<PathBuf> = ["squad", "web"].iter().map(|d| base().join(d).join("design/references")).collect();
    let path = Path::new(&ask.path);
    match roots.iter().find(|r| path.starts_with(r)) {
        Some(root) => equipe_core::visuals::read(path, std::slice::from_ref(root), ask.max),
        None => Err(format!("{} isn't a picture {} shows", ask.path, ask.squad)),
    }
}

/// `design_refs`: read the folders the daemon's way, one page.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    let Command::DesignRefs { project, query, offset, .. } = command else { return false };
    let Found { total, patterns, sources, folder } = refgallery::read_in(folders(project.as_deref()), query, *offset as usize);
    let _ = events.send(Event::DesignRefs { squad: "demo".into(), project: project.clone(), query: query.clone(), offset: *offset, total: total as u32, patterns, sources, folder });
    true
}
