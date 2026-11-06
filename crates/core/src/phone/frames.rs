//! Frames of a recording, to check a flow step by step: where the screen
//! changed and settled, the moments around each action, or times a member
//! asks for. Read with AVFoundation on a Mac, else `ffmpeg`; which moments
//! matter is decided here, from small grey copies of frames sampled across
//! the whole video.

use std::path::{Path, PathBuf};
use std::time::Duration;

use image::{GrayImage, RgbImage, imageops::FilterType};

/// A frame's longest side when it is saved for a member and the person: sharp
/// enough to read a phone's text, small enough to send.
pub const SAVED_SIDE: u32 = 1200;
/// The small copies compared to find changes.
const PROBE_SIDE: u32 = 64;
/// At most this many samples across a video, at least this far apart.
const SAMPLES: usize = 240;
const SAMPLE_EVERY: f64 = 0.25;
/// How different two samples are (mean absolute grey difference, 0–255) for
/// the screen to have changed, and how still for it to have settled.
const CHANGED: f64 = 6.0;
const STILL: f64 = 1.5;

/// The times to sample a video of `duration` seconds at.
pub fn sample_times(duration: f64) -> Vec<f64> {
    if duration <= 0. {
        return Vec::new();
    }
    let step = (duration / SAMPLES as f64).max(SAMPLE_EVERY);
    let n = (duration / step).floor() as usize;
    (0..=n).map(|i| (i as f64 * step).min((duration - 0.05).max(0.))).collect()
}

/// How different two small grey frames are: the mean absolute difference.
pub fn difference(a: &GrayImage, b: &GrayImage) -> f64 {
    if a.dimensions() != b.dimensions() || a.is_empty() {
        return 255.;
    }
    let sum: u64 = a.pixels().zip(b.pixels()).map(|(p, q)| u64::from(p.0[0].abs_diff(q.0[0]))).sum();
    sum as f64 / f64::from(a.width() * a.height())
}

/// A small grey copy, to compare.
pub fn probe(image: &RgbImage) -> GrayImage {
    let gray = image::DynamicImage::ImageRgb8(image.clone()).to_luma8();
    let (w, h) = gray.dimensions();
    let scale = f64::from(PROBE_SIDE) / f64::from(w.max(h).max(1));
    let (nw, nh) = (((f64::from(w) * scale).round() as u32).max(1), ((f64::from(h) * scale).round() as u32).max(1));
    image::imageops::resize(&gray, nw, nh, FilterType::Triangle)
}

/// The moments a flow's screens settled: the first sample, then — each time
/// the screen changed — the first sample after it once it stood still again
/// (or the last before the next change), and the last sample. `samples` are
/// (time, small grey frame), in time order.
pub fn settled(samples: &[(f64, GrayImage)]) -> Vec<f64> {
    let Some(first) = samples.first() else { return Vec::new() };
    let mut out = vec![first.0];
    let mut kept = 0;
    let mut i = 1;
    while i < samples.len() {
        if difference(&samples[i - 1].1, &samples[i].1) < CHANGED {
            i += 1;
            continue;
        }
        // Changing from i: find where it settles.
        let mut j = i;
        while j + 1 < samples.len() && difference(&samples[j].1, &samples[j + 1].1) >= STILL {
            j += 1;
        }
        out.push(samples[j].0);
        kept = j;
        i = j + 1;
    }
    // A slow drift no two samples showed: the end, if it differs from the last kept.
    let end = samples.len() - 1;
    if end > kept + 1 && difference(&samples[kept].1, &samples[end].1) >= CHANGED {
        out.push(samples[end].0);
    }
    out
}

/// An action marked while recording: when, in seconds from its start, and what.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Mark {
    pub at: f64,
    pub what: String,
}

/// Why a frame was kept, for the member reading the list.
#[derive(Debug, Clone, PartialEq)]
pub enum Why {
    Settled,
    Before(String),
    After(String),
    Asked,
}

