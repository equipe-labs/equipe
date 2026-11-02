//! Math and diagrams drawn as pictures: LaTeX through latex-rust, Mermaid
//! through mermaid-rs-renderer, each to SVG, rasterized by GPUI's SVG renderer
//! (resvg, at twice the size, so it is sharp on a Retina screen) and kept by
//! their source, so a formula is drawn once however often its row is.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::{App, RenderImage};

/// A picture and the size it is shown at, in logical pixels; `baseline` is
/// how far down its text baseline sits (math set inline).
#[derive(Clone)]
pub struct Picture {
    pub image: Arc<RenderImage>,
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
}

/// Why something could not be drawn; its source is shown instead.
pub type Failed = String;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    InlineMath,
    DisplayMath,
    Mermaid,
}

/// Pictures by what they show: kind, source, size, colour.
type Key = (Kind, String, u32, u32);

const KEEP: usize = 96;

/// What was drawn, and in which order (the oldest goes first).
type Drawn = (HashMap<Key, Result<Picture, Failed>>, Vec<Key>);

thread_local! {
    static DRAWN: RefCell<Drawn> = RefCell::new((HashMap::new(), Vec::new()));
}

/// `source` drawn as `kind`, at `size` px text in `color` (`0xRRGGBB`), from
/// the cache when it was drawn before.
pub fn draw(kind: Kind, source: &str, size: f32, color: u32, dark: bool, cx: &App) -> Result<Picture, Failed> {
    let key: Key = (kind, source.to_owned(), (size * 10.) as u32, color | (dark as u32) << 24);
    if let Some(hit) = DRAWN.with(|d| d.borrow().0.get(&key).cloned()) {
        return hit;
    }
    let drawn = match kind {
        Kind::InlineMath | Kind::DisplayMath => math_svg(source, size, color, kind == Kind::DisplayMath),
        Kind::Mermaid => mermaid_svg(source, dark).map(|svg| (svg, None)),
    }
    .and_then(|(svg, baseline)| rasterize(&svg, baseline, cx));
    DRAWN.with(|d| {
        let (map, order) = &mut *d.borrow_mut();
        if order.len() >= KEEP {
            let old = order.remove(0);
            map.remove(&old);
        }
        order.push(key.clone());
        map.insert(key, drawn.clone());
    });
    drawn
}

/// The formula's SVG, and where its baseline sits as a share of its height.
fn math_svg(source: &str, size: f32, color: u32, display: bool) -> Result<(String, Option<f32>), Failed> {
    use latex_rust::{Color, Dim, MathFont, MathStyle, SvgOptions, layout_with_em_size_pt, parse, render_svg};
    thread_local! {
        static FONT: Option<MathFont> = MathFont::stix_two_math().ok();
    }
    let source = source.trim();
    if source.is_empty() {
        return Err("empty".into());
    }
    FONT.with(|font| {
        let font = font.as_ref().ok_or("no math font")?;
        let ast = parse(source).map_err(|e| e.to_string())?;
        // SVG lengths in points are 4/3 px: an em of `size` px is `size * 3/4` pt.
        let em = Dim::ratio((size * 30.).round() as i64, 40);
        let style = if display { MathStyle::Display } else { MathStyle::Text };
        let tree = layout_with_em_size_pt(&ast, font, style, &em).map_err(|e| e.to_string())?;
        let options = SvgOptions { font_size_pt: em, color: Color::rgb((color >> 16) as u8, (color >> 8) as u8, color as u8), display };
        let svg = render_svg(&tree, font, &options).map_err(|e| e.to_string())?;
        let number = |d: &Dim| d.to_dec_string().parse::<f32>().unwrap_or(0.);
        let (height, depth) = (number(&tree.height), number(&tree.depth));
        let baseline = (height + depth > 0.).then(|| height / (height + depth));
        Ok((svg, baseline))
    })
}

/// The diagram's SVG, in colours that sit on the conversation's background.
fn mermaid_svg(source: &str, dark: bool) -> Result<String, Failed> {
    use mermaid_rs_renderer::{RenderOptions, Theme, render_with_options};
    let mut theme = if dark { Theme::dark() } else { Theme::modern() };
    theme.font_family = "\"Helvetica Neue\", \"Apple SD Gothic Neo\", sans-serif".into();
    theme.font_size = 14.;
    if dark {
        theme.background = "transparent".into();
        theme.primary_color = "#1b1d22".into();
        theme.primary_border_color = "#7c7e85".into();
        theme.primary_text_color = "#efede7".into();
        theme.text_color = "#cfccc4".into();
        theme.line_color = "#9a9ca3".into();
        theme.cluster_background = "#141417".into();
        theme.edge_label_background = "#141417".into();
    } else {
        theme.background = "transparent".into();
    }
    let options = RenderOptions { theme, ..RenderOptions::default() };
    // The renderer panics on some malformed input; a bad diagram is its source.
    let svg = std::panic::catch_unwind(|| render_with_options(source, options)).map_err(|_| "mermaid failed".to_owned())?.map_err(|e| e.to_string())?;
    Ok(if dark { quadrants_in_the_dark(&svg) } else { svg })
}

/// A quadrant chart's own colours are fixed light ones whatever the theme —
/// pale squares, near-black words — so on a dark screen its axis labels go
/// unread: they take the theme's tones instead.
fn quadrants_in_the_dark(svg: &str) -> String {
    const LIGHT_TO_DARK: [(&str, &str); 6] = [
        ("\"#ECECFF\"", "\"#262830\""),
        ("\"#f1f1ff\"", "\"#22242b\""),
        ("\"#f6f6ff\"", "\"#1e2026\""),
        ("\"#fbfbff\"", "\"#1b1c21\""),
        ("\"#c7c7f1\"", "\"#4a4c55\""),
        ("\"#131300\"", "\"#cfccc4\""),
    ];
    LIGHT_TO_DARK.iter().fold(svg.to_owned(), |svg, (light, dark)| svg.replace(light, dark))
}

fn rasterize(svg: &str, baseline: Option<f32>, cx: &App) -> Result<Picture, Failed> {
    // Scale 1: GPUI already rasterizes at twice the size and says so.
    let image = cx.svg_renderer().render_single_frame(svg.as_bytes(), 1.0).map_err(|e| e.to_string())?;
    let px = image.size(0);
    let (width, height) = (px.width.0 as f32 / 2., px.height.0 as f32 / 2.);
    if width < 1. || height < 1. {
        return Err("empty picture".into());
    }
    Ok(Picture { image, width, height, baseline: baseline.map_or(height, |b| b * height) })
}

#[cfg(test)]
mod tests {
    use super::mermaid_svg;

    #[test]
    fn a_quadrant_chart_on_a_dark_screen_is_drawn_in_its_tones_and_read_in_light_words() {
        let source = "quadrantChart\n  title Depth vs system\n  x-axis Low --> High\n  y-axis Shallow --> Deep\n  rea: [0.25, 0.7]\n  ours: [0.85, 0.85]\n";
        let dark = mermaid_svg(source, true).expect("drawn");
        for light in ["#ECECFF", "#f1f1ff", "#f6f6ff", "#fbfbff", "#131300"] {
            assert!(!dark.contains(light), "{light} left in the dark");
        }
        assert!(dark.contains("#cfccc4"), "the axis labels in the theme's words");
        assert!(mermaid_svg(source, false).expect("drawn").contains("#131300"), "a light screen keeps them");
    }
}
