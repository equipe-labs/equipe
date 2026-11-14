//! A row's chips, fitted to the room the row gives them: only whole chips
//! are laid out (never one cut at the edge), the most important first, and
//! what does not fit is told by a `+N` chip whose tooltip lists it.
//!
//! Widths are estimated from the text (the row has no text measurement of
//! its own) a little on the generous side, and each chip is then drawn at
//! exactly its estimated width, so what was counted is what is laid out.

use gpui_kit::component::h_flex;
use gpui_kit::*;

use crate::theme::Palette;
use crate::tips::TipExt as _;

/// A chip's side padding and the gap between chips, in points.
pub const PAD: f32 = 6.;
pub const GAP: f32 = 4.;
/// The chips' type size.
pub const TEXT: f32 = 11.;

/// One chip: its words, colour and what it means on the pointer, and how
/// early it is kept when there is not room for all (lower is kept first).
#[derive(Debug, Clone, PartialEq)]
pub struct Flag {
    pub text: String,
    pub color: Rgba,
    pub tip: Option<String>,
    pub rank: u8,
}

impl Flag {
    pub fn new(text: impl Into<String>, color: Rgba, tip: Option<String>, rank: u8) -> Self {
        Self { text: text.into(), color, tip, rank }
    }
}

/// Ranks: a run that stopped, a ticket held up, a failure; then quiet; then
/// what is only kept or queued; then what it unblocks.
pub const URGENT: u8 = 0;
pub const QUIET: u8 = 1;
pub const NOTE: u8 = 2;
pub const UNBLOCKS: u8 = 3;

/// The width of `text` at `size`, estimated for the system font, rounded up.
pub fn text_width(text: &str, size: f32) -> f32 {
    let em: f32 = text
        .chars()
        .map(|c| match c {
            ' ' | '.' | ',' | ':' | ';' | '\'' | '!' | '|' => 0.30,
            'i' | 'l' | 'j' | 't' | 'f' | 'r' | 'I' | '1' => 0.34,
            'm' | 'w' | 'M' | 'W' => 0.88,
            '→' | '+' | '·' | '−' | '↳' => 0.72,
            c if c.is_ascii_uppercase() => 0.68,
            c if c.is_ascii_digit() => 0.60,
            c if c.is_ascii() => 0.58,
            // Wide scripts, emoji and symbols: about a full em.
            _ => 1.05,
        })
        .sum();
    (em * size * 1.05).ceil() + 1.
}

/// A chip's drawn width: its text and padding.
pub fn chip_width(text: &str) -> f32 {
    text_width(text, TEXT) + 2. * PAD
}

/// What `fit` decided: which chips (indexes into the input, in their own
/// order) are laid out, and which are left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fit {
    pub shown: Vec<usize>,
    pub hidden: Vec<usize>,
}

/// The `+N` chip's words.
pub fn more_text(hidden: usize) -> String {
    format!("+{hidden}")
}

/// Which of `widths` (with their `ranks`) fit in `avail`: all of them when
/// they do; else the most important that fit whole beside a `+N` chip for
/// the rest. The shown ones keep their given order. When not even a `+N`
/// fits beside anything, only a `+N` that fits alone, or nothing, is left.
pub fn fit(widths: &[f32], ranks: &[u8], avail: f32) -> Fit {
    let n = widths.len();
    let total = |idx: &[usize]| idx.iter().map(|i| widths[*i]).sum::<f32>() + GAP * idx.len().saturating_sub(1) as f32;
    let all: Vec<usize> = (0..n).collect();
    if total(&all) <= avail {
        return Fit { shown: all, hidden: Vec::new() };
    }
    // Most important first; equal ranks keep their order.
    let mut order = all;
    order.sort_by_key(|i| (ranks[*i], *i));
    for k in (0..n).rev() {
        let mut keep = order[..k].to_vec();
        keep.sort_unstable();
        let more = chip_width(&more_text(n - k));
        let used = if k == 0 { more } else { total(&keep) + GAP + more };
        if used <= avail {
            let mut hidden = order[k..].to_vec();
            hidden.sort_unstable();
            return Fit { shown: keep, hidden };
        }
    }
    // Not even a lone `+N` fits: nothing is drawn rather than something cut.
    Fit { shown: Vec::new(), hidden: (0..n).collect() }
}

