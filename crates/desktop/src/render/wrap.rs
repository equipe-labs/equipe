//! How a paragraph breaks into lines: Korean at the spaces between words
//! (CSS `word-break: keep-all`, patched into GPUI — see `vendor/gpui-pre`),
//! English as before, Japanese and Chinese between any two characters, and a
//! word longer than the line where it must. Tests only: the wrapping is
//! GPUI's, run here through its own layout with a text system of fixed
//! advances (the Mac's needs the main thread).

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::sync::Arc;
    use std::time::Instant;

    use gpui_kit::{
        Bounds, DevicePixels, Font, FontId, FontMetrics, FontRun, GlyphId, Hsla, LineLayout, Pixels, PlatformTextSystem, RenderGlyphParams, Result, ShapedGlyph, ShapedRun, SharedString, Size, TextRenderingMode, TextRun, TextSystem,
        WindowTextSystem, font, point, px, size,
    };

    use crate::theme::BODY;

    /// Hangul, kana and Han a full em wide, the rest half: about the Mac's
    /// reading font and its Korean fallback.
    struct Fixed;

    fn advance(c: char) -> f32 {
        if c as u32 >= 0x1100 { 1. } else { 0.5 }
    }

    impl PlatformTextSystem for Fixed {
        fn add_fonts(&self, _: Vec<Cow<'static, [u8]>>) -> Result<()> {
            Ok(())
        }
        fn all_font_names(&self) -> Vec<String> {
            vec!["Body".into()]
        }
        fn font_id(&self, _: &Font) -> Result<FontId> {
            Ok(FontId(1))
        }
        fn font_metrics(&self, _: FontId) -> FontMetrics {
            FontMetrics {
                units_per_em: 1000,
                ascent: 800.,
                descent: -200.,
                line_gap: 0.,
                underline_position: -100.,
                underline_thickness: 50.,
                cap_height: 700.,
                x_height: 500.,
                bounding_box: Bounds { origin: point(0., -200.), size: size(1000., 1000.) },
            }
        }
        fn typographic_bounds(&self, _: FontId, _: GlyphId) -> Result<Bounds<f32>> {
            Ok(Bounds { origin: point(0., 0.), size: size(500., 700.) })
        }
        fn advance(&self, _: FontId, glyph: GlyphId) -> Result<Size<f32>> {
            Ok(size(1000. * advance(char::from_u32(glyph.0).unwrap_or('m')), 0.))
        }
        fn glyph_for_char(&self, _: FontId, c: char) -> Option<GlyphId> {
            Some(GlyphId(c as u32))
        }
        fn glyph_raster_bounds(&self, _: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
            Ok(Bounds::default())
        }
        fn rasterize_glyph(&self, _: &RenderGlyphParams, bounds: Bounds<DevicePixels>) -> Result<(Size<DevicePixels>, Vec<u8>)> {
            Ok((bounds.size, Vec::new()))
        }
        fn layout_line(&self, text: &str, font_size: Pixels, runs: &[FontRun]) -> LineLayout {
            let mut x = px(0.);
            let glyphs = text
                .char_indices()
                .map(|(index, c)| {
                    let glyph = ShapedGlyph { id: GlyphId(c as u32), position: point(x, px(0.)), index, is_emoji: false };
                    x += font_size * advance(c);
                    glyph
                })
                .collect();
            let font_id = runs.first().map_or(FontId(1), |r| r.font_id);
            LineLayout { font_size, width: x, ascent: font_size * 0.8, descent: font_size * 0.2, runs: vec![ShapedRun { font_id, glyphs }], len: text.len() }
        }
        fn recommended_rendering_mode(&self, _: FontId, _: Pixels) -> TextRenderingMode {
            TextRenderingMode::Grayscale
        }
    }

    fn text_system() -> WindowTextSystem {
        WindowTextSystem::new(Arc::new(TextSystem::new(Arc::new(Fixed))))
    }

    /// `text` laid out at `width` (in points, at the body size): its rows.
    fn rows_in(system: &WindowTextSystem, text: &str, width: f32) -> Vec<String> {
        let run = TextRun { len: text.len(), font: font("Body"), color: Hsla::default(), background_color: None, underline: None, strikethrough: None };
        let lines = system.shape_text(SharedString::from(text.to_owned()), px(BODY), &[run], Some(px(width)), None).expect("shaped");
        let mut rows = Vec::new();
        for line in lines.iter() {
            let layout = &line.unwrapped_layout;
            let mut from = 0;
            for boundary in &line.wrap_boundaries {
                let at = layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index;
                rows.push(line.text[from..at].to_owned());
                from = at;
            }
            rows.push(line.text[from..].to_owned());
        }
        rows
    }

    fn rows(text: &str, width: f32) -> Vec<String> {
        rows_in(&text_system(), text, width)
    }

    /// Every row but the first starts a word: a space comes before it.
    fn breaks_between_words(text: &str, rows: &[String]) -> bool {
        let mut at = 0;
        rows.iter().enumerate().all(|(n, row)| {
            let ok = n == 0 || text[..at].ends_with(' ');
            at += row.len();
            ok
        })
    }

    const KOREAN: &str = "좋은 질문이에요. 지금 구조에서는 사이드바에서 프로젝트를 누르면 PM이 그 프로젝트 이야기로 대화를 이어 가고, 다른 프로젝트 이야기는 접힌 줄로 따로 모입니다. 말씀하신 건 매번 지시하지 않아도 PM이 알아서 다음 일을 고르는 흐름을 만들고 싶은데, 그게 지금 어디까지 되는지 궁금하신 거죠?";

    #[test]
    fn korean_breaks_between_words() {
        for width in [180., 260., 333., 420., 600.] {
            let rows = rows(KOREAN, width);
            assert!(rows.len() > 1, "wraps at {width}");
            assert!(breaks_between_words(KOREAN, &rows), "at {width}: {rows:#?}");
            assert_eq!(rows.concat(), KOREAN);
        }
        // Korean beside English and digits is one word: `PM이`, `T-119처럼`.
        let mixed = "가나다라마바사 PM이 T-119처럼 아자차카";
        let cut = rows(mixed, 120.);
        assert!(cut.len() > 2 && breaks_between_words(mixed, &cut), "{cut:#?}");
    }

    #[test]
    fn english_breaks_between_words_as_before() {
        let text = "The quick brown fox jumps over the lazy dog, and then it keeps running across the field until the evening comes.";
        for width in [150., 240., 400.] {
            let rows = rows(text, width);
            assert!(rows.len() > 1);
            assert!(breaks_between_words(text, &rows), "at {width}: {rows:#?}");
        }
    }

    #[test]
    fn japanese_and_chinese_still_break_anywhere() {
        for text in [
            "吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。何でも薄暗いじめじめした所でニャーニャー泣いていた事だけは記憶している。",
            "我们的项目需要一个能够自动选择下一项工作的流程，这样就不必每次都下达指示了，而且决定只留给需要的人。",
        ] {
            let rows = rows(text, 200.);
            // No spaces, and still every row but the last is full: 13 characters of 15 points.
            assert!(rows.len() > 2, "{rows:#?}");
            assert!(rows[..rows.len() - 1].iter().all(|r| r.chars().count() == 13), "{rows:#?}");
            assert_eq!(rows.concat(), text);
        }
    }

    #[test]
    fn a_word_longer_than_the_line_still_breaks() {
        let text = "가나다라마바사아자차카타파하가나다라마바사아자차카타파하가나다라마바사아자차카타파하 끝";
        let cut = rows(text, 160.);
        assert!(cut.len() >= 3, "the long word is cut: {cut:#?}");
        assert_eq!(cut[0].chars().count(), 10, "as many syllables as fit: {cut:#?}");
        assert_eq!(cut.concat(), text);
        let url = "https://example.com/very/long/path/that/keeps/going/without/any/spaces/at/all";
        assert!(rows(url, 160.).len() > 1);
    }

    /// Wrapping a long reply costs what it did: the change is one range check
    /// per character. (`cargo test wrap -- --nocapture` prints the time.)
    #[test]
    fn wrapping_long_paragraphs_stays_quick() {
        let system = text_system();
        let paragraph = KOREAN.repeat(20);
        // Each pass lays out new text, so the layout cache never answers.
        let started = Instant::now();
        let mut lines = 0;
        for n in 0..50 {
            lines += rows_in(&system, &format!("{n} {paragraph}"), 600.).len();
        }
        let each = started.elapsed() / 50;
        eprintln!("{} characters wrapped into {} rows in {each:?}", paragraph.chars().count(), lines / 50);
        assert!(each.as_millis() < 50, "{each:?} for one long paragraph");
    }
}