/// The frames to keep: the settled screens, the moment before each marked
/// action and the screen it settled on after, and any times asked for —
/// in time order, one per half second, at most `max` (the asked and the
/// marked kept first).
pub fn choose(duration: f64, settled: &[f64], marks: &[Mark], asked: &[f64], max: usize) -> Vec<(f64, Why)> {
    let clamp = |t: f64| t.clamp(0., (duration - 0.05).max(0.));
    let mut picked: Vec<(f64, Why, u8)> = Vec::new();
    for t in asked {
        picked.push((clamp(*t), Why::Asked, 0));
    }
    for mark in marks {
        picked.push((clamp(mark.at - 0.15), Why::Before(mark.what.clone()), 1));
        // The screen it led to: the first settled moment after it, else a second on.
        let after = settled.iter().copied().find(|t| *t > mark.at + 0.1).filter(|t| *t < mark.at + 8.).unwrap_or(mark.at + 1.0);
        picked.push((clamp(after), Why::After(mark.what.clone()), 1));
    }
    for t in settled {
        picked.push((clamp(*t), Why::Settled, 2));
    }
    // The most wanted first, then by time: one per half second.
    picked.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.total_cmp(&b.0)));
    let mut kept: Vec<(f64, Why)> = Vec::new();
    for (t, why, _) in picked {
        if kept.len() == max {
            break;
        }
        if kept.iter().any(|(k, _)| (k - t).abs() < 0.5) {
            continue;
        }
        kept.push((t, why));
    }
    kept.sort_by(|a, b| a.0.total_cmp(&b.0));
    kept
}

/// `0:03.2`.
pub fn clock(seconds: f64) -> String {
    let tenths = (seconds.max(0.) * 10.).round() as u64;
    format!("{}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}

/// The frames side by side, numbered by their order, at most four a row:
/// one picture a member reads at a glance.
pub fn sheet(frames: &[RgbImage]) -> Option<RgbImage> {
    let cell_h = 480u32;
    let scaled: Vec<RgbImage> = frames
        .iter()
        .map(|f| {
            let w = (f64::from(f.width()) * f64::from(cell_h) / f64::from(f.height().max(1))).round().max(1.) as u32;
            image::imageops::resize(f, w, cell_h, FilterType::Triangle)
        })
        .collect();
    let cell_w = scaled.iter().map(RgbImage::width).max()?;
    let per_row = scaled.len().min(4) as u32;
    let rows = (scaled.len() as u32).div_ceil(per_row);
    let gap = 12;
    let mut out = RgbImage::from_pixel(per_row * (cell_w + gap) + gap, rows * (cell_h + gap) + gap, image::Rgb([40, 40, 44]));
    for (i, frame) in scaled.iter().enumerate() {
        let (col, row) = (i as u32 % per_row, i as u32 / per_row);
        let (x, y) = (gap + col * (cell_w + gap), gap + row * (cell_h + gap));
        image::imageops::overlay(&mut out, frame, i64::from(x), i64::from(y));
        badge(&mut out, x + 6, y + 6, i + 1);
    }
    Some(out)
}

/// A frame's number in its corner, drawn in blocks: no font needed.
fn badge(img: &mut RgbImage, x: u32, y: u32, n: usize) {
    const DIGITS: [[u8; 5]; 10] = [
        [0b111, 0b101, 0b101, 0b101, 0b111],
        [0b010, 0b110, 0b010, 0b010, 0b111],
        [0b111, 0b001, 0b111, 0b100, 0b111],
        [0b111, 0b001, 0b111, 0b001, 0b111],
        [0b101, 0b101, 0b111, 0b001, 0b001],
        [0b111, 0b100, 0b111, 0b001, 0b111],
        [0b111, 0b100, 0b111, 0b101, 0b111],
        [0b111, 0b001, 0b010, 0b010, 0b010],
        [0b111, 0b101, 0b111, 0b101, 0b111],
        [0b111, 0b101, 0b111, 0b001, 0b111],
    ];
    let digits: Vec<usize> = n.to_string().bytes().map(|b| usize::from(b - b'0')).collect();
    let px = 5u32;
    let (w, h) = (digits.len() as u32 * 4 * px + px, 7 * px);
    for dy in 0..h {
        for dx in 0..w {
            if x + dx < img.width() && y + dy < img.height() {
                img.put_pixel(x + dx, y + dy, image::Rgb([20, 20, 20]));
            }
        }
    }
    for (i, d) in digits.iter().enumerate() {
        for (row, bits) in DIGITS[*d].iter().enumerate() {
            for col in 0..3u32 {
                if bits >> (2 - col) & 1 == 1 {
                    for oy in 0..px {
                        for ox in 0..px {
                            let (cx, cy) = (x + px + i as u32 * 4 * px + col * px + ox, y + px + row as u32 * px + oy);
                            if cx < img.width() && cy < img.height() {
                                img.put_pixel(cx, cy, image::Rgb([255, 255, 255]));
                            }
                        }
                    }
                }
            }
        }
    }
}

/// `image` no larger than `side` on its longest side.
pub fn fit(image: RgbImage, side: u32) -> RgbImage {
    let (w, h) = image.dimensions();
    if w.max(h) <= side {
        return image;
    }
    let scale = f64::from(side) / f64::from(w.max(h));
    image::imageops::resize(&image, ((f64::from(w) * scale).round() as u32).max(1), ((f64::from(h) * scale).round() as u32).max(1), FilterType::Triangle)
}

/// Save `image` as a JPEG.
pub fn save_jpeg(image: &RgbImage, path: &Path) -> Result<(), String> {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 82).encode_image(image).map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| format!("{} couldn't be written: {e}", path.display()))
}

