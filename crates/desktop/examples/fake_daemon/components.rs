//! The web project's components for `FAKE_DESIGN=1`, laid out the way a
//! squad keeps them (`equipe_core::design::gallery` reads them): a shadcn
//! registry at the top of the repository, the components' folder, a preview
//! of each in `design/components/` (variants beside it), and two matrix
//! records with their contact sheets in `design/checks/components/`. T-170
//! (in review) adds the Question Step; T-412, still at work, the Source Chip.

use std::path::Path;

/// The ticket still making a component, beside T-170 in review.
pub const WORKING_TICKET: u32 = 412;
pub const WORKING_TITLE: &str = "Search results — source chips";

const INK: &str = "#1B1B19";
const LINE: &str = "#1B1B1924";
const SOFT: &str = "#5B5A5466";
const PAPER: &str = "#FFFFFF";
const ACCENT: &str = "#1D4ED8";

/// The registry's items: name, type, title, description, the page it is shown on.
const ITEMS: [(&str, &str, &str, &str, bool); 6] = [
    ("button", "registry:ui", "Button", "The one action an area asks for: primary, outline and ghost, in three sizes.", true),
    ("evidence-card", "registry:component", "Evidence Card", "A finding with the quote it rests on, its source and how sure we are — what an answer cites.", true),
    ("dialog", "registry:ui", "Dialog", "Asks for one decision over the page; Esc and the backdrop close it.", true),
    ("input", "registry:ui", "Input", "One line of text, with its label, hint and error.", false),
    ("sign-in", "registry:block", "Sign in", "Email first, then a code: the whole page, built from Input and Button.", false),
    ("tooltip", "registry:ui", "Tooltip", "A word on what a control does, on hover and focus.", false),
];

/// Write the repository's side and the design folder's for the web project
/// at `repo`; `extra` adds one of T-170's or T-412's.
pub fn seed(repo: &Path, extra: Option<&str>) {
    let design = repo.join("design");
    let mut items: Vec<serde_json::Value> = ITEMS
        .iter()
        .map(|(name, kind, title, description, docs)| {
            let mut item = serde_json::json!({ "name": name, "type": kind, "title": title, "description": description, "files": [{ "path": format!("registry/ui/{name}.tsx"), "type": kind }], "meta": { "source": format!("https://ui.shadcn.com/docs/components/{name}"), "license": "MIT" } });
            if *docs {
                item["meta"]["docs"] = serde_json::json!(format!("http://localhost:3000/docs/components/{name}"));
            }
            item
        })
        .collect();
    match extra {
        Some("question-step") => items.push(serde_json::json!({ "name": "question-step", "type": "registry:component", "title": "Question Step", "description": "One question of a flow at a time, with where it is in the flow and a way back.", "files": [{ "path": "registry/ui/question-step.tsx", "type": "registry:component" }] })),
        Some("source-chip") => items.push(serde_json::json!({ "name": "source-chip", "type": "registry:component", "title": "Source Chip", "description": "Where a fact comes from, as a small chip: its site's mark and name; opens the page.", "files": [{ "path": "registry/ui/source-chip.tsx", "type": "registry:component" }] })),
        _ => {}
    }
    let registry = serde_json::json!({ "$schema": "https://ui.shadcn.com/schema/registry.json", "name": "web", "homepage": "http://localhost:3000", "items": items });
    std::fs::write(repo.join("registry.json"), serde_json::to_string_pretty(&registry).unwrap()).unwrap();
    // A component in the folder that the registry doesn't list yet.
    std::fs::create_dir_all(repo.join("components/ui")).unwrap();
    std::fs::write(repo.join("components/ui/badge.tsx"), "export function Badge() { return null }\n").unwrap();
    for sub in ["components/button", "checks/components/button", "checks/components/evidence-card"] {
        std::fs::create_dir_all(design.join(sub)).unwrap();
    }
    let files: Vec<(&str, String)> = vec![
        ("components/button.svg", button(ACCENT, PAPER, None)),
        ("components/button/outline.svg", button(PAPER, INK, Some(LINE))),
        ("components/button/ghost.svg", button("none", INK, None)),
        ("components/button/disabled.svg", button("#1B1B1914", "#1B1B1966", None)),
        ("components/evidence-card.svg", evidence_card()),
        ("components/dialog.svg", dialog()),
        ("components/sign-in.svg", sign_in()),
        ("checks/components/button/sheet-1280.svg", sheet(5, 3)),
        ("checks/components/button/sheet-390.svg", sheet(5, 3)),
        ("checks/components/evidence-card/sheet-1280.svg", sheet(1, 2)),
    ];
    for (name, body) in files {
        std::fs::write(design.join(name), body).unwrap();
    }
    let record = |name: &str, fails: usize, sheets: &[&str], examples: &[&str]| {
        let findings: Vec<serde_json::Value> = (0..fails).map(|n| serde_json::json!({ "level": "fail", "check": if n == 0 { "contrast" } else { "key:focus-visible" }, "words": "x" })).collect();
        let report = serde_json::json!({
            "component": name,
            "url": format!("http://localhost:3000/docs/components/{name}"),
            "at": 1_793_000_000u64,
            "widths": [390, 1280],
            "themes": ["light", "dark"],
            "examples": examples,
            "axe": true,
            "findings": findings,
            "sheets": sheets.iter().map(|s| design.join("checks/components").join(name).join(s).to_string_lossy().into_owned()).collect::<Vec<_>>(),
        });
        std::fs::write(design.join(format!("checks/components/{name}.json")), serde_json::to_string_pretty(&report).unwrap()).unwrap();
    };
    record("button", 0, &["sheet-390.svg", "sheet-1280.svg"], &["variant=primary", "variant=outline", "variant=ghost", "size=sm", "size=lg"]);
    record("evidence-card", 2, &["sheet-1280.svg"], &["confidence=high", "confidence=low"]);
    match extra {
        Some("question-step") => std::fs::write(design.join("components/question-step.svg"), question_step()).unwrap(),
        Some("source-chip") => std::fs::write(design.join("components/source-chip.svg"), source_chip()).unwrap(),
        _ => {}
    }
}

