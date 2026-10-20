//! The designer's kit: `equipe design init` lays out a `design/` folder to
//! fill in (a brand file, layered tokens, guidelines), and `equipe design
//! specimen` draws a system from its tokens alone — palette, type, spacing,
//! radius, shadow, the key components and one real screen, light and dark —
//! so a direction is seen, not described. Three directions' tokens make
//! three specimens: the style tiles to put side by side.

use std::path::{Path, PathBuf};

use super::contrast;
use super::values::{self, Color};
use crate::protocol::design::Token;

const BRAND: &str = include_str!("../../assets/design/brand.md");
const GUIDELINES: &str = include_str!("../../assets/design/guidelines.md");
const TOKENS: &str = include_str!("../../assets/design/tokens.json");
const TASTE: &str = include_str!("../../assets/design/taste.md");

/// Lay out `dir` to fill in: only what isn't there yet. What was written.
pub fn init(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    std::fs::create_dir_all(dir.join("assets").join("logo"))?;
    std::fs::create_dir_all(dir.join("components"))?;
    for (name, body) in [("brand.md", BRAND), ("guidelines.md", GUIDELINES), ("tokens.json", TOKENS), ("taste.md", TASTE)] {
        let path = dir.join(name);
        if !path.exists() {
            std::fs::write(&path, body)?;
            written.push(path);
        }
    }
    Ok(written)
}

/// A token's CSS custom property: `color.text.primary` → `--color-text-primary`.
fn var(name: &str) -> String {
    format!("--{}", name.replace(['.', ' '], "-"))
}

/// A token's value as CSS, in `mode`.
fn css(t: &Token, mode: Option<&str>) -> Option<String> {
    let v = t.value_in(mode);
    match t.kind.as_str() {
        "color" => values::color(v).map(|c| c.hex()),
        "dimension" | "number" => values::dimension(v).map(|n| format!("{n}px")),
        "fontFamily" => values::font_family(v).map(|f| families(&f)),
        "duration" => values::duration_ms(v).map(|ms| format!("{ms}ms")),
        "cubicBezier" => values::cubic_bezier(v).map(|c| format!("cubic-bezier({},{},{},{})", c[0], c[1], c[2], c[3])),
        "shadow" => values::shadows(v).map(|layers| {
            layers.iter().map(|s| format!("{}{}px {}px {}px {}px {}", if s.inset { "inset " } else { "" }, s.x, s.y, s.blur, s.spread, s.color.hex())).collect::<Vec<_>>().join(", ")
        }),
        _ => None,
    }
}

fn families(list: &[String]) -> String {
    list.iter().map(|f| if f.contains(' ') && !f.starts_with('"') { format!("\"{f}\"") } else { f.clone() }).collect::<Vec<_>>().join(", ")
}

fn esc(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The colour named one of `names` exactly (case aside), the earliest
/// first; else one of the system's own colours (under `color.`, not a
/// component's) with one of `words` as a part and none of `not`.
fn role<'a>(tokens: &'a [Token], names: &[&str], words: &[&str], not: &[&str]) -> Option<&'a Token> {
    let colours = || tokens.iter().filter(|t| t.kind == "color");
    names.iter().find_map(|n| colours().find(|t| t.name.eq_ignore_ascii_case(n))).or_else(|| {
        words.iter().find_map(|w| {
            colours().find(|t| {
                let lower = t.name.to_ascii_lowercase();
                let parts: Vec<&str> = lower.split('.').collect();
                parts.first() == Some(&"color") && parts.contains(w) && !parts.iter().any(|p| not.contains(p))
            })
        })
    })
}

fn colour_in(t: Option<&Token>, mode: Option<&str>, fallback: &str) -> String {
    t.and_then(|t| values::color(t.value_in(mode))).map_or_else(|| fallback.to_owned(), |c| c.hex())
}

