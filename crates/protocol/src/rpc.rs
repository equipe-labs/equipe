//! JSON-RPC 2.0 framing for the protocol, one message per line — the same
//! shape ACP, MCP and LSP use, so any language can attach.
//!
//! - `hello` (request): `{ version, project }` → [`Hello`] with the project's snapshot
//! - `command` (notification): a [`Command`](crate::Command)
//! - `event` (notification, daemon → screen): an [`Event`](crate::Event)
//! - `shutdown` (request): stop the daemon; sessions stay restorable

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Command, Event, Snapshot};

pub const HELLO: &str = "hello";
pub const COMMAND: &str = "command";
pub const EVENT: &str = "event";
pub const SHUTDOWN: &str = "shutdown";

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
    /// The project to follow: only its sessions are snapshotted and streamed.
    pub project: std::path::PathBuf,
}

/// The daemon's answer to `hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub version: String,
    pub snapshot: Snapshot,
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
        let req = Message::request(7, HELLO, HelloParams { version: "1".into(), project: "/p".into() });
        let res = Message::response(7, Hello { version: "1".into(), snapshot: Default::default() });
        assert_eq!(req.id, res.id);
        assert!(res.result_as::<Hello>().is_some());
        assert!(Message::from_line(&req.to_line()).unwrap().params_as::<HelloParams>().is_some());
    }
}
