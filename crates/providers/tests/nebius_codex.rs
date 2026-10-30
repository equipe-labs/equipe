//! MOCK-VERIFIED, not a real Token Factory run: the person's real, installed
//! `codex app-server`, driven through equipe's own [`codex_app::run_turn`],
//! on the provider config a member on Nebius Token Factory gets
//! ([`codex_app::provider_args`]) — its `base_url` pointed at a stand-in
//! Responses API on this machine, its key a dummy (`test-not-a-key`).
//!
//! The stand-in answers the first request with a function call that runs a
//! shell command writing a file in the turn's folder, and the next with the
//! final message. The test asserts that the command ran there, that the turn
//! completed with that message, and what Codex sent: `POST /v1/responses`,
//! `Authorization: Bearer test-not-a-key`, the `nvidia/…` model, function
//! tools only, the call's output handed back by `call_id` — and that the key
//! never reached the command. What it sent is printed (`--nocapture`), and
//! on a failure, as the evidence of how the plumbing goes.
//!
//! Skipped when `codex` isn't installed (or `$EQUIPE_CODEX` doesn't name one).
//! Nothing leaves this machine: no Nebius call is made.

#![cfg(unix)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use equipe_providers::claude_code::Access;
use equipe_providers::codex_app::{self, TurnRequest};
use equipe_providers::turn::Activity;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const KEY: &str = "test-not-a-key";
const MODEL: &str = "nvidia/nemotron-mock-super";
const COMMAND: &str = "printf 'written by nemotron' > made.txt && printf '%s' \"${NEBIUS_API_KEY:-absent}\" > key.txt";
const FINAL: &str = "Done: made.txt is written.";

/// One request the stand-in got: method, path, headers (names lowercased), body.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// The request in short, for the log: what matters to a provider.
    fn shape(&self) -> Value {
        let tools: Vec<Value> = self.body["tools"].as_array().into_iter().flatten().map(|t| json!([t["type"], t["name"]])).collect();
        let input: Vec<Value> = self.body["input"].as_array().into_iter().flatten().map(|i| json!([i["type"], i["role"], i["call_id"]])).collect();
        let mut keys: Vec<&String> = self.body.as_object().map(|o| o.keys().collect()).unwrap_or_default();
        keys.sort();
        let auth = self.header("authorization").map(|a| if a == format!("Bearer {KEY}") { "Bearer test-not-a-key".to_owned() } else { "(other)".to_owned() });
        json!({
            "request": format!("{} {}", self.method, self.path),
            "authorization": auth,
            "accept": self.header("accept"),
            "body_keys": keys,
            "model": self.body["model"],
            "stream": self.body["stream"],
            "store": self.body["store"],
            "tool_choice": self.body["tool_choice"],
            "parallel_tool_calls": self.body["parallel_tool_calls"],
            "reasoning": self.body["reasoning"],
            "include": self.body["include"],
            "service_tier": self.body["service_tier"],
            "tools": tools,
            "input": input,
        })
    }
}

/// The SSE events of one response holding `item`.
fn sse(id: &str, item: &Value) -> String {
    let usage = json!({ "input_tokens": 120, "input_tokens_details": { "cached_tokens": 0 }, "output_tokens": 12, "output_tokens_details": { "reasoning_tokens": 0 }, "total_tokens": 132 });
    let events = [
        json!({ "type": "response.created", "response": { "id": id, "object": "response", "status": "in_progress", "output": [] } }),
        json!({ "type": "response.output_item.added", "output_index": 0, "item": item }),
        json!({ "type": "response.output_item.done", "output_index": 0, "item": item }),
        json!({ "type": "response.completed", "response": { "id": id, "object": "response", "status": "completed", "output": [item], "usage": usage } }),
    ];
    events.iter().map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap_or_default())).collect()
}

