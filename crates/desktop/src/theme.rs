//! The spec's colour tokens (T-54 §2.1) and the type sizes, light and dark.
//! Dark and light follow the system appearance, unless the person picked one
//! in settings.

use gpui_kit::*;

/// Light or dark, as the person picked in settings (kept with the window's
/// other choices, in `target.rs`); the Mac's by default.
pub use crate::target::Appearance;

impl Appearance {
    /// What the window draws in, given what the Mac says.
    pub fn resolve(self, system: WindowAppearance) -> WindowAppearance {
        match self {
            Appearance::System => system,
            Appearance::Light => WindowAppearance::Light,
            Appearance::Dark => WindowAppearance::Dark,
        }
    }

    /// Make the app itself light or dark — the traffic lights, menus and
    /// Sparkle's windows too — or hand it back to the Mac (`System`).
    pub fn apply(self) {
        #[cfg(target_os = "macos")]
        {
            use objc2::msg_send;
            use objc2::runtime::{AnyClass, AnyObject};
            use objc2_foundation::NSString;
            let (Some(app_class), Some(appearance_class)) = (AnyClass::get(c"NSApplication"), AnyClass::get(c"NSAppearance")) else { return };
            // SAFETY: AppKit's `+[NSApplication sharedApplication]`,
            // `+[NSAppearance appearanceNamed:]` and `-[NSApplication setAppearance:]`
            // (nil follows the system), on the main thread, where the app runs.
            unsafe {
                let app: *mut AnyObject = msg_send![app_class, sharedApplication];
                if app.is_null() {
                    return;
                }
                let look: *mut AnyObject = match self {
                    Appearance::System => std::ptr::null_mut(),
                    Appearance::Light => msg_send![appearance_class, appearanceNamed: &*NSString::from_str("NSAppearanceNameAqua")],
                    Appearance::Dark => msg_send![appearance_class, appearanceNamed: &*NSString::from_str("NSAppearanceNameDarkAqua")],
                };
                let _: () = msg_send![app, setAppearance: look];
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Rgba,
    pub side: Rgba,
    /// A sidebar row under the pointer, and the one open: solid, so a title
    /// can fade into them.
    pub side_hover: Rgba,
    pub side_sel: Rgba,
    pub wash: Rgba,
    pub text: Rgba,
    /// Reading text — replies, your messages: a shade softer than `text`,
    /// which stays for titles and controls. Pure white on near-black glares
    /// and makes long Korean text look heavy.
    pub prose: Rgba,
    pub text2: Rgba,
    pub muted: Rgba,
    pub hair: Rgba,
    pub accent: Rgba,
    pub accent_bg: Rgba,
    pub code_bg: Rgba,
    pub raised: Rgba,
    /// The main action's fill (a button, the composer's send): in the dark a
    /// deep slate of the accent's hue, a step above the cards, with the text
    /// colour on it — emphasis that belongs to the window rather than a white
    /// slab; in the light, the text colour, inverted.
    pub primary: Rgba,
    pub on_primary: Rgba,
    /// Words or a mark on an `accent` fill (a count badge, a tick).
    pub on_accent: Rgba,
    /// Work under way.
    pub amber: Rgba,
    /// Blocked: waits for other work.
    pub orange: Rgba,
    /// Something went wrong.
    pub red: Rgba,
    members: [Rgba; 8],
}

impl Palette {
    /// Give the kit's text fields this palette's caret and selection, and its
    /// light or dark: left alone they drew the kit's default caret, which on
    /// our dark fields hardly showed.
    pub fn apply_to_kit(&self, appearance: WindowAppearance, cx: &mut gpui_kit::App) {
        use gpui_kit::component::{Theme, ThemeMode};
        let dark = matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark);
        let caret: gpui_kit::Hsla = self.text.into();
        let mut selection: gpui_kit::Hsla = self.accent.into();
        selection.a = if dark { 0.30 } else { 0.22 };
        Theme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx);
        Theme::update(cx, |theme| {
            theme.colors.caret = caret;
            theme.colors.selection = selection;
            theme.mono_font_family = MONO.into();
        });
    }

    pub fn new(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
        }
    }

