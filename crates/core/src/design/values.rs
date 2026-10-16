//! What a token's value is, read from DTCG's JSON: a colour (`#1D4ED8`,
//! `rgb(…)`, or the 2025 `{ colorSpace, components }` form), a size
//! (`16px`, `1rem`, `{ value, unit }`), a duration, a curve, a font stack, a
//! weight, a type style, a shadow — and a line saying it, for a list.

use serde_json::Value;

/// An sRGB colour; alpha 0–1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

impl Color {
    pub const WHITE: Color = Color { r: 255, g: 255, b: 255, a: 1. };
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0, a: 1. };

    pub fn rgb(hex: u32) -> Self {
        Self { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8, a: 1. }
    }

    /// `#1D4ED8`, or `#1D4ED880` when it isn't opaque.
    pub fn hex(&self) -> String {
        if self.a >= 0.999 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, (self.a * 255.).round() as u8)
        }
    }

    /// As a `0xRRGGBBAA` for drawing.
    pub fn rgba(&self) -> u32 {
        (u32::from(self.r) << 24) | (u32::from(self.g) << 16) | (u32::from(self.b) << 8) | (self.a.clamp(0., 1.) * 255.).round() as u32
    }
}

/// A colour, from any form DTCG or CSS writes it in.
pub fn color(v: &Value) -> Option<Color> {
    match v {
        Value::String(s) => color_text(s.trim()),
        Value::Object(o) => {
            if let Some(hex) = o.get("hex").and_then(Value::as_str)
                && let Some(mut c) = color_text(hex)
            {
                if let Some(alpha) = o.get("alpha").and_then(Value::as_f64) {
                    c.a = alpha.clamp(0., 1.) as f32;
                }
                return Some(c);
            }
            let space = o.get("colorSpace").and_then(Value::as_str).unwrap_or("srgb");
            if space != "srgb" {
                return None;
            }
            let parts: Vec<f64> = o.get("components")?.as_array()?.iter().map(Value::as_f64).collect::<Option<_>>()?;
            let [r, g, b] = parts[..] else { return None };
            let byte = |x: f64| (x.clamp(0., 1.) * 255.).round() as u8;
            Some(Color { r: byte(r), g: byte(g), b: byte(b), a: o.get("alpha").and_then(Value::as_f64).unwrap_or(1.).clamp(0., 1.) as f32 })
        }
        _ => None,
    }
}

fn color_text(s: &str) -> Option<Color> {
    if s.eq_ignore_ascii_case("transparent") {
        return Some(Color { a: 0., ..Color::BLACK });
    }
    if let Some(hex) = s.strip_prefix('#') {
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let digit = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok().map(|d| d * 17);
        let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        return match hex.len() {
            3 => Some(Color { r: digit(0)?, g: digit(1)?, b: digit(2)?, a: 1. }),
            4 => Some(Color { r: digit(0)?, g: digit(1)?, b: digit(2)?, a: f32::from(digit(3)?) / 255. }),
            6 => Some(Color { r: pair(0)?, g: pair(2)?, b: pair(4)?, a: 1. }),
            8 => Some(Color { r: pair(0)?, g: pair(2)?, b: pair(4)?, a: f32::from(pair(6)?) / 255. }),
            _ => None,
        };
    }
    let lower = s.to_ascii_lowercase();
    let inner = lower.strip_prefix("rgba(").or_else(|| lower.strip_prefix("rgb("))?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split([',', ' ', '/']).map(str::trim).filter(|p| !p.is_empty()).collect();
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let channel = |p: &str| -> Option<u8> {
        match p.strip_suffix('%') {
            Some(pct) => pct.parse::<f64>().ok().map(|x| (x.clamp(0., 100.) * 2.55).round() as u8),
            None => p.parse::<f64>().ok().map(|x| x.clamp(0., 255.).round() as u8),
        }
    };
    let alpha = match parts.get(3) {
        Some(p) => match p.strip_suffix('%') {
            Some(pct) => pct.parse::<f32>().ok()? / 100.,
            None => p.parse::<f32>().ok()?,
        },
        None => 1.,
    };
    Some(Color { r: channel(parts[0])?, g: channel(parts[1])?, b: channel(parts[2])?, a: alpha.clamp(0., 1.) })
}

/// The pixels on the root font size a `rem` is.
const REM: f64 = 16.;

/// A size, in px: `16px`, `1.5rem`, `0`, `{ value, unit }`, or a bare number.
pub fn dimension(v: &Value) -> Option<f64> {
    let (n, unit) = number_and_unit(v)?;
    match unit.as_str() {
        "px" | "pt" | "" => Some(n),
        "rem" | "em" => Some(n * REM),
        _ => None,
    }
}

