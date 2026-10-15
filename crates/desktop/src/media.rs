//! What a video, a PDF or another file shows before it is sent, read on this
//! Mac and always off the UI thread: a video's length, size and frames
//! (AVFoundation; `ffmpeg` when AVFoundation can't open it) and one sheet of
//! them with their times, a PDF's page count and first page (CoreGraphics),
//! anything else's Quick Look picture. Members can't watch a video; they read
//! its frames, so the frames are what a video carries to them.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use image::{Rgb, RgbImage, imageops::FilterType};

/// A frame's or a page's longest side, as a member can use it.
pub const LONG_SIDE: u32 = 1568;

/// One frame of a video, at `at` seconds.
#[derive(Debug, Clone)]
pub struct Frame {
    pub at: f64,
    pub image: RgbImage,
}

/// What a video is, and its frames, evenly spaced.
#[derive(Debug, Clone)]
pub struct Video {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: Option<f32>,
    pub frames: Vec<Frame>,
}

/// How many frames a video of `duration` seconds carries: one per ten
/// seconds, never fewer than 4 nor more than 12.
pub fn frame_count(duration: f64) -> usize {
    ((duration / 10.).round().max(0.) as usize).clamp(4, 12)
}

/// When the frames are taken: the middles of `frame_count` equal parts, so
/// neither a black first frame nor the fade at the end.
pub fn frame_times(duration: f64) -> Vec<f64> {
    let n = frame_count(duration);
    (0..n).map(|i| duration * (i as f64 + 0.5) / n as f64).collect()
}

/// `0:03`, `1:42`, `1:02:03`.
pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.).round() as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// A video's length, size and frames.
pub fn video(path: &Path) -> Result<Video, String> {
    #[cfg(target_os = "macos")]
    if let Ok(video) = avf::video(path) {
        return Ok(video);
    }
    ffmpeg::video(path)
}

/// A sound's length, when it can be read.
pub fn audio_duration(path: &Path) -> Option<f64> {
    #[cfg(target_os = "macos")]
    if let Some(seconds) = avf::duration(path) {
        return Some(seconds);
    }
    ffmpeg::probe(path).and_then(|p| p.duration)
}

/// A PDF's page count, and its first page drawn `side` pixels on its long side.
pub fn pdf(path: &Path, side: u32) -> Result<(usize, Option<RgbImage>), String> {
    #[cfg(target_os = "macos")]
    {
        cg::pdf(path, side)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, side);
        Err("no PDF reader here".into())
    }
}

/// Quick Look's picture of any file (a document's first page, an app's icon),
/// `side` pixels; `None` when it has none or takes too long.
pub fn quick_look(path: &Path, side: u32) -> Option<RgbImage> {
    let out = std::env::temp_dir().join(format!("equipe-ql-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos()));
    std::fs::create_dir_all(&out).ok()?;
    let child = Command::new("/usr/bin/qlmanage").args(["-t", "-s", &side.to_string(), "-o"]).arg(&out).arg(path).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok();
    let made = child.and_then(|child| wait(child, Duration::from_secs(6))).is_some_and(|ok| ok);
    let picture = made
        .then(|| std::fs::read_dir(&out).ok()?.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "png")))
        .flatten()
        .and_then(|png| image::open(png).ok())
        .map(|i| i.to_rgb8());
    let _ = std::fs::remove_dir_all(&out);
    picture
}

/// Wait for `child` at most `limit`; whether it ended well.
fn wait(mut child: std::process::Child, limit: Duration) -> Option<bool> {
    let start = std::time::Instant::now();
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status.success());
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// Shrink to `side` on the long side (never enlarge).
pub fn fit(image: RgbImage, side: u32) -> RgbImage {
    if image.width().max(image.height()) <= side {
        return image;
    }
    image::DynamicImage::ImageRgb8(image).resize(side, side, FilterType::Triangle).to_rgb8()
}

pub fn jpeg(image: &RgbImage) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(image).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

pub fn png(image: &RgbImage) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

// ── the contact sheet ──