    fn dark() -> Self {
        Self {
            // One warm neutral (the brand's Bone) throughout. The reading
            // surface sits a step off black — the brand's Void glares under
            // long text and leaves no room for layers — and the sidebar a
            // step behind it.
            bg: rgb(0x151513),
            side: rgb(0x100F0E),
            side_hover: rgb(0x1C1C1A),
            side_sel: rgb(0x272624),
            wash: rgba(0xFFFFFF12),
            text: rgb(0xF3F0E9),
            prose: rgb(0xE5E2DB),
            text2: rgb(0xCCC8C2),
            // At least 4.5:1 (WCAG AA, small text) on every surface it sits
            // on, the selected sidebar row and the popover included.
            muted: rgb(0xA2A09A),
            hair: rgba(0xFFFFFF1A),
            // The one colour, the cool rim: deep enough to stand apart from
            // the words, not a pastel that reads as faded text.
            accent: rgb(0x7EB8F3),
            accent_bg: rgba(0x7EB8F31F),
            code_bg: rgb(0x1B1A19),
            raised: rgb(0x1F1E1C),
            primary: rgb(0x2B3F54),
            on_primary: rgb(0xF3F0E9),
            on_accent: rgb(0x151513),
            amber: rgb(0xEABA67),
            orange: rgb(0xF19B65),
            red: rgb(0xF2857D),
            // One lightness and chroma (OKLCH 0.80, 0.11), so no member is louder.
            members: [0xE3B667, 0xF4A0C8, 0x7FC5FF, 0x89D298, 0xC3B0FD, 0x5AD5CB, 0xF7A97C, 0xBBC671].map(rgb),
        }
    }

    fn light() -> Self {
        Self {
            bg: rgb(0xFAF9F6),
            side: rgb(0xF3F2EF),
            side_hover: rgb(0xEEEDE8),
            side_sel: rgb(0xE7E6E1),
            wash: rgba(0x0000000C),
            text: rgb(0x181713),
            prose: rgb(0x24231F),
            text2: rgb(0x45443E),
            muted: rgb(0x62605A),
            hair: rgba(0x0000001A),
            accent: rgb(0x2160AB),
            accent_bg: rgba(0x2160AB14),
            code_bg: rgb(0xF2F1ED),
            raised: rgb(0xFFFFFF),
            primary: rgb(0x181713),
            on_primary: rgb(0xFAF9F6),
            on_accent: rgb(0xFAF9F6),
            // Darker than the dark appearance's, to be read as words (a
            // status in the sidebar) at 4.5:1 on any surface.
            amber: rgb(0x865901),
            orange: rgb(0x9E4507),
            red: rgb(0xAA2A27),
            // OKLCH 0.49, 0.12: 4.5:1 even on the selected sidebar row.
            members: [0x835600, 0x943C6B, 0x0065A4, 0x0D7435, 0x674EA1, 0x006F68, 0x984500, 0x5F6700].map(rgb),
        }
    }

    pub fn member(&self, hue: usize) -> Rgba {
        self.members[hue % 8]
    }
}

/// A member's mark and colour index, the same as the terminal console's.
pub fn look(role: &str, name: &str) -> (&'static str, usize) {
    match role {
        "pm" => ("◆", 0),
        "designer" => ("✦", 1),
        "frontend" => ("●", 2),
        "backend" => ("■", 3),
        "qa" => ("◎", 4),
        "data" => ("▲", 5),
        "mobile" => ("◈", 6),
        _ => {
            const MARKS: [&str; 4] = ["◇", "★", "▼", "✚"];
            let h = name.bytes().fold(7usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize));
            (MARKS[h % MARKS.len()], 6 + h % 2)
        }
    }
}

/// The Mac's own interface face (SF): drawn for the screen at every size, in
/// every weight. Helvetica Neue, the brand's, stays with the brand's moments.
pub const SANS: &str = ".SystemUIFont";