/// JPEG bytes of `image`, for a tool's answer.
pub fn jpeg(image: &RgbImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    let _ = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 78).encode_image(image);
    bytes
}

// ── reading a video ──

/// A video's length in seconds.
pub fn duration(video: &Path) -> Option<f64> {
    #[cfg(target_os = "macos")]
    if let Some(d) = avf::duration(video) {
        return Some(d);
    }
    ffmpeg::duration(video)
}

/// The frames at `times` (each `None` where none could be read), no larger than `side`.
pub fn grab(video: &Path, times: &[f64], side: u32) -> Result<Vec<Option<RgbImage>>, String> {
    #[cfg(target_os = "macos")]
    if let Ok(frames) = avf::grab(video, times, side)
        && frames.iter().any(Option::is_some)
    {
        return Ok(frames);
    }
    ffmpeg::grab(video, times, side)
}

/// Where `ffmpeg`/`ffprobe` are, if installed.
pub fn tool(name: &str) -> Option<PathBuf> {
    ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].iter().map(|dir| Path::new(dir).join(name)).find(|p| p.is_file())
}

mod ffmpeg {
    use super::*;
    use std::process::{Command, Stdio};

    pub fn duration(video: &Path) -> Option<f64> {
        let out = Command::new(tool("ffprobe")?)
            .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"])
            .arg(video)
            .stderr(Stdio::null())
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok().filter(|d: &f64| d.is_finite() && *d > 0.)
    }

    pub fn grab(video: &Path, times: &[f64], side: u32) -> Result<Vec<Option<RgbImage>>, String> {
        let ffmpeg = tool("ffmpeg").ok_or("Neither AVFoundation nor ffmpeg could read the recording; install ffmpeg to read frames.")?;
        Ok(times
            .iter()
            .map(|at| {
                let out = Command::new(&ffmpeg)
                    .args(["-v", "error", "-ss", &format!("{at:.3}"), "-i"])
                    .arg(video)
                    .args(["-frames:v", "1", "-vf", &format!("scale='min({side},iw)':-2"), "-f", "image2pipe", "-c:v", "png", "-"])
                    .stderr(Stdio::null())
                    .output()
                    .ok()?;
                image::load_from_memory(&out.stdout).ok().map(|i| fit(i.to_rgb8(), side))
            })
            .collect())
    }
}