/// The `+N` chip's tooltip: each chip left out on its own line, with what it means.
pub fn more_tip(flags: &[Flag], hidden: &[usize]) -> String {
    hidden
        .iter()
        .map(|i| match &flags[*i].tip {
            Some(tip) => format!("{} — {tip}", flags[*i].text),
            None => flags[*i].text.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The chips that fit `avail` points (indexes into the flags), and the `+N`
/// chip's text and tooltip when any were left out and it fits.
pub struct Laid {
    pub chips: Vec<usize>,
    pub more: Option<(String, String)>,
}

pub fn lay_out(flags: &[Flag], avail: f32) -> Laid {
    let widths: Vec<f32> = flags.iter().map(|f| chip_width(&f.text)).collect();
    let ranks: Vec<u8> = flags.iter().map(|f| f.rank).collect();
    let fit = fit(&widths, &ranks, avail);
    let more = (!fit.hidden.is_empty() && chip_width(&more_text(fit.hidden.len())) <= avail).then(|| (more_text(fit.hidden.len()), more_tip(flags, &fit.hidden)));
    Laid { chips: fit.shown, more }
}

/// The room the chips of a ticket row get: what the row has left after its
/// other columns (`fixed`, the title's least included), up to `cap`.
pub fn room(list_width: f32, fixed: f32, cap: f32) -> f32 {
    (list_width - fixed).clamp(0., cap)
}

/// The chips as a right-aligned row of whole chips and a `+N`. `ids` names
/// the chips' elements (unique per row); a "⛔ T-n" chip opens that ticket's
/// card on hover when `cards` is on, else every chip with words shows them.
pub fn row(p: Palette, flags: &[Flag], avail: f32, ids: SharedString, cards: bool) -> Div {
    let laid = lay_out(flags, avail);
    let chip = |key: SharedString, text: &str, color: Rgba| {
        div().id(key).flex_none().w(px(chip_width(text))).h(px(18.)).flex().items_center().justify_center().overflow_hidden().whitespace_nowrap().rounded(px(4.)).bg(p.wash).text_size(px(TEXT)).text_color(color).child(text.to_owned())
    };
    let mut out = h_flex().flex_none().gap(px(GAP)).justify_end();
    for i in laid.chips {
        let f = &flags[i];
        let key = SharedString::from(format!("{ids}-{i}"));
        let dep = f.text.strip_prefix("⛔ T-").and_then(|n| n.parse().ok()).filter(|_| cards);
        out = out.child(match (dep, &f.tip) {
            (Some(dep), _) => crate::views::ticket_card::hover_link(chip(key, &f.text, f.color), dep, crate::views::ticket_card::Side::Below).into_any_element(),
            (None, Some(tip)) => chip(key, &f.text, f.color).tip(p, tip.clone(), None).into_any_element(),
            (None, None) => chip(key, &f.text, f.color).into_any_element(),
        });
    }
    if let Some((text, tip)) = laid.more {
        out = out.child(chip(SharedString::from(format!("{ids}-more")), &text, p.muted).tip(p, tip, None));
    }
    out
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui_kit's glob brings its own `test` macro over `#[test]`.
    use super::{Flag, GAP, NOTE, QUIET, UNBLOCKS, URGENT, chip_width, fit, lay_out, more_text, more_tip, room, text_width};
    use gpui_kit::Rgba;

    const BLACK: Rgba = Rgba { r: 0., g: 0., b: 0., a: 1. };

    fn flag(text: &str, rank: u8) -> Flag {
        Flag::new(text, BLACK, None, rank)
    }

    #[test]
    fn everything_shows_when_it_fits() {
        let f = fit(&[40., 60., 30.], &[URGENT, QUIET, UNBLOCKS], 40. + 60. + 30. + 2. * GAP);
        assert_eq!((f.shown, f.hidden), (vec![0, 1, 2], vec![]));
    }

    #[test]
    fn the_least_important_go_first_and_a_plus_chip_stands_for_them() {
        // In the row's order: →4, Quiet, Stalled.
        let widths = [30., 70., 50.];
        let ranks = [UNBLOCKS, QUIET, URGENT];
        let room = 70. + 50. + 2. * GAP + chip_width("+1");
        let f = fit(&widths, &ranks, room);
        assert_eq!((f.shown, f.hidden), (vec![1, 2], vec![0]), "→4 goes first");
        let f = fit(&widths, &ranks, room - 1.);
        assert_eq!((f.shown, f.hidden), (vec![2], vec![0, 1]), "then Quiet; Stalled stays beside +2");
        // Room for Stalled alone but not for it and a +N: nothing is cut in, only a +3.
        let f = fit(&widths, &ranks, 50.);
        assert!(f.shown.is_empty(), "{f:?}");
        assert_eq!(f.hidden, vec![0, 1, 2]);
    }

    #[test]
    fn whatever_is_shown_fits_at_every_width() {
        let widths = [30., 70., 50., 44.];
        let ranks = [UNBLOCKS, QUIET, URGENT, NOTE];
        for avail in 0..300 {
            let avail = avail as f32;
            let f = fit(&widths, &ranks, avail);
            assert_eq!(f.shown.len() + f.hidden.len(), widths.len());
            let shown = f.shown.iter().map(|i| widths[*i]).sum::<f32>() + GAP * f.shown.len().saturating_sub(1) as f32;
            let plus = if f.hidden.is_empty() { 0. } else if f.shown.is_empty() { chip_width(&more_text(f.hidden.len())).min(0.) } else { GAP + chip_width(&more_text(f.hidden.len())) };
            assert!(shown + plus <= avail, "{avail}: {f:?}");
            // The most important are never left out for a less important one.
            for h in &f.hidden {
                for s in &f.shown {
                    assert!(ranks[*s] <= ranks[*h], "{avail}: {f:?}");
                }
            }
        }
    }

    #[test]
    fn equal_ranks_keep_their_order() {
        let f = fit(&[40.; 4], &[QUIET; 4], 40. + GAP + 40. + GAP + chip_width("+2"));
        assert_eq!((f.shown, f.hidden), (vec![0, 1], vec![2, 3]));
    }

    #[test]
    fn the_plus_chip_tells_what_it_stands_for() {
        let flags = [flag("→4", UNBLOCKS), Flag::new("Quiet 29m", BLACK, Some("no word for 29m".into()), QUIET), flag("Stalled", URGENT)];
        assert_eq!(more_tip(&flags, &[0, 1]), "→4\nQuiet 29m — no word for 29m");
        // Room for Stalled and +2, not for more.
        let laid = lay_out(&flags, chip_width("Stalled") + GAP + chip_width("+2"));
        assert_eq!(laid.chips, vec![2]);
        assert_eq!(laid.more, Some(("+2".to_owned(), "→4\nQuiet 29m — no word for 29m".to_owned())));
        // All of them: no +N.
        let laid = lay_out(&flags, 1000.);
        assert_eq!((laid.chips, laid.more), (vec![0, 1, 2], None));
    }

    #[test]
    fn a_row_too_narrow_for_even_the_plus_chip_draws_nothing() {
        let laid = lay_out(&[flag("Stalled", URGENT)], 10.);
        assert!(laid.chips.is_empty() && laid.more.is_none());
        assert_eq!(room(100., 300., 160.), 0.);
        assert_eq!(room(1000., 300., 160.), 160.);
    }

    #[test]
    fn widths_are_generous_and_wide_scripts_count_as_wide() {
        let quiet = text_width("Quiet 29m", 11.);
        assert!((45. ..=70.).contains(&quiet), "{quiet}");
        assert!(text_width("\u{AC00}\u{AC00}", 11.) > text_width("ab", 11.));
        assert!(chip_width("⛔ T-12") > chip_width("T-12"));
    }
}
