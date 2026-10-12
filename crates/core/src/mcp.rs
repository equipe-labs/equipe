//! The MCP server equipe gives each member's engine: its board tools.
//!
//! Claude Code starts `equipe mcp --squad … --member … --role …` as a stdio MCP
//! server (newline-delimited JSON-RPC). The server knows nothing itself: each
//! tool call goes to the daemon over its socket, where the runtime decides. This
//! module is the protocol half, as plain functions; the CLI wires it to stdin,
//! stdout and the socket.

use equipe_protocol::rpc::ToolResult;
use serde_json::{Value, json};

/// The MCP revision equipe speaks when the client doesn't ask for another.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// What to do with one line from the client.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Answer with this message.
    Reply(Value),
    /// Run a tool, then answer `id` with [`call_reply`].
    Call { id: Value, name: String, arguments: Value },
    /// A notification, or something unreadable: nothing to say.
    Nothing,
}

/// Decide what one client line asks for. `tools` is what `tools/list` offers.
pub fn handle(line: &str, tools: &[Value], version: &str) -> Step {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return Step::Nothing;
    };
    let Some(id) = message.get("id").filter(|id| !id.is_null()).cloned() else {
        return Step::Nothing; // notifications/initialized and the like
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match message.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" => json!({
            "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "equipe", "version": version },
            "instructions": "The squad's board. Read it, put work on it, and hand in your ticket here.",
        }),
        "tools/list" => json!({ "tools": tools }),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            return Step::Call { id, name, arguments };
        }
        "ping" => json!({}),
        other => {
            return Step::Reply(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("equipe's MCP server has no method {other}") },
            }));
        }
    };
    Step::Reply(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// The answer to a tool call.
pub fn call_reply(id: Value, result: &ToolResult) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": result.text }],
            "isError": result.is_error,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board;

    #[test]
    fn it_introduces_itself_in_the_clients_revision() {
        let line = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{}}}"#;
        let Step::Reply(reply) = handle(line, &[], "1.0") else { panic!() };
        assert_eq!(reply["id"], 0);
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(reply["result"]["serverInfo"]["name"], "equipe");
        assert!(reply["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn it_lists_the_members_tools() {
        let tools = board::tools(false);
        let Step::Reply(reply) = handle(r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#, &tools, "1") else {
            panic!()
        };
        assert_eq!(reply["id"], "a", "string ids come back as strings");
        assert_eq!(reply["result"]["tools"].as_array().unwrap().len(), tools.len());
    }

    #[test]
    fn a_tool_call_is_handed_on_and_its_result_comes_back_as_text() {
        let line = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"board","arguments":{}}}"#;
        assert_eq!(handle(line, &[], "1"), Step::Call { id: json!(7), name: "board".into(), arguments: json!({}) });
        let reply = call_reply(json!(7), &ToolResult { text: "The board is empty.".into(), is_error: false });
        assert_eq!(reply["result"]["content"][0]["text"], "The board is empty.");
        assert_eq!(reply["result"]["isError"], false);
    }

    #[test]
    fn notifications_get_no_answer_and_unknown_methods_an_error() {
        assert_eq!(handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, &[], "1"), Step::Nothing);
        assert_eq!(handle("not json", &[], "1"), Step::Nothing);
        let Step::Reply(reply) = handle(r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#, &[], "1") else {
            panic!()
        };
        assert_eq!(reply["error"]["code"], -32601);
    }
}