/// The stand-in's answer to `seen`: the turn's first request gets the call,
/// a request carrying its output the final message; anything else a message.
fn answer(seen: &Seen, n: usize) -> (u16, &'static str, String) {
    if seen.method == "GET" {
        return (200, "application/json", json!({ "object": "list", "data": [{ "id": MODEL, "object": "model" }] }).to_string());
    }
    let input = seen.body["input"].as_array().cloned().unwrap_or_default();
    let has_tool = seen.body["tools"].as_array().into_iter().flatten().any(|t| t["name"] == "exec_command");
    let answered = input.iter().any(|i| i["type"] == "function_call_output");
    let item = if has_tool && !answered {
        json!({ "type": "function_call", "id": "fc_1", "call_id": "call_mock_1", "name": "exec_command", "arguments": json!({ "cmd": COMMAND }).to_string(), "status": "completed" })
    } else {
        json!({ "type": "message", "id": format!("msg_{n}"), "role": "assistant", "status": "completed", "content": [{ "type": "output_text", "text": FINAL, "annotations": [] }] })
    };
    (200, "text/event-stream", sse(&format!("resp_mock_{n}"), &item))
}

/// Read one HTTP/1.1 request off `stream`; `None` when it closed first.
async fn read_request(stream: &mut tokio::net::TcpStream) -> Option<Seen> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    let head_end = loop {
        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let (method, path) = (first.next()?.to_owned(), first.next()?.to_owned());
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned())).collect();
    let length: usize = headers.iter().find(|(n, _)| n == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    // Codex may compress what it sends; this stand-in reads it plain or not at all.
    let body = if headers.iter().any(|(n, v)| n == "content-encoding" && v != "identity") {
        json!({ "compressed": true })
    } else {
        serde_json::from_slice(&body).unwrap_or(Value::Null)
    };
    Some(Seen { method, path, headers, body })
}

