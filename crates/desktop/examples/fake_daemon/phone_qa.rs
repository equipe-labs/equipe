//! `FAKE_PHONE_QA=<file>`: the conversation ends on a phone test's report —
//! the note equipe writes when a member calls the phone tools' `report`
//! (`equipe_core::phone::report::note`), read from `<file>` as it was kept,
//! so a real run's note (and its frames, beside it) can be looked at.
//! `read_image` gives the pictures it names, and nothing else.

use std::path::Path;

use equipe_core::protocol::rpc::{ImageRead, ReadImage};

fn note_file() -> Option<String> {
    std::env::var("FAKE_PHONE_QA").ok().filter(|v| !v.is_empty())
}

/// The report's note, when it is on.
pub fn note() -> Option<String> {
    std::fs::read_to_string(note_file()?).ok().map(|t| t.trim().to_owned())
}

/// A frame the note names.
pub fn read(ask: &ReadImage) -> Result<ImageRead, String> {
    let note = note().ok_or("no phone report")?;
    let named = equipe_core::visuals::markdown::local_images(&note).into_iter().any(|i| i.path == ask.path);
    if !named {
        return Err(format!("{} isn't a frame of the report", ask.path));
    }
    let dir = Path::new(&ask.path).parent().ok_or("no folder")?.to_owned();
    equipe_core::visuals::read(Path::new(&ask.path), &[dir], ask.max)
}
