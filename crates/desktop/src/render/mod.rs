//! How a conversation's text is drawn: Markdown with highlighted code,
//! file chips, math, diagrams and callouts (`view`, `plugins`), the rules
//! behind them (`code`, `links`, `prose`, `stream`, `tools`) and the pictures
//! (`raster`). The rules have no UI types and are tested on their own.

pub mod code;
pub mod links;
pub mod plugins;
pub mod prose;
pub mod raster;
pub mod stream;
pub mod tools;
pub mod view;
mod wrap;

use std::path::{Path, PathBuf};

use equipe_core::t;
use gpui_kit::*;

use crate::Shell;
use crate::theme::Palette;

/// The window's shell, for elements drawn inside a text view (which can't
/// hold it: their handlers must be `Send`) to reach when clicked.
pub struct ShellRef(pub WeakEntity<Shell>);

impl Global for ShellRef {}

/// Run `f` on the shell, if there is one.
pub fn with_shell(cx: &mut App, f: impl FnOnce(&mut Shell, &mut Context<Shell>)) {
    if let Some(shell) = cx.try_global::<ShellRef>().map(|s| s.0.clone()) {
        shell.update(cx, f).ok();
    }
}

/// Colours the conversation needs beyond the palette's: a diff's, an error's,
/// the callouts'.
#[derive(Clone, Copy)]
pub struct Tones {
    pub added: Rgba,
    pub added_bg: Rgba,
    pub removed: Rgba,
    pub removed_bg: Rgba,
    pub danger: Rgba,
    pub danger_bg: Rgba,
    pub note: Rgba,
    pub tip: Rgba,
    pub important: Rgba,
    pub warning: Rgba,
}

pub fn is_dark(p: &Palette) -> bool {
    p.bg.r < 0.5
}

pub fn tones(p: &Palette) -> Tones {
    if is_dark(p) {
        Tones {
            added: rgb(0x8CD096),
            added_bg: rgba(0x8CD0961C),
            removed: rgb(0xEC8A8A),
            removed_bg: rgba(0xEC8A8A1C),
            danger: rgb(0xF08C8C),
            danger_bg: rgba(0xF08C8C14),
            note: rgb(0x8DB8E8),
            tip: rgb(0x8CD096),
            important: rgb(0xB6A2F2),
            warning: rgb(0xE8BE78),
        }
    } else {
        Tones {
            added: rgb(0x28803C),
            added_bg: rgba(0x28803C17),
            removed: rgb(0xB0323A),
            removed_bg: rgba(0xB0323A14),
            danger: rgb(0xB0323A),
            danger_bg: rgba(0xB0323A0F),
            note: rgb(0x1E68AA),
            tip: rgb(0x28803C),
            important: rgb(0x6446BE),
            warning: rgb(0xA06814),
        }
    }
}

/// What a click on a file does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Open {
    /// Quick Look: a look without leaving the conversation.
    Look,
    /// In the app that opens it (an editor, for code).
    Edit,
    Reveal,
}

/// The directory a member works in, against which its relative paths are read.
pub fn base_dir(shell: &Shell) -> PathBuf {
    shell.model.convo().map(|c| c.base.clone()).unwrap_or_default()
}

/// Open `path` as `how` asks. A path this Mac doesn't have (the daemon is
/// another machine's, or the file is gone) says so, and its path is copied.
pub fn open_file(path: &Path, how: Open, cx: &mut App) {
    if !path.exists() {
        cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string()));
        flash(t!("render.file_not_here", path = path.display()), cx);
        return;
    }
    match how {
        Open::Edit => cx.open_with_system(path),
        Open::Reveal => cx.reveal_path(path),
        Open::Look if path.is_dir() => cx.reveal_path(path),
        Open::Look => quick_look(path),
    }
}

fn quick_look(path: &Path) {
    let spawned = std::process::Command::new("qlmanage")
        .arg("-p")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    // Waited for on a thread of its own, so it leaves no zombie behind.
    if let Ok(mut child) = spawned {
        std::thread::spawn(move || child.wait());
    }
}

/// A word for the flash line under the composer.
pub fn flash(text: String, cx: &mut App) {
    with_shell(cx, |shell, cx| {
        shell.composer.flash = Some(text);
        cx.notify();
    });
}

/// A stable number for `value`, for element ids and caches.
pub fn hash_of(value: impl std::hash::Hash) -> u64 {
    use std::hash::Hasher as _;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}