/// The Mac's faces for what SF doesn't draw — Korean, Japanese, Chinese —
/// the app's language's first: Han characters are shared, and each language
/// wants its own shapes for them. All come with every Mac, in every weight.
pub fn fallbacks(lang: equipe_core::i18n::Lang) -> FontFallbacks {
    use equipe_core::i18n::Lang;
    static BY_LANG: std::sync::OnceLock<Vec<(Lang, FontFallbacks)>> = std::sync::OnceLock::new();
    const KO: &str = "Apple SD Gothic Neo";
    const JA: &str = "Hiragino Sans";
    const SC: &str = "PingFang SC";
    const TC: &str = "PingFang TC";
    let all = BY_LANG.get_or_init(|| {
        Lang::ALL
            .iter()
            .map(|&lang| {
                let first = match lang {
                    Lang::Ja => JA,
                    Lang::ZhHans => SC,
                    Lang::ZhHant => TC,
                    _ => KO,
                };
                let faces = std::iter::once(first).chain([KO, JA, SC, TC].into_iter().filter(|f| *f != first)).map(str::to_owned).collect();
                (lang, FontFallbacks::from_fonts(faces))
            })
            .collect()
    });
    all.iter().find(|(l, _)| *l == lang).map_or_else(|| all[0].1.clone(), |(_, f)| f.clone())
}

/// The interface's face, and the faces for the scripts it lacks.
pub trait Sans: Styled + Sized {
    fn sans(mut self) -> Self {
        let style = self.text_style();
        style.font_family = Some(SANS.into());
        style.font_fallbacks = Some(fallbacks(crate::i18n::lang()));
        self
    }
}

impl<E: Styled> Sans for E {}

// ── type ──
//
// Three sizes carry the app: reading text 15, the interface 14, secondary 13;
// meta (times, counts, "thought for 11s") 12. Korean is never below 12, and
// wants more leading than Latin: reading text is 15 on 25 (1.65).

/// Reading text: replies, your messages, the composer.
pub const BODY: f32 = 15.;
pub const BODY_LINE: f32 = 25.;
/// The interface: rows, buttons, menus.
pub const UI: f32 = 14.;
/// Text at the interface size read as text (a ticket's description, an ask).
pub const UI_PROSE_LINE: f32 = 23.;
/// Secondary text: notes, tool rows, thoughts, hints.
pub const SMALL: f32 = 13.;
pub const SMALL_LINE: f32 = 20.;
/// Meta: how long, when, how many.
pub const META: f32 = 12.;
pub const META_LINE: f32 = 18.;

/// The conversation's column, the composer's with it.
pub const COLUMN: f32 = 720.;
/// The measure: a reply's text is no wider than this — about 40 Korean
/// characters, or 75 Latin, at 15. Cards, the composer and the column are wider.
pub const MEASURE: f32 = 600.;
/// Between paragraphs: 0.7 of the body size.
pub const PARAGRAPH_GAP: f32 = 11.;
/// Between a list's items: tighter than paragraphs, so a list reads as one.
pub const LIST_GAP: f32 = 4.;
/// A heading's room: more above (beyond the paragraph gap) than below, so it
/// belongs to what follows.
pub const HEADING_ABOVE: f32 = 8.;
pub const HEADING_BELOW: f32 = 4.;
/// A link's underline: a point, in the link's colour held well back, so it
/// marks the link without striking through the line.
pub const LINK_UNDERLINE: f32 = 1.;
pub const LINK_UNDERLINE_ALPHA: f32 = 0.35;
/// The room under the conversation's last line, over the composer.
pub const TAIL_ROOM: f32 = 20.;

/// Project chips take the project's order and these colours (T-54 §2.1) —
/// blue, violet, teal, pink, green: never the amber of work under way or the
/// orange of what waits on the person, which a project's dot would be read as.
pub const PROJECT_HUES: [usize; 5] = [2, 4, 5, 1, 3];
pub const MONO: &str = ".AppleSystemUIFontMonospaced";

