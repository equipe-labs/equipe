//! Sites' one-time codes from text messages: on a Mac whose Messages app
//! gets the person's iPhone texts (Text Message Forwarding), the codes land
//! in `~/Library/Messages/chat.db`. macOS guards that file: equipe can read
//! it only once the person grants it Full Disk Access themselves (System
//! Settings › Privacy & Security › Full Disk Access); without that grant it
//! refuses and says so — it never works around the guard. Read-only, only
//! messages from the last minutes, only the code out of a message that is
//! the site's ([`super::codes::sms_matches`]).

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use super::codes;

/// Why codes can't be read from Messages here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Not a Mac: no Messages database.
    Unsupported,
    /// macOS won't let equipe read it: the person hasn't granted Full Disk Access.
    NoAccess,
    /// No Messages database (Messages never set up on this Mac).
    Missing,
    Other(String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Unsupported => f.write_str("reading text messages needs macOS's Messages"),
            Refused::NoAccess => f.write_str("equipe has no Full Disk Access, which macOS asks for before Messages can be read (System Settings › Privacy & Security › Full Disk Access)"),
            Refused::Missing => f.write_str("Messages isn't set up on this Mac"),
            Refused::Other(why) => f.write_str(why),
        }
    }
}

/// Where Messages keeps its texts, for the person `home`.
pub fn database(home: &Path) -> PathBuf {
    home.join("Library/Messages/chat.db")
}

/// Whether equipe may read Messages now: `None` off a Mac.
pub fn access(home: &Path) -> Option<bool> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    Some(std::fs::File::open(database(home)).is_ok())
}

/// Seconds between 1970 and 2001, Messages' epoch.
const APPLE_EPOCH: i64 = 978_307_200;

/// A message's date as Messages keeps it (nanoseconds since 2001, or
/// seconds in older databases) as unix seconds.
fn unix_of(date: i64) -> u64 {
    let secs = if date.abs() > 10_000_000_000 { date / 1_000_000_000 } else { date };
    u64::try_from(secs + APPLE_EPOCH).unwrap_or(0)
}

fn apple_of(unix: u64) -> i64 {
    (unix as i64 - APPLE_EPOCH) * 1_000_000_000
}

/// A message's text out of `attributedBody`, where newer macOS keeps it
/// (an archived NSAttributedString): the string after `NSString`, its
/// length in a byte (or 0x81 and two bytes) after a `+`.
fn attributed_text(blob: &[u8]) -> Option<String> {
    let at = blob.windows(8).position(|w| w == b"NSString")? + 8;
    let rest = &blob[at..];
    let plus = rest.iter().position(|&b| b == b'+')?;
    let rest = &rest[plus + 1..];
    let (len, start) = match *rest.first()? {
        0x81 => (u16::from_le_bytes([*rest.get(1)?, *rest.get(2)?]) as usize, 3),
        0x82 => (u32::from_le_bytes([*rest.get(1)?, *rest.get(2)?, *rest.get(3)?, *rest.get(4)?]) as usize, 5),
        n => (n as usize, 1),
    };
    rest.get(start..start + len).map(|b| String::from_utf8_lossy(b).into_owned())
}

/// The code for `site` among the texts that came since `since` (unix
/// seconds) in the Messages database `db`: the newest message that is the
/// site's. Opened read-only; nothing written.
pub fn code_since(db: &Path, site: &str, since: u64) -> Result<Option<String>, Refused> {
    if !cfg!(target_os = "macos") && !cfg!(test) {
        return Err(Refused::Unsupported);
    }
    match std::fs::File::open(db) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Refused::Missing),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Err(Refused::NoAccess),
        Err(e) => return Err(Refused::Other(e.to_string())),
    }
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| Refused::Other(e.to_string()))?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(2));
    let mut stmt = conn
        .prepare("SELECT text, attributedBody, date FROM message WHERE is_from_me = 0 AND date >= ?1 ORDER BY date DESC LIMIT 20")
        .map_err(|e| if e.to_string().contains("authorization") { Refused::NoAccess } else { Refused::Other(e.to_string()) })?;
    let rows = stmt
        .query_map([apple_of(since.saturating_sub(30))], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<Vec<u8>>>(1)?, r.get::<_, i64>(2)?)))
        .map_err(|e| Refused::Other(e.to_string()))?;
    for (text, body, date) in rows.filter_map(Result::ok) {
        if unix_of(date) + 30 < since {
            continue;
        }
        let Some(text) = text.filter(|t| !t.trim().is_empty()).or_else(|| body.as_deref().and_then(attributed_text)) else { continue };
        if codes::sms_matches(&text, site)
            && let Some(code) = codes::find_code(&text)
        {
            return Ok(Some(code));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(dir: &Path) -> PathBuf {
        let path = dir.join("chat.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE message (ROWID INTEGER PRIMARY KEY, text TEXT, attributedBody BLOB, date INTEGER, is_from_me INTEGER)").unwrap();
        path
    }

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("equipe-sms-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_sites_newest_text_since_the_sign_in_gives_the_code() {
        let dir = temp();
        let path = db(&dir);
        let conn = Connection::open(&path).unwrap();
        let now = 1_794_528_000;
        let add = |text: Option<&str>, body: Option<Vec<u8>>, at: u64, mine: i64| {
            conn.execute("INSERT INTO message (text, attributedBody, date, is_from_me) VALUES (?1, ?2, ?3, ?4)", rusqlite::params![text, body, apple_of(at), mine]).unwrap();
        };
        add(Some("Your GitHub code is 111111"), None, now - 600, 0);
        add(Some("Your bank code is 222222"), None, now + 5, 0);
        add(Some("Your GitHub code is 333333"), None, now + 10, 1);
        assert_eq!(code_since(&path, "github.com", now).unwrap(), None, "older, another sender's, and the person's own");
        let mut blob = b"\x04\x0bstreamtyped\x81\xe8\x03\x84\x01@\x84\x84\x84\x12NSAttributedString\x00\x84\x84\x08NSObject\x00\x85\x92\x84\x84\x84\x08NSString\x01\x94\x84\x01+".to_vec();
        let text = "444444 is your GitHub authentication code.";
        blob.push(text.len() as u8);
        blob.extend_from_slice(text.as_bytes());
        add(None, Some(blob), now + 20, 0);
        assert_eq!(code_since(&path, "github.com", now).unwrap().as_deref(), Some("444444"), "read from attributedBody");
        assert_eq!(code_since(&dir.join("none.db"), "github.com", now), Err(Refused::Missing));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dates_are_messages_own() {
        assert_eq!(unix_of(apple_of(1_794_528_000)), 1_794_528_000);
        assert_eq!(unix_of(100), 978_307_300, "seconds in an old database");
    }
}
