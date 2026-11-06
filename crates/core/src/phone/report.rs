//! A member's report of a flow it tested on a phone: each step with what it
//! expected, what it saw, its verdict and the frame that shows it, beside the
//! recording. Kept as `<flow>.report.json` next to the video, and told to the
//! person as a note in the conversation — its first line in their language,
//! the steps as the member wrote them, the key frames as pictures — which the
//! desktop draws as a card and every other screen shows as text and pictures.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A step's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Fail,
    /// It couldn't be checked (the app crashed before it, a blocker): never a pass.
    Blocked,
}

impl Verdict {
    /// The mark a step line starts with.
    pub fn mark(self) -> char {
        match self {
            Verdict::Pass => '✓',
            Verdict::Fail => '✗',
            Verdict::Blocked => '–',
        }
    }

    pub fn of_mark(c: char) -> Option<Self> {
        [Verdict::Pass, Verdict::Fail, Verdict::Blocked].into_iter().find(|v| v.mark() == c)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub step: String,
    pub expected: String,
    pub seen: String,
    pub verdict: Verdict,
    /// The frame that shows it, by absolute path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub flow: String,
    pub ticket: u32,
    pub member: String,
    pub recording: PathBuf,
    /// The recording's length in seconds, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    pub steps: Vec<Step>,
}

impl Report {
    pub fn passed(&self) -> usize {
        self.steps.iter().filter(|s| s.verdict == Verdict::Pass).count()
    }

    pub fn verdict(&self) -> Verdict {
        if self.steps.iter().any(|s| s.verdict == Verdict::Fail) {
            Verdict::Fail
        } else if self.steps.iter().any(|s| s.verdict == Verdict::Blocked) {
            Verdict::Blocked
        } else {
            Verdict::Pass
        }
    }
}

/// How many key frames the note shows.
pub const NOTE_FRAMES: usize = 6;

/// A report from a tool's arguments: every step with a verdict, what it
/// expected and saw, and — for a pass or a fail — the frame that shows it,
/// saved by `frames` under `recordings`. `recording` is resolved there too.
pub fn from_args(args: &Value, recordings: &Path, ticket: u32, member: &str) -> Result<Report, String> {
    let flow = args["flow"].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or("Name the flow you tested (`flow`).")?.to_owned();
    let recording = args["recording"].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or("Name the recording the report is about (`recording`), as `record` gave it.")?;
    let recording = resolve(recordings, recording, "mp4").ok_or_else(|| format!("There's no recording {recording} for T-{ticket}. Record the flow with `record` first."))?;
    let raw = args["steps"].as_array().filter(|s| !s.is_empty()).ok_or("List the flow's steps (`steps`), each with what you expected, what you saw, a verdict and its frame.")?;
    let mut steps = Vec::new();
    for (i, s) in raw.iter().enumerate() {
        let n = i + 1;
        let field = |key: &str| s[key].as_str().map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned);
        let step = field("step").ok_or(format!("Step {n} has no `step`: what was done."))?;
        let expected = field("expected").ok_or(format!("Step {n} ({step}) has no `expected`: what the screen should show."))?;
        let seen = field("seen").ok_or(format!("Step {n} ({step}) has no `seen`: what the frame shows."))?;
        let verdict: Verdict = serde_json::from_value(s["verdict"].clone()).map_err(|_| format!("Step {n} ({step}) needs a `verdict`: pass, fail or blocked."))?;
        let frame = match field("frame") {
            Some(f) => Some(resolve(recordings, &f, "jpg").ok_or_else(|| format!("Step {n}'s frame {f} isn't one of T-{ticket}'s frames: give a path `frames` returned."))?),
            None if verdict == Verdict::Blocked => None,
            None => return Err(format!("Step {n} ({step}) is a {} without a frame: a verdict stands on the frame that shows it. Take it with `frames`.", if verdict == Verdict::Pass { "pass" } else { "fail" })),
        };
        steps.push(Step { step, expected, seen, verdict, frame });
    }
    let seconds = crate::phone::frames::duration(&recording);
    Ok(Report { flow, ticket, member: member.to_owned(), recording, seconds, steps })
}

/// `name` as a file under `dir`: an absolute path inside it, or a name there
/// (with `ext` added when it has none). Nothing outside `dir`, nothing missing.
pub fn resolve(dir: &Path, name: &str, ext: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    let path = if path.is_absolute() {
        path.to_owned()
    } else if path.extension().is_none() {
        dir.join(format!("{name}.{ext}"))
    } else {
        dir.join(name)
    };
    let real = path.canonicalize().ok()?;
    let root = dir.canonicalize().ok()?;
    (real.starts_with(&root) && real.is_file()).then_some(real)
}

/// The note's first line's arguments, for a language's `notice.qa_report`.
pub fn head_args(report: &Report) -> Vec<(&'static str, String)> {
    vec![
        ("name", report.member.clone()),
        ("flow", report.flow.clone()),
        ("ticket", report.ticket.to_string()),
        ("passed", report.passed().to_string()),
        ("steps", report.steps.len().to_string()),
        ("video", report.recording.to_string_lossy().into_owned()),
    ]
}