/// The stand-in Responses API on a free local port: its base URL, and what it got.
async fn stand_in() -> (String, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::<Seen>::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                while let Some(request) = read_request(&mut stream).await {
                    let n = {
                        let mut log = log.lock().unwrap();
                        log.push(request.clone());
                        log.len()
                    };
                    let (status, kind, body) = answer(&request, n);
                    let head = format!("HTTP/1.1 {status} OK\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nx-request-id: mock-{n}\r\n\r\n", body.len());
                    if stream.write_all(head.as_bytes()).await.is_err() || stream.write_all(body.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (format!("http://127.0.0.1:{port}/v1"), seen)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("equipe-nebius-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn installed_codex() -> Option<String> {
    let binary = std::env::var("EQUIPE_CODEX").ok().filter(|b| !b.is_empty()).unwrap_or_else(|| codex_app::DEFAULT_BINARY.to_owned());
    codex_app::find(&binary).map(|p| p.to_string_lossy().into_owned())
}

#[tokio::test]
async fn the_real_codex_runs_a_members_turn_on_a_stand_in_token_factory() {
    let Some(codex) = installed_codex() else {
        eprintln!("skipped: no `codex` installed (or $EQUIPE_CODEX)");
        return;
    };
    let (base_url, seen) = stand_in().await;
    let cwd = scratch("work");
    // Codex's own home, empty: no ChatGPT sign-in, none of the person's config or threads.
    let codex_home = scratch("codex-home");
    let env = vec![("CODEX_HOME".to_owned(), codex_home.to_string_lossy().into_owned()), ("NEBIUS_API_KEY".to_owned(), KEY.to_owned())];
    let args = codex_app::provider_args("nebius", "Nebius Token Factory", &base_url, "NEBIUS_API_KEY");
    assert!(!args.iter().any(|a| a.contains(KEY)), "the key never goes on the command line");
    let request = TurnRequest { prompt: "Write made.txt.", model: Some(MODEL), access: Access::EditFiles, env: &env, args: &args, ..Default::default() };

    let mut activities = Vec::new();
    let run = codex_app::run_turn(&codex, &cwd, request, None, None, None, |a| activities.push(a));
    let outcome = tokio::time::timeout(Duration::from_secs(120), run).await;

    let seen = seen.lock().unwrap().clone();
    let shapes: Vec<Value> = seen.iter().map(Seen::shape).collect();
    let evidence = serde_json::to_string_pretty(&shapes).unwrap();
    let made = std::fs::read_to_string(cwd.join("made.txt")).ok();
    let key_seen = std::fs::read_to_string(cwd.join("key.txt")).ok();
    // The evidence, shown with --nocapture and on any failure below.
    eprintln!("MOCK-VERIFIED (not a real Token Factory run). What codex sent the stand-in:\n{evidence}");
    eprintln!("activities: {activities:?}");
    let context = format!("\nrequests:\n{evidence}\nactivities: {activities:?}\noutcome: {outcome:?}");

    let turn = outcome.unwrap_or_else(|_| panic!("the turn didn't end in 120s{context}")).unwrap_or_else(|err| panic!("the turn failed: {err}{context}"));
    assert_eq!(turn.result.status, "completed", "{context}");
    assert_eq!(turn.result.text, FINAL, "{context}");
    assert!(turn.thread_id.is_some(), "{context}");
    assert_eq!(made.as_deref(), Some("written by nemotron"), "the command ran in the turn's folder{context}");
    assert_eq!(key_seen.as_deref(), Some("absent"), "the key stays out of the agent's commands{context}");
    assert!(activities.iter().any(|a| matches!(a, Activity::Model(m) if m == MODEL)), "{context}");
    assert!(activities.iter().any(|a| matches!(a, Activity::ToolDone(d) if d.name == "Bash" && d.ok)), "the command showed as Bash{context}");

    let turns: Vec<&Seen> = seen.iter().filter(|s| s.method == "POST" && s.path == "/v1/responses").collect();
    assert!(turns.len() >= 2, "a request for the call and one with its output{context}");
    for request in &turns {
        assert_eq!(request.header("authorization"), Some(format!("Bearer {KEY}").as_str()), "{context}");
        assert_eq!(request.body["model"], MODEL, "{context}");
        assert_eq!(request.body["stream"], true, "{context}");
        let tools = request.body["tools"].as_array().cloned().unwrap_or_default();
        assert!(tools.iter().all(|t| t["type"] == "function"), "function tools only{context}");
    }
    let back = turns.iter().find(|s| s.body["input"].as_array().into_iter().flatten().any(|i| i["type"] == "function_call_output")).unwrap_or_else(|| panic!("the call's output went back{context}"));
    let output = back.body["input"].as_array().unwrap().iter().find(|i| i["type"] == "function_call_output").unwrap();
    assert_eq!(output["call_id"], "call_mock_1", "{context}");
    let _ = std::fs::remove_dir_all(&cwd);
    let _ = std::fs::remove_dir_all(&codex_home);
}

#[test]
fn a_providers_args_point_codex_at_it_and_keep_the_requests_plain() {
    let args = codex_app::provider_args("nebius", "Nebius Token Factory", "https://api.tokenfactory.nebius.com/v1/", "NEBIUS_API_KEY");
    let pairs: Vec<&str> = args.chunks(2).map(|p| {
        assert_eq!(p[0], "-c");
        p[1].as_str()
    }).collect();
    for want in [
        "model_provider=\"nebius\"",
        "model_providers.nebius.name=\"Nebius Token Factory\"",
        "model_providers.nebius.base_url=\"https://api.tokenfactory.nebius.com/v1/\"",
        "model_providers.nebius.env_key=\"NEBIUS_API_KEY\"",
        "model_providers.nebius.wire_api=\"responses\"",
        "model_providers.nebius.requires_openai_auth=false",
        "web_search=\"disabled\"",
        "features.multi_agent=false",
        "features.remote_compaction_v2=false",
    ] {
        assert!(pairs.contains(&want), "{want} in {pairs:?}");
    }
}
