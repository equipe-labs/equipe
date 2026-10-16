//! JSON-RPC 2.0 framing for the protocol, one message per line — the same
//! shape ACP, MCP and LSP use, so any language can attach.
//!
//! - `hello` (request): `{ version, squad }` → [`Hello`] with the squad's snapshot
//! - `command` (notification): a [`Command`](crate::Command)
//! - `event` (notification, daemon → screen): an [`Event`](crate::Event)
//! - `tool` (request): a member's board tool call, from the MCP server equipe
//!   gives its engine → [`ToolResult`]
//! - `desktop` (request): a member's desktop tool call (`equipe mcp --server
//!   desktop`) → [`ToolResult`], answered by the desktop helper
//! - `shutdown` (request): stop the daemon; sessions stay restorable
//! - `version` (request): → [`Version`], the daemon's build and wire version
//! - `squads` (request): → [`Squads`], the squads on the daemon's machine
//! - `upload` (request): [`Upload`] → [`Uploaded`], an image (with a `kind`,
//!   any file) saved on the daemon's machine for a message to point at. A big
//!   file goes over its own SSH channel instead (`equipe receive`), never as
//!   one giant line.
//! - `read_upload` (request): [`ReadUpload`] → [`UploadRead`], a file a message
//!   attached, read back from the daemon's `uploads` folder (and nowhere else)
//!   so a screen on another machine can show it; only after `hello`
//! - `browser_stream` (request): the connection becomes the browser panel's
//!   picture stream ([`crate::stream`])
//! - `attached` (notification, `equipe attach --stdio` → screen): [`Attached`],
//!   the first line a screen reads through the bridge, before anything the
//!   daemon says

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Command, Event, Snapshot};

pub const HELLO: &str = "hello";
pub const COMMAND: &str = "command";
pub const EVENT: &str = "event";
pub const SHUTDOWN: &str = "shutdown";
pub const TOOL: &str = "tool";
pub const VERSION: &str = "version";
pub const ATTACHED: &str = "attached";
pub const SQUADS: &str = "squads";
pub const UPLOAD: &str = "upload";
pub const READ_UPLOAD: &str = "read_upload";

/// The wire's version: bumped when a change would break a screen built
/// against the one before (a method or a field it relies on removed or
/// changed). Added methods, events and optional fields don't bump it.
pub const PROTOCOL: u32 = 1;

pub const DESKTOP: &str = "desktop";
/// Who is busy: members mid-turn and requests waiting for the person.
pub const BUSY: &str = "busy";
/// A window for the person in the members' shared Chrome, at a page (`equipe
/// browser`): `true` when there is such a Chrome.
pub const BROWSER_WINDOW: &str = "browser_window";
/// The live browser panel's own connection (params
/// [`crate::stream::StreamHello`]), answered `true`; after the answer the
/// connection carries [`crate::stream`]'s binary messages, both ways.
pub const BROWSER_STREAM: &str = "browser_stream";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

/// What a screen says when it attaches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloParams {
    pub version: String,
    /// The squad to follow: only its members and board are snapshotted and streamed.
    pub squad: crate::SquadName,
    /// The language the screen speaks (`ko`, `zh-Hant`): what equipe says to
    /// the person from now on — notices, errors — is in it. An older daemon
    /// ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// The screen shows members' browsers in a panel of its own (the desktop
    /// app, on this Mac or another): a member handing the person its browser
    /// opens there, rather than as a window on the daemon's Mac, which nobody
    /// may be looking at. An older screen, or one without a panel: `false`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub browser_panel: bool,
}

/// A member calling one of its board tools.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub squad: crate::SquadName,
    /// The member's name, as equipe started its MCP server.
    pub member: String,
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

/// What a tool call returns: text for the model, whether it failed, and — for
/// a desktop screenshot — an image, which is never written anywhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    pub text: String,
    pub is_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<Image>,
}

impl ToolResult {
    pub fn ok(text: impl Into<String>) -> Self {
        Self { text: text.into(), is_error: false, image: None }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self { text: text.into(), is_error: true, image: None }
    }
}

/// An image in a tool result: its MIME type and its bytes in base64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    pub mime: String,
    pub data: String,
}

/// The daemon's answer to `hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub version: String,
    pub snapshot: Snapshot,
}

/// The daemon's answer to `version`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// Its build, as `hello` reports it (`0.4.1+1760400000`).
    pub version: String,
    pub protocol: u32,
}

/// The squads on the daemon's machine, for a screen that can't read its disk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Squads {
    pub squads: Vec<SquadEntry>,
    /// The one the person opened last, when it still exists; else the first.
    pub last: Option<crate::SquadName>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SquadEntry {
    pub name: crate::SquadName,
    /// Its projects' folders, on the daemon's machine.
    pub projects: Vec<std::path::PathBuf>,
    /// Its home folder (`/home`), on the daemon's machine.
    pub home: std::path::PathBuf,
}

/// A file to keep on the daemon's machine: an image unless `kind` says otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Upload {
    /// The file's name where it came from; its extension and a tidied stem
    /// are kept, never its folder.
    pub name: String,
    /// The bytes, base64 (standard alphabet, padded).
    pub data: String,
    /// What it is. Absent (screens from before attachments): an image, and
    /// only image extensions are taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<UploadKind>,
}

/// What an [`Upload`] carries, which decides the extensions it may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadKind {
    /// png, jpg, gif, webp — a member looks at it with Read.
    Image,
    /// A video, kept as it came; its frames travel as images.
    Video,
    Pdf,
    /// Anything else: text, code, documents, audio.
    File,
}

