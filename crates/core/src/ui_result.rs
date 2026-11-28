//! Finished work that changes a screen is shown to the person, never only
//! described. When a ticket that changed what users see is accepted (by the
//! PM, by its own member landing it, or after its checks), equipe posts its
//! result in the team's conversation itself — no model turn spent on it:
//!
//! - screenshots of the built screens, reused from its hand-in and design QA
//!   (the engineer's, the designer's, the precheck's): desktop and mobile,
//!   light and dark where it has both ([`pick`]);
//! - a short recording of the flow, when one was made: the newest
//!   walkthrough of the flow map in its worktree ([`walkthrough`]); one too
//!   big to send is made smaller with ffmpeg when it is installed, and left
//!   out otherwise;
//! - its live or preview address: one its hand-in names, else the site its
//!   project names ([`link`]).
//!
//! Each file is copied into the daemon's uploads and named in the message by
//! its attachment tag (`[image: /…/uploads/…png]`, `[video: …mp4]`), the
//! same tags a message the person sends carries, so every screen that draws
//! those draws these — and they are kept like a sent message's files
//! (`crate::kept`), never swept as a draft.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::protocol::rpc::UploadKind;
use crate::protocol::{Ticket, TicketId, TicketKind};

/// How many screenshots a result shows at most.
pub const SHOTS: usize = 4;
/// The largest screenshot copied.
pub const SHOT_MAX: u64 = 8 << 20;
/// The largest recording sent as it is: what a message may carry.
pub const VIDEO_MAX: u64 = equipe_protocol::ATTACHMENT_MAX_BYTES as u64;
/// The longest a recording made smaller runs.
const VIDEO_SECS: u32 = 30;
/// The roles whose code tickets change what users see.
pub const UI_ROLES: [&str; 3] = ["engineer", "designer", "mobile"];
/// Hosts whose links are code or a conversation, not the product.
const NOT_THE_PRODUCT: [&str; 6] = ["github.com", "gitlab.com", "bitbucket.org", "linear.app", "figma.com", "localhost"];

/// Whether accepting `ticket` finishes work that changed what users see: a
/// code ticket that went through design QA (`design_qa_rounds` of them), or
/// one a screen-building role did.
pub fn is_ui_change(ticket: &Ticket, design_qa_rounds: u32) -> bool {
    if ticket.kind != TicketKind::Code {
        return false;
    }
    let by_ui_role = [ticket.assignee.as_deref(), ticket.owner.as_deref()].into_iter().flatten().any(|who| UI_ROLES.contains(&crate::board::role_of(who)));
    design_qa_rounds > 0 || ticket.design_qa.is_some() || by_ui_role
}

/// How a screenshot was taken, read from its file name
/// (`login/default.dark.390.normal.png`, `home-1280-dark.png`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Look {
    /// Its window's width, when the name says.
    width: Option<u32>,
    dark: bool,
    /// Text at 200% or the page zoomed: a check, not the result.
    enlarged: bool,
    /// A crop, a baseline or a "before": not the result either.
    aside: bool,
}

fn look(path: &str) -> Look {
    let name = Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or(path).to_ascii_lowercase();
    let words: Vec<&str> = name.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |w: &str| words.contains(&w);
    let width = words.iter().filter_map(|w| w.parse::<u32>().ok()).find(|n| (280..=3840).contains(n)).or_else(|| {
        if has("mobile") || has("phone") || has("iphone") {
            Some(390)
        } else if has("desktop") {
            Some(1280)
        } else {
            None
        }
    });
    Look {
        width,
        dark: has("dark"),
        enlarged: has("zoom") || (has("text") && has("200")),
        aside: ["finding", "change", "crop", "baseline", "base", "before", "diff"].iter().any(|w| words.first() == Some(w)) || has("baseline") || has("before"),
    }
}