fn svg(w: u32, h: u32, body: &str) -> String {
    format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}">{body}</svg>"##)
}

fn bar(x: u32, y: u32, w: u32, h: u32, fill: &str) -> String {
    format!(r##"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{}" fill="{fill}"/>"##, h / 2)
}

fn button(fill: &str, ink: &str, stroke: Option<&str>) -> String {
    let stroke = stroke.map(|s| format!(r##" stroke="{s}" stroke-width="2""##)).unwrap_or_default();
    svg(320, 140, &format!(r##"<rect x="70" y="46" width="180" height="48" rx="10" fill="{fill}"{stroke}/>{}{}"##, bar(110, 66, 72, 8, ink), format_args!(r##"<path d="M200 63 l8 7 -8 7" fill="none" stroke="{ink}" stroke-width="2.5" stroke-linecap="round"/>"##)))
}

fn evidence_card() -> String {
    let body = [
        format!(r##"<rect x="20" y="16" width="280" height="168" rx="14" fill="{PAPER}" stroke="{LINE}" stroke-width="2"/>"##),
        format!(r##"<rect x="40" y="38" width="4" height="58" rx="2" fill="{ACCENT}"/>"##),
        bar(56, 40, 210, 9, INK),
        bar(56, 58, 190, 7, SOFT),
        bar(56, 72, 200, 7, SOFT),
        bar(56, 86, 120, 7, SOFT),
        format!(r##"<rect x="40" y="118" width="108" height="24" rx="12" fill="#1B1B190D" stroke="{LINE}"/><circle cx="54" cy="130" r="6" fill="#E8590C"/>"##),
        bar(66, 126, 70, 7, INK),
        bar(40, 158, 240, 6, "#1B1B1914"),
        bar(40, 158, 176, 6, "#2F9E44"),
    ];
    svg(320, 200, &body.concat())
}

fn source_chip() -> String {
    let body = [
        format!(r##"<rect x="64" y="50" width="192" height="40" rx="20" fill="{PAPER}" stroke="{LINE}" stroke-width="2"/>"##),
        r##"<circle cx="88" cy="70" r="10" fill="#E8590C"/>"##.to_owned(),
        bar(106, 66, 96, 8, INK),
        format!(r##"<path d="M222 64 h10 v10 M232 64 l-12 12" fill="none" stroke="{INK}" stroke-width="2" stroke-linecap="round"/>"##),
    ];
    svg(320, 140, &body.concat())
}

fn question_step() -> String {
    let mut body = vec![bar(30, 24, 260, 6, "#1B1B1914"), bar(30, 24, 104, 6, ACCENT), bar(30, 48, 200, 12, INK)];
    for (n, y) in [80u32, 114, 148].into_iter().enumerate() {
        let on = n == 1;
        body.push(format!(r##"<rect x="30" y="{y}" width="260" height="28" rx="8" fill="{PAPER}" stroke="{}" stroke-width="2"/>"##, if on { ACCENT } else { LINE }));
        body.push(format!(r##"<circle cx="48" cy="{}" r="6" fill="{}" stroke="{}" stroke-width="2"/>"##, y + 14, if on { ACCENT } else { PAPER }, if on { ACCENT } else { SOFT }));
        body.push(bar(64, y + 10, 120 + 30 * n as u32, 8, INK));
    }
    svg(320, 196, &body.concat())
}

fn dialog() -> String {
    let body = [
        r##"<rect width="320" height="200" fill="#1B1B1933"/>"##.to_owned(),
        format!(r##"<rect x="50" y="30" width="220" height="140" rx="14" fill="{PAPER}"/>"##),
        bar(70, 52, 120, 10, INK),
        bar(70, 74, 170, 7, SOFT),
        bar(70, 88, 150, 7, SOFT),
        format!(r##"<rect x="128" y="130" width="60" height="26" rx="7" fill="{PAPER}" stroke="{LINE}" stroke-width="2"/><rect x="196" y="130" width="60" height="26" rx="7" fill="{ACCENT}"/>"##),
    ];
    svg(320, 200, &body.concat())
}

fn sign_in() -> String {
    let body = [
        format!(r##"<rect x="70" y="14" width="180" height="172" rx="14" fill="{PAPER}" stroke="{LINE}" stroke-width="2"/>"##),
        bar(90, 34, 80, 10, INK),
        bar(90, 60, 50, 6, SOFT),
        format!(r##"<rect x="90" y="72" width="140" height="28" rx="8" fill="{PAPER}" stroke="{LINE}" stroke-width="2"/>"##),
        bar(100, 82, 70, 7, SOFT),
        format!(r##"<rect x="90" y="112" width="140" height="30" rx="8" fill="{ACCENT}"/>"##),
        bar(132, 123, 56, 7, PAPER),
        bar(110, 160, 100, 6, SOFT),
    ];
    svg(320, 200, &body.concat())
}

/// A contact sheet: `cols` states across, `rows` examples down, light then dark.
fn sheet(cols: u32, rows: u32) -> String {
    let (cw, ch, pad) = (120u32, 64u32, 12u32);
    let (w, h) = (pad + cols * (cw + pad), pad + rows * 2 * (ch + pad));
    let mut body = vec![format!(r##"<rect width="{w}" height="{h}" fill="#F4F3EF"/>"##)];
    for r in 0..rows * 2 {
        let dark = r >= rows;
        for c in 0..cols {
            let (x, y) = (pad + c * (cw + pad), pad + r * (ch + pad));
            body.push(format!(r##"<rect x="{x}" y="{y}" width="{cw}" height="{ch}" rx="8" fill="{}"/>"##, if dark { "#121211" } else { PAPER }));
            let fill = match (r % rows, c) {
                (_, 4) => "#1B1B1926",
                (0, _) => ACCENT,
                (1, _) => "none",
                _ => if dark { "#FFFFFF1A" } else { "#1B1B190F" },
            };
            let stroke = if c == 2 { format!(r##" stroke="{ACCENT}" stroke-width="3""##) } else if r % rows == 1 { format!(r##" stroke="{}" stroke-width="2""##, if dark { "#FFFFFF33" } else { LINE }) } else { String::new() };
            body.push(format!(r##"<rect x="{}" y="{}" width="80" height="28" rx="7" fill="{fill}"{stroke}/>"##, x + 20, y + 18));
            body.push(bar(x + 38, y + 29, 44, 6, if r % rows == 0 && c != 4 { PAPER } else if dark { "#EFEDE7" } else { INK }));
        }
    }
    svg(w, h, &body.concat())
}