/// The note under its first line `head`: a line per step (`✓ 2. Tap Pay — what was seen`),
/// then the frames of the failed steps and the rest, as pictures, at most [`NOTE_FRAMES`].
pub fn note(head: &str, report: &Report) -> String {
    let mut text = head.trim().to_owned();
    for (i, s) in report.steps.iter().enumerate() {
        text.push_str(&format!("\n{} {}. {} — {}", s.verdict.mark(), i + 1, one_line(&s.step), one_line(&s.seen)));
    }
    let mut framed: Vec<(usize, &Step)> = report.steps.iter().enumerate().filter(|(_, s)| s.frame.is_some()).collect();
    // The failures first, then in order; shown in order.
    framed.sort_by_key(|(i, s)| (s.verdict != Verdict::Fail, *i));
    framed.truncate(NOTE_FRAMES);
    framed.sort_by_key(|(i, _)| *i);
    for (i, s) in framed {
        if let Some(frame) = &s.frame {
            text.push_str(&format!("\n![{} {}. {}]({})", s.verdict.mark(), i + 1, one_line(&s.step).replace([']', '['], ""), frame.display()));
        }
    }
    text
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A step line of a note, read back: its verdict, number, step and what was seen.
pub fn step_line(line: &str) -> Option<(Verdict, usize, String, String)> {
    let mut chars = line.trim().chars();
    let verdict = Verdict::of_mark(chars.next()?)?;
    let rest = chars.as_str().trim_start();
    let (n, rest) = rest.split_once(". ")?;
    let n = n.parse().ok()?;
    let (step, seen) = rest.split_once(" — ").unwrap_or((rest, ""));
    Some((verdict, n, step.to_owned(), seen.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-phone-report-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("checkout")).unwrap();
        std::fs::write(dir.join("checkout.mp4"), b"not really").unwrap();
        std::fs::write(dir.join("checkout/0001.jpg"), b"x").unwrap();
        std::fs::write(dir.join("checkout/0004.jpg"), b"x").unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn a_report_stands_on_its_frames() {
        let dir = dir();
        let args = json!({"flow": "Checkout", "recording": "checkout", "steps": [
            {"step": "Open the app", "expected": "Cart (0)", "seen": "Cart (0)", "verdict": "pass", "frame": "checkout/0001.jpg"},
            {"step": "Tap Pay", "expected": "Thank you", "seen": "Error 500", "verdict": "fail", "frame": dir.join("checkout/0004.jpg").to_string_lossy()},
            {"step": "Receipt mail", "expected": "a mail", "seen": "couldn't get there", "verdict": "blocked"}
        ]});
        let report = from_args(&args, &dir, 12, "qa").unwrap();
        assert_eq!((report.passed(), report.verdict()), (1, Verdict::Fail));
        assert_eq!(report.recording, dir.join("checkout.mp4"));
        assert_eq!(report.steps[0].frame.as_deref(), Some(dir.join("checkout/0001.jpg").as_path()));

        let mut no_frame = args.clone();
        no_frame["steps"][0]["frame"] = Value::Null;
        assert!(from_args(&no_frame, &dir, 12, "qa").unwrap_err().contains("without a frame"));
        let mut elsewhere = args.clone();
        elsewhere["steps"][0]["frame"] = json!("/etc/hosts");
        assert!(from_args(&elsewhere, &dir, 12, "qa").unwrap_err().contains("isn't one of T-12's frames"));
        let mut escape = args.clone();
        escape["steps"][0]["frame"] = json!("../../../etc/hosts");
        assert!(from_args(&escape, &dir, 12, "qa").is_err());
        let mut no_video = args;
        no_video["recording"] = json!("other");
        assert!(from_args(&no_video, &dir, 12, "qa").unwrap_err().contains("no recording other"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_note_lists_steps_then_frames_and_reads_back() {
        let report = Report {
            flow: "Checkout".into(),
            ticket: 12,
            member: "qa".into(),
            recording: "/h/recordings/T-12/checkout.mp4".into(),
            seconds: Some(12.),
            steps: vec![
                Step { step: "Open the app".into(), expected: "x".into(), seen: "Cart (0)".into(), verdict: Verdict::Pass, frame: Some("/h/f/1.jpg".into()) },
                Step { step: "Tap [Pay]".into(), expected: "x".into(), seen: "Error\n500".into(), verdict: Verdict::Fail, frame: Some("/h/f/4.jpg".into()) },
                Step { step: "Mail".into(), expected: "x".into(), seen: "-".into(), verdict: Verdict::Blocked, frame: None },
            ],
        };
        let note = note("HEAD", &report);
        assert_eq!(
            note,
            "HEAD\n✓ 1. Open the app — Cart (0)\n✗ 2. Tap [Pay] — Error 500\n– 3. Mail — -\n![✓ 1. Open the app](/h/f/1.jpg)\n![✗ 2. Tap Pay](/h/f/4.jpg)"
        );
        assert_eq!(step_line("✗ 2. Tap [Pay] — Error 500"), Some((Verdict::Fail, 2, "Tap [Pay]".into(), "Error 500".into())));
        assert_eq!(step_line("just words"), None);
        let images = crate::visuals::markdown::local_images(&note);
        assert_eq!(images.len(), 2, "every screen finds the frames as pictures");
    }
}
