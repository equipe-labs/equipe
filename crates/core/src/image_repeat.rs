//! A picture a member already looked at, not read twice. An image stays in
//! Claude Code's context for the rest of the session and is paid for again on
//! every model call, so reading the same screenshot again adds cost and shows
//! the member nothing new. Claude Code asks a `PreToolUse` hook (`equipe hook
//! --images`) before each `Read`; for an image file whose bytes this session
//! already read, the answer is a refusal that says so.
//!
//! - What is remembered: the SHA-256 of each image a session read, in a small
//!   file per session id under the lane's own temporary folder, so it goes with
//!   the lane ([`crate::disk::clear_lane_tmp`]). A changed file at the same
//!   path has other bytes, and is read.
//! - Never in the way: the second ask in a row for the same picture goes
//!   through. A conversation that was compacted may no longer hold the image,
//!   and a member that really needs it again must be able to get it.
//! - Fails open: a file that can't be read, is too big, isn't an image, a
//!   state folder that can't be written, input that isn't understood: the
//!   read goes on as it would have.
//! - Counted: each refusal adds a line to the session's count file, and the
//!   total goes in the log when the lane's folder is cleared ([`saved`]).

//!
//! Claude Code only. Codex CLI (0.154) has a `pre_tool_use` hook event, but its
//! pictures come in through the `view_image` tool, and nothing shows that the
//! hook is asked about that tool, so Codex members rely on the charter's one
//! sentence (look at a screenshot once) until that is confirmed.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The tool whose calls this hook is asked about.
pub const MATCHER: &str = "Read";

/// Where, under a lane's temporary folder, the sessions' state is kept.
pub const STATE_DIR: &str = ".equipe-images";

/// A file bigger than this is let through unhashed: screenshots are far smaller,
/// and the hook must stay quick.
const MAX_BYTES: u64 = 25 * 1024 * 1024;

const HOOK_TIMEOUT_SECS: u64 = 10;

/// What the member is told when it asks for a picture it already viewed.
pub const REPEAT_REFUSED: &str = "You already viewed this exact image earlier in this conversation (same content); \
it is still in your context. Look back at it. If the screen changed, take a new screenshot first. Only if you truly \
can't see it any more (the conversation was compacted), ask for it again: the next read goes through.";

/// Add the hook to Claude Code's `hooks` settings: `equipe hook --images <dir>`
/// before each `Read`, `dir` being the lane's temporary folder.
pub fn add_hook(hooks: &mut Value, binary: &Path, dir: &Path) {
    let command = format!("{} hook --images {}", quote(&binary.to_string_lossy()), quote(&dir.to_string_lossy()));
    let entry = json!({ "matcher": MATCHER, "hooks": [{ "type": "command", "command": command, "timeout": HOOK_TIMEOUT_SECS }] });
    if let Some(list) = hooks.get_mut("PreToolUse").and_then(Value::as_array_mut) {
        list.push(entry);
    } else if let Some(map) = hooks.as_object_mut() {
        map.insert("PreToolUse".to_owned(), json!([entry]));
    }
}

fn quote(word: &str) -> String {
    if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+:@%=".contains(c)) {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["png", "jpg", "jpeg", "gif", "webp"].iter().any(|k| e.eq_ignore_ascii_case(k)))
}

fn sha(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// A session id as a file name, or `None` for anything that could leave the folder.
fn session_file(dir: &Path, session: &str) -> Option<PathBuf> {
    let ok = !session.is_empty() && session.len() <= 100 && session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| dir.join(STATE_DIR).join(session))
}