/// The frames on one sheet, in a grid, each with its time in its corner: one
/// picture that shows the whole video at a glance.
pub fn contact_sheet(frames: &[Frame]) -> RgbImage {
    const GAP: u32 = 4;
    let n = frames.len().max(1) as u32;
    let cols = match n {
        0..=4 => 2,
        5..=9 => 3,
        _ => 4,
    };
    let rows = n.div_ceil(cols);
    let cell_w = (LONG_SIDE - GAP * (cols + 1)) / cols;
    let (fw, fh) = frames.first().map_or((16, 9), |f| (f.image.width().max(1), f.image.height().max(1)));
    let cell_h = (cell_w * fh / fw).clamp(cell_w / 3, cell_w * 2);
    let mut sheet = RgbImage::from_pixel(GAP + cols * (cell_w + GAP), GAP + rows * (cell_h + GAP), Rgb([17, 17, 17]));
    for (i, frame) in frames.iter().enumerate() {
        let (col, row) = (i as u32 % cols, i as u32 / cols);
        let small = image::DynamicImage::ImageRgb8(frame.image.clone()).resize(cell_w, cell_h, FilterType::Triangle).to_rgb8();
        let x = GAP + col * (cell_w + GAP) + (cell_w - small.width()) / 2;
        let y = GAP + row * (cell_h + GAP) + (cell_h - small.height()) / 2;
        image::imageops::replace(&mut sheet, &small, i64::from(x), i64::from(y));
        let label = clock(frame.at);
        let scale = 3;
        let (lw, lh) = (text_width(&label) * scale + 12, 7 * scale + 10);
        let (lx, ly) = (GAP + col * (cell_w + GAP) + 6, GAP + row * (cell_h + GAP) + cell_h - lh - 6);
        fill(&mut sheet, lx, ly, lw, lh, Rgb([0, 0, 0]));
        draw_text(&mut sheet, &label, lx + 6, ly + 5, scale, Rgb([255, 255, 255]));
    }
    sheet
}

fn fill(image: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, color: Rgb<u8>) {
    for yy in y..(y + h).min(image.height()) {
        for xx in x..(x + w).min(image.width()) {
            image.put_pixel(xx, yy, color);
        }
    }
}

/// 5×7 digits and a colon: enough for a time, with no font to load.
fn glyph(c: char) -> [u8; 7] {
    match c {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        _ => [0; 7],
    }
}

fn advance(c: char) -> u32 {
    if c == ':' { 4 } else { 6 }
}

fn text_width(text: &str) -> u32 {
    text.chars().map(advance).sum::<u32>().saturating_sub(1)
}

fn draw_text(image: &mut RgbImage, text: &str, x: u32, y: u32, scale: u32, color: Rgb<u8>) {
    let mut at = x;
    for c in text.chars() {
        let (rows, shift) = (glyph(c), if c == ':' { 1 } else { 0 });
        for (r, bits) in rows.iter().enumerate() {
            for col in 0..5u32 {
                if bits & (0x10 >> col) != 0 && col >= shift {
                    fill(image, at + (col - shift) * scale, y + r as u32 * scale, scale, scale, color);
                }
            }
        }
        at += advance(c) * scale;
    }
}

// ── ffmpeg, when AVFoundation can't ──

mod ffmpeg {
    use super::*;

    fn tool(name: &str) -> Option<std::path::PathBuf> {
        ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].iter().map(|dir| Path::new(dir).join(name)).find(|p| p.is_file())
    }

    pub struct Probe {
        pub duration: Option<f64>,
        pub size: Option<(u32, u32)>,
        pub fps: Option<f32>,
    }

    pub fn probe(path: &Path) -> Option<Probe> {
        let out = Command::new(tool("ffprobe")?)
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,r_frame_rate:format=duration", "-of", "default=nw=1"])
            .arg(path)
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key).and_then(|v| v.strip_prefix('='))).map(str::trim);
        let fps = field("r_frame_rate").and_then(|r| {
            let (a, b) = r.split_once('/')?;
            let (a, b) = (a.parse::<f32>().ok()?, b.parse::<f32>().ok()?);
            (b > 0.).then(|| a / b)
        });
        let size = field("width").and_then(|w| w.parse().ok()).zip(field("height").and_then(|h| h.parse().ok()));
        Some(Probe { duration: field("duration").and_then(|d| d.parse().ok()), size, fps })
    }

    pub fn video(path: &Path) -> Result<Video, String> {
        let ffmpeg = tool("ffmpeg").ok_or("no ffmpeg")?;
        let probe = probe(path).ok_or("ffprobe couldn't read it")?;
        let duration = probe.duration.filter(|d| d.is_finite() && *d > 0.).ok_or("no length")?;
        let mut frames = Vec::new();
        for at in frame_times(duration) {
            let out = Command::new(&ffmpeg)
                .args(["-v", "error", "-ss", &format!("{at:.3}"), "-i"])
                .arg(path)
                .args(["-frames:v", "1", "-vf", &format!("scale='min({LONG_SIDE},iw)':-2"), "-f", "image2pipe", "-c:v", "png", "-"])
                .stderr(Stdio::null())
                .output()
                .map_err(|e| e.to_string())?;
            if let Ok(image) = image::load_from_memory(&out.stdout) {
                frames.push(Frame { at, image: fit(image.to_rgb8(), LONG_SIDE) });
            }
        }
        if frames.is_empty() {
            return Err("no frame could be read".into());
        }
        let (width, height) = probe.size.unwrap_or((frames[0].image.width(), frames[0].image.height()));
        Ok(Video { duration, width, height, fps: probe.fps, frames })
    }
}

