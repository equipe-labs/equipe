//! Screen points, and the pixels of the image a member was shown.
//!
//! macOS places windows and posts events in points; the model looks at an
//! image sized for it and answers in that image's pixels. A [`Mapping`] is the
//! bridge, kept per member from its last screenshot.

/// A point on the screen, in points (global display coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

/// A rectangle on the screen, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Frame {
    pub fn center(&self) -> Pt {
        Pt { x: self.x + self.w / 2.0, y: self.y + self.h / 2.0 }
    }

    pub fn contains(&self, at: Pt) -> bool {
        at.x >= self.x && at.x < self.x + self.w && at.y >= self.y && at.y < self.y + self.h
    }
}

/// The longest edge of an image for the model, and the most pixels it has
/// (Claude's vision downsizes anything larger).
pub const MAX_EDGE: f64 = 1568.0;
pub const MAX_PIXELS: f64 = 1_150_000.0;

/// The size to capture something `w`×`h` device pixels at for the model:
/// never larger than it is, its long edge and pixel count within limits.
pub fn model_size(w: f64, h: f64) -> (usize, usize) {
    if w < 1.0 || h < 1.0 {
        return (1, 1);
    }
    let scale = (MAX_EDGE / w.max(h)).min((MAX_PIXELS / (w * h)).sqrt()).min(1.0);
    (((w * scale).round() as usize).max(1), ((h * scale).round() as usize).max(1))
}

/// How the image a member saw lies on the screen: pixel (0, 0) at `origin`,
/// `width`×`height` pixels covering `frame`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mapping {
    pub frame: Frame,
    pub width: usize,
    pub height: usize,
}

impl Mapping {
    /// The image a capture of `frame` gets, on a display of `backing` device
    /// pixels per point.
    pub fn for_frame(frame: Frame, backing: f64) -> Self {
        let (width, height) = model_size(frame.w * backing, frame.h * backing);
        Self { frame, width, height }
    }

    /// Screen points per image pixel.
    pub fn scale(&self) -> f64 {
        self.frame.w / self.width as f64
    }

    /// The screen point under pixel (`x`, `y`), if it is in the image.
    pub fn to_screen(&self, x: f64, y: f64) -> Option<Pt> {
        if !(0.0..=self.width as f64).contains(&x) || !(0.0..=self.height as f64).contains(&y) {
            return None;
        }
        Some(Pt {
            x: self.frame.x + x * self.frame.w / self.width as f64,
            y: self.frame.y + y * self.frame.h / self.height as f64,
        })
    }

    /// Where a screen point falls in the image (possibly outside it).
    pub fn to_image(&self, at: Pt) -> (i64, i64) {
        (
            ((at.x - self.frame.x) * self.width as f64 / self.frame.w).round() as i64,
            ((at.y - self.frame.y) * self.height as f64 / self.frame.h).round() as i64,
        )
    }

    /// A frame on the screen as the member reads it: `(x, y) w×h` in image pixels.
    pub fn describe(&self, frame: Frame) -> String {
        let (x, y) = self.to_image(Pt { x: frame.x, y: frame.y });
        let (w, h) = ((frame.w / self.scale()).round() as i64, (frame.h * self.height as f64 / self.frame.h).round() as i64);
        format!("({x}, {y}) {w}×{h}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_retina_screen_is_captured_small_enough_for_the_model() {
        // 2560×1440 pt at 2×: 5120×2880 px.
        let (w, h) = model_size(5120.0, 2880.0);
        assert!(w as f64 <= MAX_EDGE && (w * h) as f64 <= MAX_PIXELS * 1.001, "{w}×{h}");
        assert!((w as f64 / h as f64 - 16.0 / 9.0).abs() < 0.01, "keeps its shape");
        assert_eq!(model_size(800.0, 600.0), (800, 600), "never enlarged");
    }

    #[test]
    fn image_pixels_map_to_screen_points_and_back() {
        let screen = Frame { x: 0.0, y: 0.0, w: 2560.0, h: 1440.0 };
        let map = Mapping::for_frame(screen, 2.0);
        let center = map.to_screen(map.width as f64 / 2.0, map.height as f64 / 2.0).unwrap();
        assert!((center.x - 1280.0).abs() < 0.01 && (center.y - 720.0).abs() < 0.01);
        assert_eq!(map.to_image(center), (map.width as i64 / 2, map.height as i64 / 2));
        assert_eq!(map.to_screen(-1.0, 10.0), None, "outside the image");
        assert_eq!(map.to_screen(10.0, map.height as f64 + 1.0), None);

        // A window's capture starts at the window.
        let window = Mapping::for_frame(Frame { x: 100.0, y: 50.0, w: 800.0, h: 600.0 }, 1.0);
        assert_eq!(window.to_screen(0.0, 0.0), Some(Pt { x: 100.0, y: 50.0 }));
        assert_eq!(window.describe(Frame { x: 110.0, y: 60.0, w: 20.0, h: 10.0 }), "(10, 10) 20×10");
    }
}