/// The hook's answer for one call (Claude Code's JSON on stdin): a refusal when
/// this session already read this image, else `None` — never an approval.
pub fn answer(call: &Value, dir: &Path) -> Option<Value> {
    if call.get("tool_name").and_then(Value::as_str)? != MATCHER {
        return None;
    }
    let session = call.get("session_id").and_then(Value::as_str)?;
    let path = call.get("tool_input")?.get("file_path").and_then(Value::as_str)?;
    let cwd = call.get("cwd").and_then(Value::as_str).map(PathBuf::from);
    let path = match (Path::new(path).is_absolute(), cwd) {
        (true, _) => PathBuf::from(path),
        (false, Some(cwd)) => cwd.join(path),
        (false, None) => return None,
    };
    if !is_image(&path) || std::fs::metadata(&path).ok()?.len() > MAX_BYTES {
        return None;
    }
    let hash = sha(&std::fs::read(&path).ok()?);
    let file = session_file(dir, session)?;
    if repeated(&file, &hash)? {
        count(&file);
        return Some(json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": REPEAT_REFUSED } }));
    }
    None
}

/// Whether `hash` should be refused for the session whose state is `file`, and
/// the state after: a first sight is remembered, a second refused once (marked
/// `!`), and the ask after that goes through and clears the mark. `None` when
/// the state can't be kept.
fn repeated(file: &Path, hash: &str) -> Option<bool> {
    let known = std::fs::read_to_string(file).unwrap_or_default();
    let warned = format!("!{hash}");
    let lines: Vec<&str> = known.lines().collect();
    let (refuse, next): (bool, Vec<String>) = if lines.contains(&warned.as_str()) {
        (false, lines.iter().map(|l| if *l == warned { hash.to_owned() } else { (*l).to_owned() }).collect())
    } else if lines.contains(&hash) {
        (true, lines.iter().filter(|l| **l != hash).map(|l| (*l).to_owned()).chain([warned]).collect())
    } else {
        (false, lines.iter().map(|l| (*l).to_owned()).chain([hash.to_owned()]).collect())
    };
    std::fs::create_dir_all(file.parent()?).ok()?;
    std::fs::write(file, next.join("\n") + "\n").ok()?;
    Some(refuse)
}

fn count(file: &Path) {
    let mut name = file.as_os_str().to_owned();
    name.push(".blocked");
    if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(name) {
        let _ = out.write_all(b"1\n");
    }
}

/// How many repeated reads the lane folder `dir` saved, over all its sessions,
/// as a line for the log; `None` when there were none.
pub fn saved(dir: &Path) -> Option<String> {
    let mut blocked = 0usize;
    let mut sessions = 0usize;
    for entry in std::fs::read_dir(dir.join(STATE_DIR)).ok()?.flatten() {
        if entry.file_name().to_string_lossy().ends_with(".blocked") {
            blocked += std::fs::read_to_string(entry.path()).map_or(0, |s| s.lines().count());
            sessions += 1;
        }
    }
    (blocked > 0).then(|| format!("saved {blocked} repeated image read{} in {sessions} session{}", if blocked == 1 { "" } else { "s" }, if sessions == 1 { "" } else { "s" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-image-repeat-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(session: &str, file: &Path) -> Value {
        json!({ "session_id": session, "tool_name": "Read", "tool_input": { "file_path": file }, "cwd": "/" })
    }

    fn denied(answer: Option<Value>) -> bool {
        answer.is_some_and(|a| a["hookSpecificOutput"]["permissionDecision"] == "deny" && a["hookSpecificOutput"]["permissionDecisionReason"] == REPEAT_REFUSED)
    }

    #[test]
    fn the_first_read_goes_through_and_an_exact_repeat_is_refused() {
        let dir = scratch("repeat");
        let shot = dir.join("a.png");
        std::fs::write(&shot, b"pixels").unwrap();
        assert!(answer(&read("s1", &shot), &dir).is_none(), "the first read");
        assert!(denied(answer(&read("s1", &shot), &dir)), "the same bytes again");
        // Asked again at once, it goes through (a compacted conversation may have lost it), then is refused again.
        assert!(answer(&read("s1", &shot), &dir).is_none());
        assert!(denied(answer(&read("s1", &shot), &dir)));
        assert_eq!(saved(&dir).unwrap(), "saved 2 repeated image reads in 1 session");
    }

    #[test]
    fn a_changed_image_at_the_same_path_and_another_session_are_read() {
        let dir = scratch("changed");
        let shot = dir.join("a.PNG");
        std::fs::write(&shot, b"before").unwrap();
        assert!(answer(&read("s1", &shot), &dir).is_none());
        std::fs::write(&shot, b"after").unwrap();
        assert!(answer(&read("s1", &shot), &dir).is_none(), "other bytes");
        assert!(denied(answer(&read("s1", &shot), &dir)));
        assert!(answer(&read("s2", &shot), &dir).is_none(), "another session has not seen it");
    }

    #[test]
    fn what_is_not_an_image_is_always_read() {
        let dir = scratch("text");
        let text = dir.join("a.txt");
        std::fs::write(&text, b"words").unwrap();
        for _ in 0..3 {
            assert!(answer(&read("s1", &text), &dir).is_none());
        }
        assert!(saved(&dir).is_none());
    }

    #[test]
    fn errors_let_the_read_through() {
        let dir = scratch("errors");
        let shot = dir.join("a.png");
        std::fs::write(&shot, b"pixels").unwrap();
        assert!(answer(&read("s1", &dir.join("missing.png")), &dir).is_none(), "a file that isn't there");
        assert!(answer(&read("../x", &shot), &dir).is_none(), "a session id that is not a name");
        assert!(answer(&json!({ "tool_name": "Read" }), &dir).is_none(), "no input");
        assert!(answer(&json!({ "session_id": "s", "tool_name": "Read", "tool_input": { "file_path": "a.png" } }), &dir).is_none(), "a relative path with no cwd");
        // A state folder that can't be made: a file stands where it should be.
        let blocked = scratch("errors-state");
        std::fs::write(blocked.join(STATE_DIR), b"in the way").unwrap();
        assert!(answer(&read("s1", &shot), &blocked).is_none());
        assert!(answer(&read("s1", &shot), &blocked).is_none(), "nothing is remembered, nothing refused");
    }

    #[test]
    fn the_hook_is_added_beside_the_guard() {
        let mut hooks = crate::guard::hooks(Path::new("/opt/equipe"), Path::new("/w"), &[]);
        add_hook(&mut hooks, Path::new("/opt/equipe"), Path::new("/home/tmp/acme/t1"));
        let list = hooks["PreToolUse"].as_array().unwrap();
        assert_eq!(list.len(), 2, "the guard stays");
        assert_eq!(list[1]["matcher"], "Read");
        assert_eq!(list[1]["hooks"][0]["command"], "/opt/equipe hook --images /home/tmp/acme/t1");
    }
}