// ── AVFoundation ──

#[cfg(target_os = "macos")]
mod avf {
    use super::*;
    use objc2::rc::Retained;
    use objc2_av_foundation::{AVAsset, AVAssetImageGenerator, AVMediaTypeVideo};
    use objc2_core_foundation::CGSize;
    use objc2_core_graphics::CGImage;
    use objc2_core_media::CMTime;
    use objc2_foundation::{NSError, NSString, NSURL};

    fn open(path: &Path) -> Option<Retained<AVAsset>> {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
        Some(unsafe { AVAsset::assetWithURL(&url) })
    }

    #[allow(deprecated)]
    fn seconds(asset: &AVAsset) -> Option<f64> {
        let seconds = unsafe { asset.duration().seconds() };
        (seconds.is_finite() && seconds > 0.).then_some(seconds)
    }

    pub fn duration(path: &Path) -> Option<f64> {
        seconds(&*open(path)?)
    }

    #[allow(deprecated)]
    pub fn video(path: &Path) -> Result<Video, String> {
        let asset = open(path).ok_or("unreadable path")?;
        let duration = seconds(&asset).ok_or("no length")?;
        let kind = unsafe { AVMediaTypeVideo }.ok_or("no video type")?;
        let track = unsafe { asset.tracksWithMediaType(kind) }.firstObject().ok_or("no video track")?;
        let natural = unsafe { track.naturalSize() };
        let turn = unsafe { track.preferredTransform() };
        // A phone's portrait video is stored on its side; its transform turns it.
        let width = (turn.a * natural.width + turn.c * natural.height).abs().round() as u32;
        let height = (turn.b * natural.width + turn.d * natural.height).abs().round() as u32;
        let fps = Some(unsafe { track.nominalFrameRate() }).filter(|f| *f > 0.);
        let generator = unsafe { AVAssetImageGenerator::assetImageGeneratorWithAsset(&asset) };
        unsafe {
            generator.setAppliesPreferredTrackTransform(true);
            generator.setMaximumSize(CGSize::new(f64::from(LONG_SIDE), f64::from(LONG_SIDE)));
            // The frame at the time it is labelled with, not the keyframe seconds away.
            let near = CMTime::with_seconds(0.1, 600);
            generator.setRequestedTimeToleranceBefore(near);
            generator.setRequestedTimeToleranceAfter(near);
        }
        let mut frames = Vec::new();
        for at in frame_times(duration) {
            let (tx, rx) = std::sync::mpsc::channel::<Option<RgbImage>>();
            let handler = block2::RcBlock::new(move |image: *mut CGImage, _actual: CMTime, _error: *mut NSError| {
                let picture = unsafe { image.as_ref() }.and_then(super::cg::rgb);
                let _ = tx.send(picture);
            });
            unsafe { generator.generateCGImageAsynchronouslyForTime_completionHandler(CMTime::with_seconds(at, 600), &handler) };
            if let Ok(Some(image)) = rx.recv_timeout(Duration::from_secs(15)) {
                frames.push(Frame { at, image: fit(image, LONG_SIDE) });
            }
        }
        if frames.is_empty() {
            return Err("no frame could be read".into());
        }
        Ok(Video { duration, width, height, fps, frames })
    }
}

// ── CoreGraphics ──