/// A duration, in ms: `200ms`, `0.2s`, `{ value, unit }`, or a bare number of ms.
pub fn duration_ms(v: &Value) -> Option<f64> {
    let (n, unit) = number_and_unit(v)?;
    match unit.as_str() {
        "ms" | "" => Some(n),
        "s" => Some(n * 1000.),
        _ => None,
    }
}

fn number_and_unit(v: &Value) -> Option<(f64, String)> {
    match v {
        Value::Number(n) => n.as_f64().map(|n| (n, String::new())),
        Value::String(s) => {
            let s = s.trim();
            let at = s.find(|c: char| c.is_ascii_alphabetic() || c == '%').unwrap_or(s.len());
            let n = s[..at].trim().parse::<f64>().ok()?;
            Some((n, s[at..].trim().to_ascii_lowercase()))
        }
        Value::Object(o) => Some((o.get("value")?.as_f64()?, o.get("unit").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase())),
        _ => None,
    }
}

/// A timing curve's four numbers; CSS's keywords too.
pub fn cubic_bezier(v: &Value) -> Option<[f64; 4]> {
    match v {
        Value::Array(items) if items.len() == 4 => {
            let n: Vec<f64> = items.iter().map(Value::as_f64).collect::<Option<_>>()?;
            ((0. ..=1.).contains(&n[0]) && (0. ..=1.).contains(&n[2])).then(|| [n[0], n[1], n[2], n[3]])
        }
        Value::String(s) => match s.trim() {
            "linear" => Some([0., 0., 1., 1.]),
            "ease" => Some([0.25, 0.1, 0.25, 1.]),
            "ease-in" => Some([0.42, 0., 1., 1.]),
            "ease-out" => Some([0., 0., 0.58, 1.]),
            "ease-in-out" => Some([0.42, 0., 0.58, 1.]),
            other => {
                let inner = other.strip_prefix("cubic-bezier(")?.strip_suffix(')')?;
                let n: Vec<f64> = inner.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
                cubic_bezier(&Value::Array(n.into_iter().map(Value::from).collect()))
            }
        },
        _ => None,
    }
}

/// Where a curve stands at `t` (0–1 of the time): the eased progress.
pub fn ease(curve: [f64; 4], t: f64) -> f64 {
    let [x1, y1, x2, y2] = curve;
    if t <= 0. {
        return 0.;
    }
    if t >= 1. {
        return 1.;
    }
    let bez = |a: f64, b: f64, s: f64| 3. * a * (1. - s).powi(2) * s + 3. * b * (1. - s) * s * s + s.powi(3);
    // Find the curve's s for this x (time), by bisection: steady and exact enough.
    let (mut lo, mut hi) = (0., 1.);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.;
        if bez(x1, x2, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bez(y1, y2, (lo + hi) / 2.)
}

/// A font stack: a name, a list, or CSS's `"Inter", sans-serif`.
pub fn font_family(v: &Value) -> Option<Vec<String>> {
    let names: Vec<String> = match v {
        Value::String(s) => s.split(',').map(|n| n.trim().trim_matches(['"', '\'']).to_owned()).collect(),
        Value::Array(items) => items.iter().map(|i| i.as_str().map(str::to_owned)).collect::<Option<_>>()?,
        _ => return None,
    };
    let names: Vec<String> = names.into_iter().filter(|n| !n.is_empty()).collect();
    (!names.is_empty()).then_some(names)
}

/// A weight, 1–1000, from a number or one of DTCG's names.
pub fn font_weight(v: &Value) -> Option<u16> {
    match v {
        Value::Number(n) => n.as_f64().filter(|w| (1. ..=1000.).contains(w)).map(|w| w.round() as u16),
        Value::String(s) => Some(match s.trim().to_ascii_lowercase().replace([' ', '_'], "-").as_str() {
            "thin" | "hairline" => 100,
            "extra-light" | "ultra-light" => 200,
            "light" => 300,
            "normal" | "regular" | "book" => 400,
            "medium" => 500,
            "semi-bold" | "demi-bold" => 600,
            "bold" => 700,
            "extra-bold" | "ultra-bold" => 800,
            "black" | "heavy" => 900,
            "extra-black" | "ultra-black" => 950,
            other => return other.parse::<u16>().ok().filter(|w| (1..=1000).contains(w)),
        }),
        _ => None,
    }
}

/// A type style, as much of it as is said.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Typography {
    pub family: Vec<String>,
    pub size: Option<f64>,
    pub weight: Option<u16>,
    /// The line's height in px (a ratio is multiplied by the size, 16 when none).
    pub line: Option<f64>,
    /// In px.
    pub tracking: Option<f64>,
}