/// Web fonts the families need, where they are served: Pretendard and SUIT
/// from jsDelivr, the rest from Google Fonts; system families need none.
fn font_links(tokens: &[Token]) -> String {
    let mut names: Vec<String> = Vec::new();
    for t in tokens {
        let list = match t.kind.as_str() {
            "fontFamily" => values::font_family(&t.value).unwrap_or_default(),
            "typography" => values::typography(&t.value).map(|s| s.family).unwrap_or_default(),
            _ => Vec::new(),
        };
        if let Some(first) = list.into_iter().next()
            && !names.contains(&first)
        {
            names.push(first);
        }
    }
    let system = ["system-ui", "-apple-system", "sans-serif", "serif", "monospace", "apple sd gothic neo", "sf pro", "helvetica", "arial", "menlo"];
    let mut links = String::new();
    for name in names {
        let lower = name.to_ascii_lowercase();
        if system.iter().any(|s| lower.starts_with(s)) {
            continue;
        }
        let href = if lower.starts_with("pretendard") {
            "https://cdn.jsdelivr.net/gh/orioncactus/pretendard@v1.3.9/dist/web/variable/pretendardvariable-dynamic-subset.min.css".to_owned()
        } else if lower == "suit" || lower.starts_with("suit ") {
            "https://cdn.jsdelivr.net/gh/sun-typeface/SUIT@2/fonts/variable/woff2/SUIT-Variable.css".to_owned()
        } else {
            format!("https://fonts.googleapis.com/css2?family={}:wght@300;400;500;600;700;800&display=swap", name.replace(' ', "+"))
        };
        links.push_str(&format!("<link rel=\"stylesheet\" href=\"{href}\">\n"));
    }
    links
}