// ── one scale for everything ──
//
// Space: 4 · 8 · 12 · 16 · 24 (GPUI's `_1` `_2` `_3` `_4` `_6`; S3 and S6 named where a
// padding has to be computed). Radii: 6 for chips and small buttons, 8 for
// controls, menus and rows, 12 for cards, 16 for the composer and dialogs.
// Controls are 28 tall with 16-point icons; a text field is 36; a menu or list
// row is 32. Motion is short and eases out (`crate::motion`).

pub const S3: f32 = 12.;
pub const S6: f32 = 24.;

pub const R_SM: f32 = 6.;
pub const R_MD: f32 = 8.;
pub const R_LG: f32 = 12.;
pub const R_XL: f32 = 16.;

/// Buttons, pickers and icon buttons.
pub const CONTROL: f32 = 28.;
/// A one-line text field, and the buttons on its line.
pub const FIELD: f32 = 36.;
/// A row of a menu, a list or a chooser.
pub const ROW: f32 = 32.;
pub const ICON: f32 = 16.;
pub const ICON_SM: f32 = 14.;
/// Popovers and tooltips sit this far from what opened them.
pub const GAP: f32 = 6.;

/// For a shortcut beside words (⌘N, ⇧A, +): the face's case-sensitive forms,
/// which lift +, −, × and the like from the lowercase middle to the capitals',
/// so a key sits level with the words — Korean ones too — as in the Mac's menus.
pub fn key_face() -> FontFeatures {
    FontFeatures(std::sync::Arc::new(vec![("case".into(), 1)]))
}

impl Palette {
    pub fn is_dark(&self) -> bool {
        self.bg.r < 0.5
    }

    /// What floats over the window: menus, popovers, the jump button.
    pub fn popover(&self) -> Rgba {
        if self.is_dark() { rgb(0x252422) } else { rgb(0xFFFFFF) }
    }

    /// A tooltip: a small dark chip in either appearance.
    pub fn tip_bg(&self) -> Rgba {
        if self.is_dark() { rgb(0x2F2E2C) } else { rgb(0x232220) }
    }

    pub fn tip_text(&self) -> Rgba {
        rgb(0xF3F0E9)
    }

    pub fn tip_muted(&self) -> Rgba {
        rgb(0xA9A7A1)
    }

    /// What went wrong: the status red, one red for both.
    pub fn danger(&self) -> Rgba {
        self.red
    }

    pub fn danger_bg(&self) -> Rgba {
        Rgba { a: if self.is_dark() { 0.12 } else { 0.08 }, ..self.red }
    }

    // ── controls ──

    /// The main button under the pointer: a step lighter (dark) or softer (light).
    pub fn primary_hover(&self) -> Rgba {
        if self.is_dark() { rgb(0x334A62) } else { rgb(0x2C2B27) }
    }

    /// The main button held down: a step deeper.
    pub fn primary_press(&self) -> Rgba {
        if self.is_dark() { rgb(0x233447) } else { rgb(0x050504) }
    }

    /// A key hint inside the main button (⌘↵): the label's colour, held back.
    pub fn primary_key(&self) -> Rgba {
        mix(self.on_primary, self.primary, 0.3)
    }

    /// A secondary or ghost button held down: the hover wash, doubled.
    pub fn press(&self) -> Rgba {
        Rgba { a: (self.wash.a * 2.).min(1.), ..self.wash }
    }

    /// A switch that is on: the accent, deep enough to carry a white knob.
    pub fn switch_on(&self) -> Rgba {
        if self.is_dark() { rgb(0x4378AD) } else { self.accent }
    }

    /// A switch that is off: a neutral track, a step off the card.
    pub fn switch_off(&self) -> Rgba {
        if self.is_dark() { rgb(0x42413E) } else { rgb(0xC9C7C2) }
    }

    /// The switch's knob, in either state.
    pub fn knob(&self) -> Rgba {
        if self.is_dark() { rgb(0xF3F0E9) } else { rgb(0xFFFFFF) }
    }
}