#[cfg(target_os = "macos")]
mod cg {
    use super::*;
    use objc2_core_foundation::{CGPoint, CGRect, CGSize, CFURL};
    use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo, CGPDFBox, CGPDFDocument, CGPDFPage};

    /// Draw into an RGBA buffer of `w`×`h` and keep it as RGB.
    fn render(w: usize, h: usize, draw: impl FnOnce(&CGContext)) -> Option<RgbImage> {
        if w == 0 || h == 0 {
            return None;
        }
        let mut buffer = vec![0u8; w * h * 4];
        let space = CGColorSpace::new_device_rgb()?;
        let context = unsafe { CGBitmapContextCreate(buffer.as_mut_ptr().cast(), w, h, 8, w * 4, Some(&space), CGImageAlphaInfo::PremultipliedLast.0) }?;
        draw(&context);
        drop(context);
        let rgba = image::RgbaImage::from_raw(w as u32, h as u32, buffer)?;
        Some(image::DynamicImage::ImageRgba8(rgba).to_rgb8())
    }

    pub fn rgb(image: &CGImage) -> Option<RgbImage> {
        let (w, h) = (CGImage::width(Some(image)), CGImage::height(Some(image)));
        render(w, h, |context| CGContext::draw_image(Some(context), CGRect::new(CGPoint::new(0., 0.), CGSize::new(w as f64, h as f64)), Some(image)))
    }

    pub fn pdf(path: &Path, side: u32) -> Result<(usize, Option<RgbImage>), String> {
        let url = CFURL::from_file_path(path).ok_or("unreadable path")?;
        let document = CGPDFDocument::with_url(Some(&url)).ok_or("not a PDF")?;
        let pages = CGPDFDocument::number_of_pages(Some(&document));
        if pages == 0 {
            return Err("an empty PDF".into());
        }
        let page = CGPDFDocument::page(Some(&document), 1);
        let first = page.as_deref().and_then(|page| {
            let rect = CGPDFPage::box_rect(Some(page), CGPDFBox::CropBox);
            let turned = CGPDFPage::rotation_angle(Some(page)).rem_euclid(180) == 90;
            let (pw, ph) = if turned { (rect.size.height, rect.size.width) } else { (rect.size.width, rect.size.height) };
            if pw <= 0. || ph <= 0. {
                return None;
            }
            let scale = f64::from(side) / pw.max(ph);
            let (w, h) = ((pw * scale).round() as usize, (ph * scale).round() as usize);
            render(w, h, |context| {
                CGContext::set_rgb_fill_color(Some(context), 1., 1., 1., 1.);
                let all = CGRect::new(CGPoint::new(0., 0.), CGSize::new(w as f64, h as f64));
                CGContext::fill_rect(Some(context), all);
                let transform = CGPDFPage::drawing_transform(Some(page), CGPDFBox::CropBox, all, 0, true);
                CGContext::concat_ctm(Some(context), transform);
                CGContext::draw_pdf_page(Some(context), Some(page));
            })
        });
        Ok((pages, first))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_one_per_ten_seconds_between_four_and_twelve() {
        assert_eq!(frame_count(3.), 4);
        assert_eq!(frame_count(62.), 6);
        assert_eq!(frame_count(102.), 10);
        assert_eq!(frame_count(3600.), 12);
        let times = frame_times(40.);
        assert_eq!(times, vec![5., 15., 25., 35.], "the middles of four parts");
    }

    #[test]
    fn times_read_like_a_player() {
        assert_eq!(clock(3.2), "0:03");
        assert_eq!(clock(102.), "1:42");
        assert_eq!(clock(3723.), "1:02:03");
    }

    #[test]
    fn the_sheet_lays_frames_out_in_a_grid_with_their_times() {
        let frame = |at| Frame { at, image: RgbImage::from_pixel(320, 180, Rgb([200, 30, 30])) };
        let sheet = contact_sheet(&[frame(3.), frame(15.), frame(27.), frame(39.), frame(51.), frame(63.)]);
        assert!(sheet.width() <= LONG_SIDE);
        // Three across, two down, 16:9 cells.
        let cell_w = (LONG_SIDE - 4 * 4) / 3;
        assert_eq!(sheet.height(), 4 + 2 * (cell_w * 9 / 16 + 4));
        // The time's box is dark, with light digits in it.
        let label_y = 4 + cell_w * 9 / 16 - 7 * 3 - 10 - 6 + 5;
        let row: Vec<_> = (10..60).map(|x| *sheet.get_pixel(x, label_y + 3)).collect();
        assert!(row.contains(&Rgb([255, 255, 255])) && row.contains(&Rgb([0, 0, 0])), "{row:?}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_pdf_has_its_pages_counted_and_its_first_page_drawn() {
        // The smallest PDF with two pages: CoreGraphics reads it.
        let dir = std::env::temp_dir().join(format!("equipe-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("two.pdf");
        std::fs::write(&path, two_page_pdf()).unwrap();
        let (pages, first) = pdf(&path, 200).unwrap();
        assert_eq!(pages, 2);
        let first = first.expect("page one drawn");
        assert_eq!(first.width().max(first.height()), 200);
        assert_eq!(*first.get_pixel(5, 5), Rgb([255, 255, 255]), "a blank page is white");
        assert!(pdf(&dir.join("missing.pdf"), 200).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(target_os = "macos")]
    fn two_page_pdf() -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] >>",
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj {body} endobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for at in offsets {
            out.extend(format!("{at:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer << /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
        out
    }
}