/// The system drawn: `title` over it, light and dark side by side when it has dark values.
pub fn specimen(tokens: &[Token], modes: &[String], title: &str) -> String {
    let dark = modes.iter().find(|m| m.to_ascii_lowercase().contains("dark")).map(String::as_str);
    let mut vars = String::new();
    let mut dark_vars = String::new();
    for t in tokens {
        if let Some(v) = css(t, None) {
            vars.push_str(&format!("{}:{v};", var(&t.name)));
        }
        if let Some(mode) = dark
            && t.modes.contains_key(mode)
            && let Some(v) = css(t, Some(mode))
        {
            dark_vars.push_str(&format!("{}:{v};", var(&t.name)));
        }
    }
    // The roles the sample screens are drawn with, found by name.
    let body_family = tokens
        .iter()
        .find_map(|t| match t.kind.as_str() {
            "fontFamily" if t.name.to_ascii_lowercase().contains("body") || t.name.to_ascii_lowercase().contains("sans") => values::font_family(&t.value),
            "typography" if t.name.to_ascii_lowercase().contains("body") => values::typography(&t.value).map(|s| s.family).filter(|f| !f.is_empty()),
            _ => None,
        })
        .or_else(|| tokens.iter().find_map(|t| (t.kind == "fontFamily").then(|| values::font_family(&t.value)).flatten()))
        .map(|f| families(&f))
        .unwrap_or_else(|| "system-ui, sans-serif".into());
    let pick = |mode: Option<&str>| {
        let bg = contrast::background(tokens, mode, if mode.is_some() { Color::rgb(0x0B0B0D) } else { Color::WHITE }).hex();
        let text = contrast::text(tokens, mode, if mode.is_some() { Color::rgb(0xF2F2F2) } else { Color::rgb(0x141414) }).hex();
        let text_named = role(tokens, &["color.text.primary", "color.text.default", "color.text", "color.fg.default", "color.fg", "color.foreground", "color.content.primary"], &[], &[]);
        let text = colour_in(text_named, mode, &text);
        let surface = colour_in(role(tokens, &["color.surface.default", "color.surface", "color.surface.raised", "color.card", "color.bg.surface", "color.background.surface"], &["surface", "card", "raised", "panel"], &["text", "on"]), mode, &bg);
        let muted = colour_in(role(tokens, &["color.text.secondary", "color.text.muted", "color.text.subtle", "color.fg.muted", "color.foreground.muted", "color.content.secondary"], &[], &[]), mode, &text);
        let border = colour_in(role(tokens, &["color.border.default", "color.border", "color.line", "color.divider", "color.outline"], &["border", "divider", "outline"], &[]), mode, "#8888884D");
        let primary = colour_in(role(tokens, &["color.action.primary", "color.primary", "color.accent", "color.brand.primary", "color.interactive", "color.action.default", "color.brand"], &["action", "accent", "primary", "brand"], &["text", "on", "onprimary", "bg", "background", "surface", "border"]), mode, &text);
        let on_primary = colour_in(role(tokens, &["color.action.onPrimary", "color.onPrimary", "color.on-primary", "color.action.on-primary", "color.text.onPrimary", "color.text.inverse", "color.onAccent"], &["onprimary", "inverse"], &[]), mode, "#FFFFFF");
        let danger = colour_in(role(tokens, &["color.danger", "color.error", "color.status.danger", "color.feedback.error", "color.text.danger"], &["danger", "error", "negative", "critical"], &[]), mode, "#D92D20");
        let success = colour_in(role(tokens, &["color.success", "color.status.success", "color.feedback.success", "color.text.success"], &["success", "positive"], &[]), mode, "#12A150");
        let radius = tokens.iter().find(|t| t.name.to_ascii_lowercase().contains("radius") && (t.name.contains("md") || t.name.contains("medium") || t.name.contains("default"))).or_else(|| tokens.iter().find(|t| t.name.to_ascii_lowercase().contains("radius"))).and_then(|t| values::dimension(&t.value)).unwrap_or(8.);
        let shadow = tokens.iter().find(|t| t.kind == "shadow").and_then(|t| css(t, mode)).unwrap_or_else(|| "0 1px 2px #0000001A".into());
        format!(
            "--s-bg:{bg};--s-text:{text};--s-surface:{surface};--s-muted:{muted};--s-border:{border};--s-primary:{primary};--s-on-primary:{on_primary};--s-danger:{danger};--s-success:{success};--s-radius:{radius}px;--s-shadow:{shadow};"
        )
    };
    let light_roles = pick(None);
    let dark_roles = dark.map(|m| pick(Some(m)));

    // The palette: each colour, its dark value, its contrast on the background.
    let mut palette = String::new();
    let mut group = String::new();
    for t in tokens.iter().filter(|t| t.kind == "color") {
        if t.group() != group {
            group = t.group().to_owned();
            palette.push_str(&format!("<h4>{}</h4>", esc(&group)));
        }
        let light = colour_in(Some(t), None, "#000");
        let dark_hex = dark.filter(|m| t.modes.contains_key(*m)).map(|m| colour_in(Some(t), Some(m), "#000"));
        let ratio = contrast::measure(tokens, t, None, false).map(|r| format!("{r:.1}:1")).unwrap_or_default();
        palette.push_str(&format!(
            "<div class=sw><div class=chip style=\"background:{light}\">{}</div><div class=meta><b>{}</b><span>{light}{} · {ratio}</span></div></div>",
            dark_hex.as_ref().map(|d| format!("<i style=\"background:{d}\"></i>")).unwrap_or_default(),
            esc(t.leaf()),
            dark_hex.map(|d| format!(" / {d}")).unwrap_or_default(),
        ));
    }
    // Type: each style with Korean and Latin, its numbers.
    let mut type_rows = String::new();
    for t in tokens.iter().filter(|t| t.kind == "typography") {
        let Some(s) = values::typography(&t.value) else { continue };
        let style = format!(
            "font-family:{};{}{}{}{}",
            if s.family.is_empty() { body_family.clone() } else { families(&s.family) },
            s.size.map(|v| format!("font-size:{v}px;")).unwrap_or_default(),
            s.weight.map(|v| format!("font-weight:{v};")).unwrap_or_default(),
            s.line.map(|v| format!("line-height:{v}px;")).unwrap_or_default(),
            s.tracking.map(|v| format!("letter-spacing:{v}px;")).unwrap_or_default()
        );
        type_rows.push_str(&format!(
            "<div class=ty><div class=tyname>{}<span>{}{}{}</span></div><div style=\"{style}\">브랜드를 말하는 글자 · The quick brown fox 0123</div></div>",
            esc(&t.name),
            s.size.map(|v| format!("{v}px")).unwrap_or_default(),
            s.weight.map(|v| format!(" / {v}")).unwrap_or_default(),
            s.line.map(|v| format!(" / {v}px")).unwrap_or_default()
        ));
    }
    if type_rows.is_empty() {
        for t in tokens.iter().filter(|t| t.kind == "fontFamily") {
            let family = css(t, None).unwrap_or_default();
            type_rows.push_str(&format!("<div class=ty><div class=tyname>{}</div><div style=\"font-family:{family};font-size:28px\">브랜드를 말하는 글자 Aa 0123</div></div>", esc(&t.name)));
        }
    }
    // Spacing and radius, as bars and boxes.
    let mut spaces = String::new();
    let mut radii = String::new();
    for t in tokens.iter().filter(|t| matches!(t.kind.as_str(), "dimension" | "number")) {
        let Some(n) = values::dimension(&t.value) else { continue };
        let lower = t.name.to_ascii_lowercase();
        if lower.contains("radius") || lower.contains("rounded") || lower.contains("corner") {
            radii.push_str(&format!("<div class=rad><div style=\"border-radius:{n}px\"></div><span>{} · {n}</span></div>", esc(t.leaf())));
        } else if lower.contains("space") || lower.contains("spacing") || lower.contains("gap") || lower.contains("size") {
            spaces.push_str(&format!("<div class=sp><span>{} · {n}</span><div style=\"width:{}px\"></div></div>", esc(t.leaf()), n.min(480.)));
        }
    }
    let mut shadows = String::new();
    for t in tokens.iter().filter(|t| t.kind == "shadow") {
        if let Some(v) = css(t, None) {
            shadows.push_str(&format!("<div class=sh style=\"box-shadow:{v}\">{}</div>", esc(t.leaf())));
        }
    }
    let screen = |roles: &str, label: &str| {
        format!(
            r#"<div class="screen" style="{roles}">
  <div class="label">{label}</div>
  <div class="bar"><b>{title}</b><span class="spacer"></span><button class="btn ghost">도움말</button><button class="btn primary">새로 만들기</button></div>
  <div class="kpis">
    <div class="card"><span>이번 달 거래</span><b>1,284<small>건</small></b><em class="up">▲ 12.4%</em></div>
    <div class="card"><span>운용 자산</span><b>600,000<small>백만원</small></b><em>지난달과 같음</em></div>
    <div class="card"><span>검토 대기</span><b>7<small>건</small></b><em class="down">▼ 2건</em></div>
  </div>
  <div class="card table">
    <div class="tr th"><span>이름</span><span>상태</span><span class="num">금액</span></div>
    <div class="tr"><span>시리즈 A 투자</span><span class="badge ok">완료</span><span class="num">12,500</span></div>
    <div class="tr"><span>브리지 라운드</span><span class="badge">검토 중</span><span class="num">3,200</span></div>
    <div class="tr"><span>엔젤 매칭</span><span class="badge bad">반려</span><span class="num">800</span></div>
  </div>
  <div class="row">
    <label class="field"><span>회사 이름</span><input value="에이티넘"></label>
    <label class="field err"><span>사업자 번호</span><input value="123-45"><em>숫자 10자리를 입력해 주세요</em></label>
  </div>
  <div class="row"><button class="btn primary">저장하기</button><button class="btn secondary">취소</button><button class="btn primary" disabled>보내는 중…</button><button class="btn danger">삭제</button></div>
  <div class="empty">아직 기록이 없어요. 첫 투자를 추가해 보세요.</div>
</div>"#,
            title = esc(title)
        )
    };
    let screens = match &dark_roles {
        Some(dark_roles) => format!("<div class=pair>{}{}</div>", screen(&light_roles, "Light"), screen(&format!("{dark_vars}{dark_roles}"), "Dark")),
        None => format!("<div class=pair>{}</div>", screen(&light_roles, "Light")),
    };
    format!(
        r#"<!doctype html><html lang="ko"><head><meta charset="utf-8"><title>{title} — specimen</title>
{fonts}<style>
:root{{{vars}}}
*{{box-sizing:border-box}} body{{margin:0;padding:40px;background:#F4F4F5;color:#18181B;font-family:{body_family};-webkit-font-smoothing:antialiased}}
h1{{font-size:28px;margin:0 0 4px}} h2{{font-size:13px;letter-spacing:.08em;text-transform:uppercase;color:#71717A;margin:40px 0 14px}} h4{{width:100%;margin:14px 0 6px;font-size:12px;color:#71717A}}
.sub{{color:#71717A;margin:0 0 24px}}
.grid{{display:flex;flex-wrap:wrap;gap:10px}}
.sw{{width:150px;background:#fff;border-radius:10px;overflow:hidden;border:1px solid #E4E4E7}} .chip{{height:64px;position:relative}} .chip i{{position:absolute;right:0;top:0;bottom:0;width:34%}}
.meta{{padding:8px 10px;font-size:12px}} .meta b{{display:block}} .meta span{{color:#71717A;font-variant-numeric:tabular-nums}}
.ty{{display:grid;grid-template-columns:200px 1fr;gap:16px;align-items:baseline;padding:12px 0;border-bottom:1px solid #E4E4E7}} .tyname{{font-size:12px;color:#52525B}} .tyname span{{display:block;color:#A1A1AA}}
.sp{{display:flex;align-items:center;gap:12px;font-size:12px;color:#52525B;margin:4px 0}} .sp span{{width:120px}} .sp div{{height:12px;background:#6B7280;border-radius:2px}}
.rad{{display:inline-flex;flex-direction:column;align-items:center;gap:6px;margin:0 14px 10px 0;font-size:12px;color:#52525B}} .rad div{{width:64px;height:64px;border:2px solid #6B7280;background:#fff}}
.sh{{display:inline-flex;align-items:center;justify-content:center;width:140px;height:80px;margin:0 18px 18px 0;background:#fff;border-radius:10px;font-size:12px;color:#52525B}}
.pair{{display:grid;grid-template-columns:repeat(auto-fit,minmax(520px,1fr));gap:20px}}
.screen{{background:var(--s-bg);color:var(--s-text);border-radius:16px;padding:20px;display:flex;flex-direction:column;gap:14px;border:1px solid #0000000F}}
.screen .label{{font-size:11px;color:var(--s-muted)}}
.bar{{display:flex;align-items:center;gap:8px}} .bar b{{font-size:18px}} .spacer{{flex:1}}
.kpis{{display:grid;grid-template-columns:repeat(3,1fr);gap:10px}}
.card{{background:var(--s-surface);border:1px solid var(--s-border);border-radius:var(--s-radius);box-shadow:var(--s-shadow);padding:14px}}
.kpis .card span{{font-size:12px;color:var(--s-muted)}} .kpis .card b{{display:block;font-size:24px;margin:4px 0;font-variant-numeric:tabular-nums}} .kpis small{{font-size:12px;font-weight:400;margin-left:2px;color:var(--s-muted)}} .kpis em{{font-style:normal;font-size:12px;color:var(--s-muted)}} .up{{color:var(--s-success)!important}} .down{{color:var(--s-danger)!important}}
.table{{padding:4px 14px}} .tr{{display:grid;grid-template-columns:1fr 90px 90px;padding:10px 0;border-bottom:1px solid var(--s-border);font-size:14px;align-items:center}} .tr:last-child{{border:0}} .th{{font-size:12px;color:var(--s-muted)}} .num{{text-align:right;font-variant-numeric:tabular-nums}}
.badge{{font-size:12px;padding:2px 8px;border-radius:999px;border:1px solid var(--s-border);justify-self:start}} .badge.ok{{color:var(--s-success)}} .badge.bad{{color:var(--s-danger)}}
.row{{display:flex;gap:10px;flex-wrap:wrap;align-items:flex-start}}
.field{{display:flex;flex-direction:column;gap:6px;font-size:12px;color:var(--s-muted);flex:1;min-width:180px}} .field input{{font:inherit;font-size:14px;color:var(--s-text);background:var(--s-surface);border:1px solid var(--s-border);border-radius:var(--s-radius);padding:9px 11px}} .field.err input{{border-color:var(--s-danger)}} .field em{{font-style:normal;color:var(--s-danger)}}
.btn{{font:inherit;font-size:14px;font-weight:600;padding:9px 14px;border-radius:var(--s-radius);border:1px solid transparent;cursor:pointer}} .btn.primary{{background:var(--s-primary);color:var(--s-on-primary)}} .btn.secondary{{background:transparent;color:var(--s-text);border-color:var(--s-border)}} .btn.ghost{{background:transparent;color:var(--s-muted)}} .btn.danger{{background:transparent;color:var(--s-danger);border-color:var(--s-danger)}} .btn:disabled{{opacity:.45}}
.empty{{font-size:14px;color:var(--s-muted);padding:18px;border:1px dashed var(--s-border);border-radius:var(--s-radius);text-align:center}}
</style></head><body>
<h1>{title}</h1><p class="sub">{count} tokens · drawn from tokens.json by <code>equipe design specimen</code></p>
<h2>Screens</h2>{screens}
<h2>Colour</h2><div class="grid">{palette}</div>
<h2>Type</h2>{type_rows}
<h2>Spacing</h2>{spaces}
<h2>Radius</h2>{radii}
<h2>Shadow</h2>{shadows}
</body></html>"#,
        title = esc(title),
        fonts = font_links(tokens),
        count = tokens.len(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_lays_out_a_folder_to_fill_in_and_keeps_what_is_there() {
        let d = std::env::temp_dir().join(format!("equipe-kit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("brand.md"), "mine").unwrap();
        let written = init(&d).unwrap();
        let names: Vec<String> = written.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["guidelines.md", "tokens.json", "taste.md"], "brand.md was there already");
        assert_eq!(std::fs::read_to_string(d.join("brand.md")).unwrap(), "mine");
        assert!(d.join("assets/logo").is_dir() && d.join("components").is_dir());
        // The tokens it starts with read cleanly, in layers, and pass the check's reading.
        let parsed = super::super::tokens::parse(&std::fs::read_to_string(d.join("tokens.json")).unwrap());
        assert!(parsed.problems.is_empty(), "{:?}", parsed.problems);
        assert!(parsed.tokens.iter().any(|t| t.name.starts_with("color.text.") && t.alias.is_some()), "roles alias the palette");
        assert!(parsed.modes.iter().any(|m| m == "dark"));
        assert!(BRAND.contains("해요체") && BRAND.contains("this, not that"), "the brand file asks for the voice");
    }

    #[test]
    fn a_specimen_draws_the_system_light_and_dark_from_its_tokens() {
        let parsed = super::super::tokens::parse(TOKENS);
        let html = specimen(&parsed.tokens, &parsed.modes, "Atinum <draft>");
        assert!(html.contains("<h1>Atinum &lt;draft&gt;</h1>"), "the title escaped");
        assert!(html.contains("--color-text-primary:"), "every token a CSS variable");
        assert!(html.contains(">Dark</div>"), "dark beside light");
        assert!(html.contains("pretendard"), "Pretendard's web font linked");
        assert!(html.contains("브랜드를 말하는 글자"), "type set in Korean");
        // The page's roles, not a component's: the button's white text isn't the page's text.
        assert!(html.contains("--s-text:#16161A;"), "{}", &html[html.find("--s-bg").unwrap()..][..400]);
        assert!(html.contains("--s-primary:#1F6FEB;") && html.contains("--s-on-primary:#FFFFFF;"));
        // The kit's own tokens pass the check's contrast.
        let d = std::env::temp_dir().join(format!("equipe-kit-check-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        init(&d).unwrap();
        let findings = super::super::check::check(&d);
        assert!(!findings.iter().any(|f| f.level == super::super::check::Level::Error), "{findings:#?}");
    }
}