/// Of a hand-in's pictures, those that show its result best, at most
/// [`SHOTS`]: desktop and mobile, light and dark — one of each that it has,
/// in that order — then the rest as they came. Checks at large text, crops
/// and "before" pictures only when nothing else is there.
pub fn pick(images: &[String]) -> Vec<String> {
    let looks: Vec<(&String, Look)> = images.iter().map(|p| (p, look(p))).collect();
    let result: Vec<&(&String, Look)> = looks.iter().filter(|(_, l)| !l.enlarged && !l.aside).collect();
    let mut out: Vec<String> = Vec::new();
    // Desktop light, mobile light, desktop dark, mobile dark.
    for (desktop, dark) in [(true, false), (false, false), (true, true), (false, true)] {
        let fits = |w: u32| if desktop { w >= 1024 } else { w < 768 };
        if let Some((p, _)) = result.iter().find(|(p, l)| l.dark == dark && l.width.is_some_and(fits) && !out.contains(p)) {
            out.push((*p).clone());
        }
    }
    for (p, _) in &result {
        if out.len() >= SHOTS {
            break;
        }
        if !out.contains(p) {
            out.push((*p).clone());
        }
    }
    if out.is_empty()
        && let Some((p, _)) = looks.first()
    {
        out.push((*p).clone());
    }
    out.truncate(SHOTS);
    out
}

/// The address the person opens to see the work: the first web address in
/// its hand-in (`note`) that is the product — not code, not this machine —
/// else the site its project names.
pub fn link(note: Option<&str>, site: Option<&str>) -> Option<String> {
    let product = |url: &str| {
        let rest = url.strip_prefix("https://")?;
        let host = rest.split(['/', '?', '#']).next()?.split(':').next()?.to_ascii_lowercase();
        let local = host.starts_with("127.") || host == "0.0.0.0" || host.ends_with(".local") || !host.contains('.');
        let code = NOT_THE_PRODUCT.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")));
        (!local && !code).then(|| url.to_owned())
    };
    let in_note = note.into_iter().flat_map(|n| n.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>' | '"' | '\'' | '`' | '[' | ']'))).find_map(|word| {
        let url = word.trim_end_matches(['.', ',', ';', ':', '!', '?']);
        product(url)
    });
    in_note.or_else(|| site.map(str::trim).filter(|s| s.starts_with("https://") || s.starts_with("http://")).map(str::to_owned))
}