pub fn typography(v: &Value) -> Option<Typography> {
    let o = v.as_object()?;
    let size = o.get("fontSize").and_then(dimension);
    let line = o.get("lineHeight").and_then(|l| match l {
        Value::Number(n) => n.as_f64().map(|r| r * size.unwrap_or(REM)),
        Value::String(s) if s.trim().parse::<f64>().is_ok() => s.trim().parse::<f64>().ok().map(|r| r * size.unwrap_or(REM)),
        other => dimension(other),
    });
    let tracking = o.get("letterSpacing").and_then(|l| match number_and_unit(l) {
        Some((n, unit)) if unit == "em" => Some(n * size.unwrap_or(REM)),
        Some((n, unit)) if unit == "%" => Some(n / 100. * size.unwrap_or(REM)),
        _ => dimension(l),
    });
    Some(Typography {
        family: o.get("fontFamily").and_then(font_family).unwrap_or_default(),
        size,
        weight: o.get("fontWeight").and_then(font_weight),
        line,
        tracking,
    })
}

/// One shadow layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub color: Color,
    pub x: f64,
    pub y: f64,
    pub blur: f64,
    pub spread: f64,
    pub inset: bool,
}

/// A shadow, one layer or a list of them.
pub fn shadows(v: &Value) -> Option<Vec<Shadow>> {
    let one = |v: &Value| -> Option<Shadow> {
        let o = v.as_object()?;
        let size = |k: &str| o.get(k).map_or(Some(0.), dimension);
        Some(Shadow {
            color: o.get("color").and_then(color).unwrap_or(Color { a: 0.25, ..Color::BLACK }),
            x: size("offsetX")?,
            y: size("offsetY")?,
            blur: size("blur")?,
            spread: size("spread")?,
            inset: o.get("inset").and_then(Value::as_bool).unwrap_or(false),
        })
    };
    match v {
        Value::Array(items) => items.iter().map(one).collect(),
        other => one(other).map(|s| vec![s]),
    }
}

/// A transition: how long, after how long, on what curve.
pub fn transition(v: &Value) -> Option<(f64, f64, [f64; 4])> {
    let o = v.as_object()?;
    Some((o.get("duration").and_then(duration_ms)?, o.get("delay").and_then(duration_ms).unwrap_or(0.), o.get("timingFunction").and_then(cubic_bezier).unwrap_or([0.25, 0.1, 0.25, 1.])))
}

/// Whether `v` is a value of `kind` (a type this doesn't know always is).
pub fn fits(kind: &str, v: &Value) -> bool {
    match kind {
        "color" => color(v).is_some(),
        "dimension" => dimension(v).is_some(),
        "duration" => duration_ms(v).is_some(),
        "cubicBezier" => cubic_bezier(v).is_some(),
        "fontFamily" => font_family(v).is_some(),
        "fontWeight" => font_weight(v).is_some(),
        "number" => v.is_number(),
        "typography" => typography(v).is_some(),
        "shadow" => shadows(v).is_some(),
        "transition" => transition(v).is_some(),
        _ => true,
    }
}

fn num(n: f64) -> String {
    if (n - n.round()).abs() < 0.005 { format!("{}", n.round() as i64) } else { format!("{n:.2}").trim_end_matches('0').trim_end_matches('.').to_owned() }
}