/// `a` moved `t` (0–1) of the way to `b`, alpha too.
pub fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0., 1.);
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    Rgba { r: lerp(a.r, b.r), g: lerp(a.g, b.g), b: lerp(a.b, b.b), a: lerp(a.a, b.a) }
}

/// WCAG's contrast ratio of two colours (1 to 21), alpha ignored.
#[cfg(test)]
pub fn contrast(a: Rgba, b: Rgba) -> f32 {
    let lum = |c: Rgba| {
        let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::{Appearance, Palette, contrast};
    use gpui_kit::WindowAppearance;

    /// Every colour text is drawn in, on every surface it is drawn on, reads
    /// at WCAG AA for small text (4.5:1), in both appearances.
    #[test]
    fn text_colours_pass_aa_on_every_surface() {
        for appearance in [WindowAppearance::Dark, WindowAppearance::Light] {
            let p = Palette::new(appearance);
            let surfaces = [("bg", p.bg), ("side", p.side), ("side_hover", p.side_hover), ("side_sel", p.side_sel), ("raised", p.raised), ("code_bg", p.code_bg), ("popover", p.popover())];
            let inks = [("text", p.text), ("prose", p.prose), ("text2", p.text2), ("muted", p.muted), ("accent", p.accent), ("danger", p.danger()), ("amber", p.amber), ("orange", p.orange), ("red", p.red)];
            for (surface, bg) in surfaces {
                for (ink, fg) in inks {
                    let ratio = contrast(fg, bg);
                    assert!(ratio >= 4.5, "{ink} on {surface} ({appearance:?}): {ratio:.2}");
                }
            }
            assert!(contrast(p.tip_muted(), p.tip_bg()) >= 4.5 && contrast(p.tip_text(), p.tip_bg()) >= 4.5);
            assert!(contrast(p.on_primary, p.primary) >= 4.5);
            // Reading text is softer than titles, and still far past AAA (7:1).
            assert!(contrast(p.prose, p.bg) < contrast(p.text, p.bg) && contrast(p.prose, p.bg) >= 7.);
        }
    }

    /// Lightness (CIE L*, 0–100) of a colour.
    fn lightness(c: gpui_kit::Rgba) -> f32 {
        let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        let y = 0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b);
        if y > 216. / 24389. { 116. * y.cbrt() - 16. } else { y * 24389. / 27. }
    }

    /// The dark reading surface is lifted off black, so the cards, code and
    /// popovers above it read as layers; the sidebar sits behind it, darker
    /// in either appearance.
    #[test]
    fn surfaces_step_up_from_a_sidebar_behind_the_conversation() {
        let p = Palette::new(WindowAppearance::Dark);
        assert!((5. ..=10.).contains(&lightness(p.bg)), "dark bg L* {}", lightness(p.bg));
        let steps = [p.side, p.bg, p.code_bg, p.raised, p.popover()];
        for pair in steps.windows(2) {
            assert!(lightness(pair[1]) - lightness(pair[0]) >= 1.5, "{} then {}", lightness(pair[0]), lightness(pair[1]));
        }
        assert!(lightness(p.side) < lightness(p.side_hover) && lightness(p.side_hover) < lightness(p.side_sel));
        let p = Palette::new(WindowAppearance::Light);
        assert!(lightness(p.side) < lightness(p.bg) && lightness(p.bg) < lightness(p.raised));
        assert!(lightness(p.side_sel) < lightness(p.side_hover) && lightness(p.side_hover) < lightness(p.side));
    }

    /// Every neutral, surface or ink, leans the same warm way (the brand's
    /// Bone), never the cool blue-grey some of them had.
    #[test]
    fn the_neutrals_share_one_warm_hue() {
        for appearance in [WindowAppearance::Dark, WindowAppearance::Light] {
            let p = Palette::new(appearance);
            let neutrals = [("bg", p.bg), ("side", p.side), ("side_hover", p.side_hover), ("side_sel", p.side_sel), ("raised", p.raised), ("code_bg", p.code_bg), ("text", p.text), ("prose", p.prose), ("text2", p.text2), ("muted", p.muted), ("tip_bg", p.tip_bg()), ("tip_muted", p.tip_muted()), ("switch_off", p.switch_off())];
            for (name, c) in neutrals {
                let hsla = gpui_kit::Hsla::from(c);
                if hsla.s > 0.01 && c != gpui_kit::rgb(0xFFFFFF) {
                    let hue = hsla.h * 360.;
                    assert!((20. ..=100.).contains(&hue), "{name} ({appearance:?}) leans {hue:.0}°");
                }
            }
        }
    }

    /// Ink comes in even steps — titles, reading text, secondary, muted — and
    /// muted (times, counts) is still easy to read; a link stands apart from
    /// the words around it rather than reading as faded text.
    #[test]
    fn ink_steps_down_evenly_and_links_stand_apart() {
        for appearance in [WindowAppearance::Dark, WindowAppearance::Light] {
            let p = Palette::new(appearance);
            let on = |c| contrast(c, p.bg);
            assert!(on(p.text) > on(p.prose) && on(p.prose) > on(p.text2) && on(p.text2) > on(p.muted));
            assert!(on(p.muted) >= 5.8, "muted on bg ({appearance:?}): {:.2}", on(p.muted));
            assert!(contrast(p.accent, p.prose) >= 1.5, "link against the words ({appearance:?}): {:.2}", contrast(p.accent, p.prose));
        }
    }

    /// Every language gets every CJK face, its own first: Japanese Han in
    /// Hiragino's shapes, Chinese in PingFang's, Hangul (and Latin UIs) in
    /// Apple SD Gothic Neo's.
    #[test]
    fn each_language_draws_cjk_in_its_own_faces_first() {
        use equipe_core::i18n::Lang;
        for (lang, first) in [(Lang::Ko, "Apple SD Gothic Neo"), (Lang::Ja, "Hiragino Sans"), (Lang::ZhHans, "PingFang SC"), (Lang::ZhHant, "PingFang TC"), (Lang::En, "Apple SD Gothic Neo"), (Lang::De, "Apple SD Gothic Neo")] {
            let faces = super::fallbacks(lang);
            let faces = faces.fallback_list();
            assert_eq!(faces[0], first, "{lang:?}");
            assert_eq!(faces.len(), 4, "{lang:?}: {faces:?}");
            for face in ["Apple SD Gothic Neo", "Hiragino Sans", "PingFang SC", "PingFang TC"] {
                assert!(faces.iter().any(|f| f == face), "{lang:?} lacks {face}");
            }
        }
    }

    #[test]
    fn a_picked_appearance_wins_over_the_macs_and_system_follows_it() {
        assert_eq!(Appearance::System.resolve(WindowAppearance::Dark), WindowAppearance::Dark);
        assert_eq!(Appearance::Light.resolve(WindowAppearance::VibrantDark), WindowAppearance::Light);
        assert!(Palette::new(Appearance::Dark.resolve(WindowAppearance::Light)).is_dark());
        assert!(Appearance::ALL.iter().all(|a| Appearance::parse(a.code()) == Some(*a)));
        assert_eq!(Appearance::parse("sepia"), None);
    }

    #[test]
    fn no_projects_colour_reads_as_a_status() {
        let hue = |c: gpui_kit::Rgba| gpui_kit::Hsla::from(c).h * 360.;
        let apart = |a: f32, b: f32| (a - b).abs().min(360. - (a - b).abs());
        for p in [Palette::new(WindowAppearance::Dark), Palette::new(WindowAppearance::Light)] {
            for at in 0..12 {
                let c = hue(p.member(super::PROJECT_HUES[at % super::PROJECT_HUES.len()]));
                for status in [p.orange, p.amber] {
                    assert!(apart(c, hue(status)) >= 40., "project {at}'s hue {c} is too near a status's {}", hue(status));
                }
            }
        }
    }
}