/// The newest walkthrough recorded in `worktree`'s flow map
/// (`design/flows/<flow>/walkthrough.<mode>.<width>.mp4`) since `since`.
pub fn walkthrough(worktree: &Path, since: SystemTime) -> Option<PathBuf> {
    let flows = worktree.join("design").join("flows");
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for flow in std::fs::read_dir(&flows).ok()?.flatten().take(200) {
        let Ok(files) = std::fs::read_dir(flow.path()) else { continue };
        for file in files.flatten().take(200) {
            let name = file.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("walkthrough.") && name.ends_with(".mp4")) {
                continue;
            }
            // A plain file, not a link out of the worktree.
            let Ok(meta) = std::fs::symlink_metadata(file.path()) else { continue };
            let Ok(at) = meta.modified() else { continue };
            if meta.is_file() && at >= since && best.as_ref().is_none_or(|(b, _)| at > *b) {
                best = Some((at, file.path()));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// What a result is put together from, read on the loop.
#[derive(Debug, Clone)]
pub struct Inputs {
    pub squad: String,
    pub ticket: TicketId,
    pub title: String,
    /// The pictures picked ([`pick`]), by the paths the ticket names them by.
    pub shots: Vec<String>,
    pub worktree: Option<PathBuf>,
    pub link: Option<String>,
}

impl Inputs {
    /// What `ticket`'s result is made of, when it changed what users see.
    pub fn of(squad: &str, ticket: &Ticket, design_qa_rounds: u32, site: Option<&str>) -> Option<Inputs> {
        if !is_ui_change(ticket, design_qa_rounds) {
            return None;
        }
        Some(Inputs {
            squad: squad.to_owned(),
            ticket: ticket.id,
            title: ticket.title.clone(),
            shots: pick(&ticket.images),
            worktree: ticket.worktree.clone(),
            link: link(ticket.note.as_deref(), site),
        })
    }
}

/// A result ready to show: its files in the uploads, by their tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub ticket: TicketId,
    pub title: String,
    /// `[image: …]` and `[video: …]`, the screenshots first.
    pub tags: Vec<String>,
    pub shots: usize,
    pub video: bool,
    pub link: Option<String>,
}

impl Shown {
    /// The message the person reads: `headline`, the files one to a line,
    /// then `link_line` (the link said in their language) when there is a link.
    pub fn message(&self, headline: &str, link_line: Option<&str>) -> String {
        let mut out = vec![headline.trim().to_owned()];
        out.extend(self.tags.iter().cloned());
        if let (Some(_), Some(line)) = (&self.link, link_line) {
            out.push(line.trim().to_owned());
        }
        out.join("\n")
    }

    /// What the one who accepted it hears: the person sees it already.
    pub fn told(&self) -> String {
        let mut parts = Vec::new();
        if self.shots > 0 {
            parts.push(format!("{} screenshot{}", self.shots, if self.shots == 1 { "" } else { "s" }));
        }
        if self.video {
            parts.push("a recording of the flow".to_owned());
        }
        if let Some(link) = &self.link {
            parts.push(format!("its link ({link})"));
        }
        let mut line = format!(
            "The person sees T-{}'s result in the team chat now: {}. Don't paste them again; say in a line or two what changed for them.",
            self.ticket,
            parts.join(", ")
        );
        if self.link.is_none() {
            line.push_str(" No live or preview link was found: if the work is up somewhere, give its address in your reply.");
        }
        line
    }
}

/// Copy `inputs`' files into `home`'s uploads and keep them; `None` when
/// there is nothing to show. Blocking; with `slow` (off the loop) a
/// recording too big to send is made smaller with ffmpeg when it is installed.
pub fn stage(home: &Path, inputs: &Inputs, slow: bool) -> Option<Shown> {
    let mut tags = Vec::new();
    let mut newest: Option<SystemTime> = None;
    for shot in &inputs.shots {
        let Some(source) = source_of(home, inputs, shot) else { continue };
        let Ok(meta) = std::fs::metadata(&source) else { continue };
        if !meta.is_file() || meta.len() > SHOT_MAX {
            continue;
        }
        if let Some(path) = copy_in(home, &source, UploadKind::Image) {
            newest = newest.max(meta.modified().ok());
            tags.push(format!("[image: {}]", path.display()));
        }
    }
    let shots = tags.len();
    // A recording made with the screenshots, or lately when there are none.
    let since = newest.map_or_else(|| SystemTime::now() - Duration::from_secs(24 * 3600), |at| at - Duration::from_secs(6 * 3600));
    let video = inputs.worktree.as_deref().and_then(|w| walkthrough(w, since)).and_then(|v| video_in(home, &v, slow));
    if let Some(path) = &video {
        tags.push(format!("[video: {}]", path.display()));
    }
    if tags.is_empty() && inputs.link.is_none() {
        return None;
    }
    crate::kept::keep(home, &tags.join("\n"));
    Some(Shown { ticket: inputs.ticket, title: inputs.title.clone(), tags, shots, video: video.is_some(), link: inputs.link.clone() })
}

/// Where a picture the ticket names is now: where it was saved, or the copy
/// kept when it was handed in (`crate::visuals::keep`) once its worktree went.
fn source_of(home: &Path, inputs: &Inputs, shot: &str) -> Option<PathBuf> {
    let path = PathBuf::from(shot);
    if path.is_absolute() && path.is_file() {
        return Some(path);
    }
    if let Some(worktree) = &inputs.worktree
        && !path.is_absolute()
        && worktree.join(&path).is_file()
    {
        return Some(worktree.join(path));
    }
    crate::visuals::keep::copy_of(home, &inputs.squad, &[inputs.ticket], false, shot)
}

/// A copy of `source` in the uploads, named for it.
fn copy_in(home: &Path, source: &Path, kind: UploadKind) -> Option<PathBuf> {
    let name = source.file_name()?.to_string_lossy().into_owned();
    let (file, path) = crate::gateway::create_upload(home, &name, Some(kind)).ok()?;
    drop(file);
    match std::fs::copy(source, &path) {
        Ok(_) => Some(path),
        Err(_) => {
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}

/// The recording at `source` in the uploads: as it is when small enough,
/// else made smaller with ffmpeg (`slow` only), else none.
fn video_in(home: &Path, source: &Path, slow: bool) -> Option<PathBuf> {
    let size = std::fs::metadata(source).ok()?.len();
    if size <= VIDEO_MAX {
        return copy_in(home, source, UploadKind::Video);
    }
    if !slow {
        return None;
    }
    let ffmpeg = ffmpeg()?;
    let (file, out) = crate::gateway::create_upload(home, "walkthrough.mp4", Some(UploadKind::Video)).ok()?;
    drop(file);
    let made = std::process::Command::new(ffmpeg)
        .args(shrink_args(source, &out))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if made && std::fs::metadata(&out).is_ok_and(|m| m.len() > 0 && m.len() <= VIDEO_MAX) {
        return Some(out);
    }
    let _ = std::fs::remove_file(&out);
    None
}

/// ffmpeg's arguments to make `source` a small MP4 at `out`: its first
/// [`VIDEO_SECS`] seconds, at most 960 px wide, no sound, playable as it loads.
fn shrink_args(source: &Path, out: &Path) -> Vec<String> {
    let mut args: Vec<String> = ["-y", "-hide_banner", "-loglevel", "error", "-i"].map(str::to_owned).to_vec();
    args.push(source.to_string_lossy().into_owned());
    args.extend(
        ["-t", &VIDEO_SECS.to_string(), "-vf", "scale='min(960,iw)':-2", "-an", "-c:v", "libx264", "-preset", "veryfast", "-crf", "32", "-pix_fmt", "yuv420p", "-movflags", "+faststart", "-f", "mp4"]
            .map(str::to_owned),
    );
    args.push(out.to_string_lossy().into_owned());
    args
}

/// ffmpeg, when it is installed.
fn ffmpeg() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).chain(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from)).map(|d| d.join("ffmpeg")).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::TicketStatus;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-ui-result-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn s(list: &[&str]) -> Vec<String> {
        list.iter().map(|p| (*p).to_owned()).collect()
    }

    fn ticket(owner: &str) -> Ticket {
        serde_json::from_value(serde_json::json!({
            "id": 12, "title": "Sign-in page", "body": "", "status": TicketStatus::Review, "owner": owner, "assignee": owner,
            "project": "/p", "depends_on": [], "branch": "b", "created_by": "pm", "note": null
        }))
        .unwrap()
    }

    #[test]
    fn a_result_shows_desktop_and_mobile_in_light_and_dark_first() {
        let images = s(&[
            "/q/shots/login/default.light.390.text-200.png",
            "/q/shots/login/default.light.1280.normal.png",
            "/q/shots/login/default.dark.1280.normal.png",
            "/q/shots/login/default.light.768.normal.png",
            "/q/finding-1.png",
            "/q/shots/login/default.dark.390.normal.png",
            "/q/shots/login/default.light.390.normal.png",
            "/q/shots/login/error.light.1280.normal.png",
        ]);
        assert_eq!(
            pick(&images),
            s(&["/q/shots/login/default.light.1280.normal.png", "/q/shots/login/default.light.390.normal.png", "/q/shots/login/default.dark.1280.normal.png", "/q/shots/login/default.dark.390.normal.png"])
        );
        // Names a member gives: `-mobile`, `-1440-dark`.
        let named = s(&["/w/handin-shots/home-mobile.png", "/w/handin-shots/home-1440.png", "/w/handin-shots/home-1440-dark.png"]);
        assert_eq!(pick(&named), s(&["/w/handin-shots/home-1440.png", "/w/handin-shots/home-mobile.png", "/w/handin-shots/home-1440-dark.png"]));
        // Nothing to tell them by: the first ones, as they came.
        assert_eq!(pick(&s(&["/w/a.png", "/w/b.png", "/w/c.png", "/w/d.png", "/w/e.png"])), s(&["/w/a.png", "/w/b.png", "/w/c.png", "/w/d.png"]));
        // Only checks and crops: still something to see.
        assert_eq!(pick(&s(&["/q/finding-1.png", "/q/finding-2.png"])), s(&["/q/finding-1.png"]));
        assert!(pick(&[]).is_empty());
    }

    #[test]
    fn only_code_that_changed_screens_is_shown() {
        assert!(is_ui_change(&ticket("frontend-2"), 0));
        assert!(is_ui_change(&ticket("mobile"), 0));
        assert!(is_ui_change(&ticket("designer"), 0));
        assert!(is_ui_change(&ticket("backend"), 0), "a backend is the software engineer, who builds screens");
        assert!(!is_ui_change(&ticket("data"), 0));
        // A change that went through design QA changed a screen.
        assert!(is_ui_change(&ticket("data"), 1));
        let mut task = ticket("frontend");
        task.kind = TicketKind::Task;
        assert!(!is_ui_change(&task, 3));
    }

    #[test]
    fn the_link_is_the_product_not_the_code_or_this_machine() {
        let note = "Built the page (see https://github.com/acme/app/pull/4). Preview: https://app-git-signin-acme.vercel.app/login. Local: http://localhost:3000";
        assert_eq!(link(Some(note), Some("https://acme.com")), Some("https://app-git-signin-acme.vercel.app/login".into()));
        assert_eq!(link(Some("Runs on https://localhost:3000 and https://127.0.0.1:8080/x"), Some("https://acme.com")), Some("https://acme.com".into()));
        assert_eq!(link(Some("PR: <https://gitlab.com/a/b/-/merge_requests/2>"), None), None);
        assert_eq!(link(Some("Live at (https://acme.pages.dev/)."), None), Some("https://acme.pages.dev/".into()));
        assert_eq!(link(None, Some("acme.com")), None);
    }

    #[test]
    fn a_result_is_copied_into_the_uploads_with_its_recording_and_kept() {
        let dir = scratch("stage");
        let home = dir.join("home");
        let worktree = dir.join("wt");
        std::fs::create_dir_all(worktree.join("shots")).unwrap();
        std::fs::create_dir_all(worktree.join("design/flows/sign-in")).unwrap();
        std::fs::write(worktree.join("shots/login-1280.png"), b"png").unwrap();
        std::fs::write(worktree.join("design/flows/sign-in/walkthrough.light.390.mp4"), b"mp4").unwrap();
        let inputs = Inputs {
            squad: "acme".into(),
            ticket: 12,
            title: "Sign-in page".into(),
            shots: s(&[&worktree.join("shots/login-1280.png").to_string_lossy(), "shots/gone.png"]),
            worktree: Some(worktree.clone()),
            link: Some("https://acme.com/login".into()),
        };
        let shown = stage(&home, &inputs, false).unwrap();
        assert_eq!((shown.shots, shown.video, shown.tags.len()), (1, true, 2), "{shown:?}");
        assert!(shown.tags[0].starts_with(&format!("[image: {}", crate::gateway::uploads_dir(&home).display())), "{shown:?}");
        assert!(shown.tags[1].starts_with("[video: ") && shown.tags[1].ends_with("walkthrough-light-390.mp4]"), "{shown:?}");
        // Kept like a sent message's files: never swept as a draft.
        assert_eq!(crate::kept::names(&home).len(), 2);
        let message = shown.message("T-12 is done — here is how it looks · Sign-in page.", Some("See it: https://acme.com/login"));
        assert_eq!(message.lines().count(), 4, "{message}");
        assert!(message.ends_with("See it: https://acme.com/login"));
        assert!(shown.told().contains("1 screenshot, a recording of the flow, its link (https://acme.com/login)"), "{}", shown.told());
        // Nothing to show at all: no message.
        let empty = Inputs { shots: Vec::new(), worktree: None, link: None, ..inputs };
        assert_eq!(stage(&home, &empty, false), None);
    }

    #[test]
    fn a_recording_too_big_to_send_is_left_out_on_the_loop() {
        let dir = scratch("big");
        let big = dir.join("walkthrough.light.390.mp4");
        std::fs::File::create(&big).unwrap().set_len(VIDEO_MAX + 1).unwrap();
        assert_eq!(video_in(&dir.join("home"), &big, false), None);
        let args = shrink_args(&big, Path::new("/u/out.mp4"));
        assert_eq!(args.last().map(String::as_str), Some("/u/out.mp4"));
        assert!(args.windows(2).any(|w| w[0] == "-t" && w[1] == "30"), "{args:?}");
    }

    #[test]
    fn without_a_link_the_one_who_accepted_is_asked_for_it() {
        let shown = Shown { ticket: 3, title: "x".into(), tags: s(&["[image: /u/a.png]"]), shots: 1, video: false, link: None };
        assert!(shown.told().contains("give its address"));
        assert_eq!(shown.message("Done.", Some("See it: x")), "Done.\n[image: /u/a.png]");
    }
}