#[cfg(target_os = "macos")]
mod avf {
    use super::*;
    use objc2::rc::Retained;
    use objc2_av_foundation::{AVAsset, AVAssetImageGenerator};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo};
    use objc2_core_media::CMTime;
    use objc2_foundation::{NSError, NSString, NSURL};

    fn open(path: &Path) -> Option<Retained<AVAsset>> {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
        Some(unsafe { AVAsset::assetWithURL(&url) })
    }

    #[allow(deprecated)]
    pub fn duration(path: &Path) -> Option<f64> {
        let seconds = unsafe { open(path)?.duration().seconds() };
        (seconds.is_finite() && seconds > 0.).then_some(seconds)
    }

    fn rgb(image: &CGImage) -> Option<RgbImage> {
        let (w, h) = (CGImage::width(Some(image)), CGImage::height(Some(image)));
        if w == 0 || h == 0 {
            return None;
        }
        let mut buffer = vec![0u8; w * h * 4];
        let space = CGColorSpace::new_device_rgb()?;
        let context = unsafe { CGBitmapContextCreate(buffer.as_mut_ptr().cast(), w, h, 8, w * 4, Some(&space), CGImageAlphaInfo::PremultipliedLast.0) }?;
        CGContext::draw_image(Some(&context), CGRect::new(CGPoint::new(0., 0.), CGSize::new(w as f64, h as f64)), Some(image));
        drop(context);
        let rgba = image::RgbaImage::from_raw(w as u32, h as u32, buffer)?;
        Some(image::DynamicImage::ImageRgba8(rgba).to_rgb8())
    }

    pub fn grab(path: &Path, times: &[f64], side: u32) -> Result<Vec<Option<RgbImage>>, String> {
        let asset = open(path).ok_or("unreadable")?;
        let generator = unsafe { AVAssetImageGenerator::assetImageGeneratorWithAsset(&asset) };
        unsafe {
            generator.setAppliesPreferredTrackTransform(true);
            generator.setMaximumSize(CGSize::new(f64::from(side), f64::from(side)));
            // The frame at the time asked, not the keyframe seconds away.
            let near = CMTime::with_seconds(0.04, 600);
            generator.setRequestedTimeToleranceBefore(near);
            generator.setRequestedTimeToleranceAfter(near);
        }
        Ok(times
            .iter()
            .map(|at| {
                let (tx, rx) = std::sync::mpsc::channel::<Option<RgbImage>>();
                let handler = block2::RcBlock::new(move |image: *mut CGImage, _actual: CMTime, _error: *mut NSError| {
                    let _ = tx.send(unsafe { image.as_ref() }.and_then(rgb));
                });
                unsafe { generator.generateCGImageAsynchronouslyForTime_completionHandler(CMTime::with_seconds(*at, 600), &handler) };
                rx.recv_timeout(Duration::from_secs(15)).ok().flatten().map(|i| fit(i, side))
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(v: u8) -> GrayImage {
        GrayImage::from_pixel(8, 16, image::Luma([v]))
    }

    #[test]
    fn samples_cover_the_video_and_stay_few() {
        let short = sample_times(2.0);
        assert_eq!(short.len(), 9, "every quarter second: {short:?}");
        assert!(short.last().unwrap() < &2.0);
        assert!(sample_times(600.).len() <= 241);
        assert!(sample_times(0.).is_empty());
    }

    #[test]
    fn a_screen_counts_once_it_settles_after_a_change() {
        // Home (0–1 s), a transition (1.25–1.5), the next screen settled from 1.75; a spinner flicker is not a screen.
        let frames = [(0.0, 10), (0.25, 10), (0.5, 10), (0.75, 10), (1.0, 10), (1.25, 60), (1.5, 120), (1.75, 130), (2.0, 130), (2.25, 131), (2.5, 131)];
        let samples: Vec<(f64, GrayImage)> = frames.iter().map(|(t, v)| (*t, flat(*v))).collect();
        assert_eq!(settled(&samples), vec![0.0, 1.75]);
        assert!(settled(&[]).is_empty());
        let still: Vec<(f64, GrayImage)> = (0..5).map(|i| (f64::from(i) * 0.25, flat(50))).collect();
        assert_eq!(settled(&still), vec![0.0], "nothing happened: one frame");
    }

    #[test]
    fn frames_around_each_action_come_first_in_time_order() {
        let marks = [Mark { at: 3.0, what: "tap \"Pay\"".into() }];
        let picked = choose(10., &[0.0, 3.6, 7.0], &marks, &[9.5], 12);
        let times: Vec<f64> = picked.iter().map(|(t, _)| *t).collect();
        assert_eq!(times, vec![0.0, 2.85, 3.6, 7.0, 9.5]);
        assert_eq!(picked[1].1, Why::Before("tap \"Pay\"".into()));
        assert_eq!(picked[2].1, Why::After("tap \"Pay\"".into()));
        let few = choose(10., &[0.0, 3.6, 7.0], &marks, &[], 2);
        assert_eq!(few.iter().map(|(t, _)| *t).collect::<Vec<_>>(), vec![2.85, 3.6], "the action's frames before the rest");
    }

    #[test]
    fn a_sheet_numbers_its_frames_and_a_clock_reads_tenths() {
        let frame = RgbImage::from_pixel(402, 874, image::Rgb([250, 250, 250]));
        let sheet = sheet(&[frame.clone(), frame.clone(), frame.clone(), frame.clone(), frame]).unwrap();
        assert_eq!(sheet.height(), 2 * (480 + 12) + 12, "two rows of four");
        assert_eq!(sheet.get_pixel(12 + 6 + 2, 12 + 6 + 2).0, [20, 20, 20], "the badge");
        assert_eq!(clock(63.27), "1:03.3");
        assert_eq!(clock(0.0), "0:00.0");
    }
}