/// A token's value in a line: `#1D4ED8`, `16px`, `200ms`,
/// `cubic-bezier(0.2, 0, 0, 1)`, `Inter 16/24 · 600`.
pub fn describe(kind: &str, v: &Value) -> String {
    let plain = || match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    match kind {
        "color" => color(v).map(|c| c.hex()),
        "dimension" => dimension(v).map(|px| format!("{}px", num(px))),
        "duration" => duration_ms(v).map(|ms| format!("{}ms", num(ms))),
        "cubicBezier" => cubic_bezier(v).map(|c| format!("cubic-bezier({})", c.map(num).join(", "))),
        "fontFamily" => font_family(v).map(|f| f.join(", ")),
        "fontWeight" => font_weight(v).map(|w| w.to_string()),
        "typography" => typography(v).map(|t| {
            let mut parts = vec![t.family.first().cloned().unwrap_or_default()];
            match (t.size, t.line) {
                (Some(s), Some(l)) => parts.push(format!("{}/{}", num(s), num(l))),
                (Some(s), None) => parts.push(num(s)),
                _ => {}
            }
            let head = parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
            match t.weight {
                Some(w) => format!("{head} · {w}"),
                None => head,
            }
        }),
        "shadow" => shadows(v).map(|layers| {
            layers.iter().map(|s| format!("{}{} {} {} {} {}", if s.inset { "inset " } else { "" }, num(s.x), num(s.y), num(s.blur), num(s.spread), s.color.hex())).collect::<Vec<_>>().join(", ")
        }),
        "transition" => transition(v).map(|(d, delay, c)| format!("{}ms{} · cubic-bezier({})", num(d), if delay > 0. { format!(" +{}ms", num(delay)) } else { String::new() }, c.map(num).join(", "))),
        _ => None,
    }
    .unwrap_or_else(plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn colours_in_every_form() {
        assert_eq!(color(&json!("#1D4ED8")).unwrap().hex(), "#1D4ED8");
        assert_eq!(color(&json!("#fff")).unwrap(), Color::WHITE);
        assert_eq!(color(&json!("#00000080")).unwrap().hex(), "#00000080");
        assert_eq!(color(&json!("rgba(10, 10, 12, .14)")).unwrap().hex(), "#0A0A0C24");
        assert_eq!(color(&json!("rgb(255 0 0 / 50%)")).unwrap().hex(), "#FF000080");
        assert_eq!(color(&json!({ "colorSpace": "srgb", "components": [0.114, 0.306, 0.847] })).unwrap().hex(), "#1D4ED8");
        assert_eq!(color(&json!({ "colorSpace": "srgb", "components": [0, 0, 0], "hex": "#3F6FF5" })).unwrap().hex(), "#3F6FF5", "the hex wins");
        for bad in [json!("#GG0000"), json!("#12345"), json!("blue-ish"), json!(12), json!({ "colorSpace": "oklch", "components": [0.5, 0.1, 200] })] {
            assert!(color(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn sizes_durations_and_curves() {
        assert_eq!(dimension(&json!("16px")), Some(16.));
        assert_eq!(dimension(&json!("1.5rem")), Some(24.));
        assert_eq!(dimension(&json!({ "value": 8, "unit": "px" })), Some(8.));
        assert_eq!(dimension(&json!("12vw")), None);
        assert_eq!(duration_ms(&json!("0.2s")), Some(200.));
        assert_eq!(duration_ms(&json!({ "value": 120, "unit": "ms" })), Some(120.));
        assert_eq!(cubic_bezier(&json!([0.2, 0, 0, 1])), Some([0.2, 0., 0., 1.]));
        assert_eq!(cubic_bezier(&json!("ease-out")), Some([0., 0., 0.58, 1.]));
        assert_eq!(cubic_bezier(&json!([1.5, 0, 0, 1])), None, "time can't run back");
        let linear = [0., 0., 1., 1.];
        assert!((ease(linear, 0.3) - 0.3).abs() < 1e-6);
        let out = [0., 0., 0.58, 1.];
        assert!(ease(out, 0.5) > 0.6 && ease(out, 0.) == 0. && (ease(out, 1.) - 1.).abs() < 1e-9);
    }

    #[test]
    fn fonts_weights_and_type_styles() {
        assert_eq!(font_family(&json!("'Inter Variable', Inter, system-ui")).unwrap(), ["Inter Variable", "Inter", "system-ui"]);
        assert_eq!(font_weight(&json!("semi-bold")), Some(600));
        assert_eq!(font_weight(&json!(1200)), None);
        let t = typography(&json!({ "fontFamily": ["Inter"], "fontSize": "16px", "fontWeight": "bold", "lineHeight": 1.5, "letterSpacing": "-0.02em" })).unwrap();
        assert_eq!((t.size, t.weight, t.line), (Some(16.), Some(700), Some(24.)));
        assert!((t.tracking.unwrap() + 0.32).abs() < 1e-9);
        assert_eq!(describe("typography", &json!({ "fontFamily": "Inter", "fontSize": "15px", "lineHeight": "25px", "fontWeight": 400 })), "Inter 15/25 · 400");
    }

    #[test]
    fn shadows_and_a_line_for_each_kind() {
        let s = shadows(&json!([{ "color": "#00000033", "offsetX": "0px", "offsetY": "8px", "blur": "24px", "spread": "-4px" }])).unwrap();
        assert_eq!((s[0].y, s[0].blur, s[0].spread), (8., 24., -4.));
        assert_eq!(describe("shadow", &json!({ "color": "#000", "offsetX": 0, "offsetY": 1, "blur": 2, "spread": 0 })), "0 1 2 0 #000000");
        assert_eq!(describe("dimension", &json!("1rem")), "16px");
        assert_eq!(describe("cubicBezier", &json!([0.2, 0, 0, 1])), "cubic-bezier(0.2, 0, 0, 1)");
        assert_eq!(describe("transition", &json!({ "duration": "160ms", "timingFunction": [0, 0, 0.58, 1] })), "160ms · cubic-bezier(0, 0, 0.58, 1)");
        assert_eq!(describe("strokeStyle", &json!("dashed")), "dashed", "a type it doesn't know: as written");
    }
}