/// Where an [`Upload`] was saved: a message attaches it as `[image: <path>]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Uploaded {
    pub path: std::path::PathBuf,
}

/// Ask for a file an attachment tag points at, back from the daemon's
/// machine: only one in its `uploads` folder is given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadUpload {
    pub path: std::path::PathBuf,
    /// Only how big it is, not its bytes (a PDF's or a document's chip).
    #[serde(default, skip_serializing_if = "is_false")]
    pub size_only: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A file read back: its size, and its bytes (base64) unless only the size was asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadRead {
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

/// What `equipe attach --stdio` says first: which equipe answered on the
/// other end, and which daemon it reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attached {
    /// The build of the `equipe` that runs the bridge.
    pub bridge: String,
    /// The daemon's build; `None` when it predates `version`.
    pub daemon: Option<String>,
    /// The daemon's wire version, else the bridge's own when the daemon
    /// predates `version`.
    pub protocol: u32,
    /// Whether the bridge had to start the daemon.
    pub started: bool,
}

/// What the daemon is in the middle of, across every squad.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Busy {
    /// Members mid-turn, as `squad/member`.
    pub working: Vec<String>,
    /// Requests for permission waiting for the person.
    pub approvals: usize,
    /// Who has the Mac's screen and who waits for it.
    #[serde(default)]
    pub desktop: crate::DesktopState,
}

impl Busy {
    pub fn idle(&self) -> bool {
        self.working.is_empty() && self.approvals == 0
    }
}

impl Message {
    fn base() -> Self {
        Self { jsonrpc: "2.0".into(), id: None, method: None, params: None, result: None, error: None }
    }

    pub fn request(id: u64, method: &str, params: impl Serialize) -> Self {
        Self { id: Some(id), method: Some(method.into()), params: Some(to_value(params)), ..Self::base() }
    }

    pub fn notification(method: &str, params: impl Serialize) -> Self {
        Self { method: Some(method.into()), params: Some(to_value(params)), ..Self::base() }
    }

    pub fn response(id: u64, result: impl Serialize) -> Self {
        Self { id: Some(id), result: Some(to_value(result)), ..Self::base() }
    }

    pub fn failure(id: u64, code: i64, message: impl Into<String>) -> Self {
        Self { id: Some(id), error: Some(RpcError { code, message: message.into() }), ..Self::base() }
    }

    pub fn command(command: &Command) -> Self {
        Self::notification(COMMAND, command)
    }

    pub fn event(event: &Event) -> Self {
        Self::notification(EVENT, event)
    }

    /// One line of the wire: JSON without embedded newlines, then `\n`.
    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).expect("messages serialize");
        line.push('\n');
        line
    }

    pub fn from_line(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line.trim_end())
    }

    /// The params decoded as `T`.
    pub fn params_as<T: for<'de> Deserialize<'de>>(&self) -> Option<T> {
        self.params.clone().and_then(|p| serde_json::from_value(p).ok())
    }

    /// The result decoded as `T`.
    pub fn result_as<T: for<'de> Deserialize<'de>>(&self) -> Option<T> {
        self.result.clone().and_then(|r| serde_json::from_value(r).ok())
    }
}

fn to_value(v: impl Serialize) -> Value {
    serde_json::to_value(v).expect("protocol values serialize")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Command;

    #[test]
    fn a_command_travels_as_a_one_line_notification() {
        let cmd = Command::Submit { session: 3, text: "two\nlines".into() };
        let line = Message::command(&cmd).to_line();
        assert_eq!(line.matches('\n').count(), 1, "exactly the trailing newline: {line:?}");
        let back = Message::from_line(&line).unwrap();
        assert_eq!(back.method.as_deref(), Some(COMMAND));
        assert_eq!(back.id, None);
        assert_eq!(back.params_as::<Command>(), Some(cmd));
    }

    #[test]
    fn requests_and_responses_pair_by_id() {
        let req = Message::request(7, HELLO, HelloParams { version: "1".into(), squad: "acme".into(), lang: None, browser_panel: false });
        let res = Message::response(7, Hello { version: "1".into(), snapshot: Default::default() });
        assert_eq!(req.id, res.id);
        assert!(res.result_as::<Hello>().is_some());
        assert!(Message::from_line(&req.to_line()).unwrap().params_as::<HelloParams>().is_some());
    }

    #[test]
    fn an_upload_without_a_kind_is_an_image_and_reads_as_before() {
        let up: Upload = serde_json::from_str(r#"{"name":"a.png","data":"AA=="}"#).unwrap();
        assert_eq!(up.kind, None);
        let line = serde_json::to_string(&up).unwrap();
        assert!(!line.contains("kind"), "an older daemon reads the same line: {line}");
        let pdf = Upload { name: "spec.pdf".into(), data: "AA==".into(), kind: Some(UploadKind::Pdf) };
        assert!(serde_json::to_string(&pdf).unwrap().contains(r#""kind":"pdf""#));
    }

    #[test]
    fn reading_an_upload_back_is_a_request_of_its_own() {
        let ask = Message::request(9, READ_UPLOAD, ReadUpload { path: "/h/uploads/1-a.png".into(), size_only: false });
        let line = ask.to_line();
        assert!(line.contains(r#""method":"read_upload""#) && !line.contains("size_only"), "{line}");
        assert_eq!(Message::from_line(&line).unwrap().params_as::<ReadUpload>().unwrap().path, std::path::PathBuf::from("/h/uploads/1-a.png"));
        let answer = Message::response(9, UploadRead { size: 3, data: None });
        assert_eq!(answer.result_as::<UploadRead>(), Some(UploadRead { size: 3, data: None }));
    }
}
