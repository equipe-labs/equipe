//! A helper's turn through the person's real installed `codex`, as the
//! daemon runs one ([`equipe_core::helpers::request`]):
//!
//! - MOCK-VERIFIED (runs by default when `codex` is installed): on a
//!   stand-in Responses API on this machine with a dummy key, the model asks
//!   to write a file and commit, and Codex's read-only sandbox refuses both.
//!   No provider is called.
//! - REAL (ignored: it spends tokens): on Nebius Token Factory with the key
//!   in `NEBIUS_API_KEY` or saved under equipe's home (never printed), a
//!   helper reads a small repository and answers with path:line, and what
//!   it was asked to change stays unchanged.
//!
//! ```sh
//! cargo test -p equipe-core --test helpers_live -- --ignored --nocapture
//! ```

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use equipe_core::helpers::{self, Limits};
use equipe_providers::codex_app;
use equipe_providers::turn::Activity;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-hl-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Process::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A small repository with one fact to find: where sessions expire.
fn repo() -> PathBuf {
    let dir = scratch("repo");
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.name", "Test"]);
    git(&dir, &["config", "user.email", "test@example.com"]);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("README.md"), "A tiny auth service.\n").unwrap();
    std::fs::write(
        dir.join("src/auth.rs"),
        "use std::time::Duration;\n\n/// How long a session lives.\npub const SESSION_TTL: Duration = Duration::from_secs(3600);\n\npub fn expired(age: Duration) -> bool {\n    age > SESSION_TTL\n}\n",
    )
    .unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

/// A stand-in Responses API: the first request gets a function call that
/// writes a file and commits; the one carrying its output, the answer.
async fn stand_in() -> (String, Arc<Mutex<Vec<Value>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}/v1", listener.local_addr().unwrap().port());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                loop {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 16 * 1024];
                    let end = loop {
                        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break at;
                        }
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
                    let length: usize = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                    let mut body = buf[end + 4..].to_vec();
                    while body.len() < length {
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => body.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    let n = {
                        let mut log = log.lock().unwrap();
                        log.push(body.clone());
                        log.len()
                    };
                    let answered = body["input"].as_array().into_iter().flatten().any(|i| i["type"] == "function_call_output");
                    let has_tool = body["tools"].as_array().into_iter().flatten().any(|t| t["name"] == "exec_command");
                    let item = if has_tool && !answered {
                        let cmd = "printf 'x\\n' > made.txt; git -c user.name=a -c user.email=b@c commit --allow-empty -qm probe; echo done";
                        json!({ "type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "exec_command", "arguments": json!({ "cmd": cmd }).to_string(), "status": "completed" })
                    } else {
                        json!({ "type": "message", "id": format!("msg_{n}"), "role": "assistant", "status": "completed", "content": [{ "type": "output_text", "text": "Couldn't write: the sandbox refused.", "annotations": [] }] })
                    };
                    let id = format!("resp_{n}");
                    let usage = json!({ "input_tokens": 100, "input_tokens_details": { "cached_tokens": 0 }, "output_tokens": 10, "output_tokens_details": { "reasoning_tokens": 0 }, "total_tokens": 110 });
                    let events = [
                        json!({ "type": "response.created", "response": { "id": id, "object": "response", "status": "in_progress", "output": [] } }),
                        json!({ "type": "response.output_item.done", "output_index": 0, "item": item }),
                        json!({ "type": "response.completed", "response": { "id": id, "object": "response", "status": "completed", "output": [item], "usage": usage } }),
                    ];
                    let sse: String = events.iter().map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap())).collect();
                    let reply = format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{sse}", sse.len());
                    if stream.write_all(reply.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (base, seen)
}

/// The real codex, in a Codex home of its own: no sign-in, none of the person's config.
fn real_codex() -> Option<(String, PathBuf)> {
    let binary = std::env::var("EQUIPE_CODEX").ok().filter(|b| !b.is_empty()).unwrap_or_else(|| "codex".to_owned());
    let real = codex_app::find(&binary)?;
    Some((real.to_string_lossy().into_owned(), scratch("codex-home")))
}

/// One helper's turn as the daemon runs it, with what it showed.
async fn helper_turn(codex: &str, cwd: &Path, brief: &str, model: &str, env: &[(String, String)], provider: &codex_app::Provider) -> (equipe_providers::Result<codex_app::Turn>, Vec<Activity>) {
    let instructions = helpers::instructions("backend", cwd, &Limits::DEFAULT);
    let request = helpers::request(brief, Some(model), None, &instructions, env, provider);
    let mut shown = Vec::new();
    let turn = tokio::time::timeout(Duration::from_secs(300), codex_app::run_turn(codex, cwd, request, None, None, None, |a| shown.push(a))).await.expect("the helper's turn ends");
    (turn, shown)
}

#[tokio::test]
async fn mock_verified_a_helper_cannot_write_or_commit_through_the_real_codex() {
    let Some((codex, codex_home)) = real_codex() else {
        eprintln!("skipped: no `codex` installed (or $EQUIPE_CODEX)");
        return;
    };
    let (base, seen) = stand_in().await;
    let repo = repo();
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let env = vec![("CODEX_HOME".to_owned(), codex_home.to_string_lossy().into_owned()), ("NEBIUS_API_KEY".to_owned(), "test-not-a-key".to_owned())];
    let provider = codex_app::Provider {
        id: "nebius".into(),
        name: "Nebius Token Factory".into(),
        base_url: base,
        env_key: "NEBIUS_API_KEY".into(),
        config: vec!["model_context_window=131072".into()],
        wire: equipe_providers::relay::Wire::Responses,
    };
    let (turn, shown) = helper_turn(&codex, &repo, "Write made.txt and commit it.", "nvidia/nemotron-mock", &env, &provider).await;
    let requests = seen.lock().unwrap().clone();
    let tools: Vec<String> = requests.first().and_then(|r| r["tools"].as_array()).into_iter().flatten().filter_map(|t| t["name"].as_str().map(str::to_owned)).collect();
    let context = format!("\ntools: {tools:?}\nshown: {shown:#?}\nturn: {turn:?}");
    eprintln!("MOCK-VERIFIED (not a real Token Factory run): tools {tools:?}");
    let turn = turn.unwrap_or_else(|e| panic!("{e}{context}"));
    assert_eq!(turn.result.text, "Couldn't write: the sandbox refused.", "{context}");
    let ran = shown.iter().find_map(|a| if let Activity::ToolDone(d) = a { (d.name == "Bash").then_some(d) } else { None }).unwrap_or_else(|| panic!("the command ran, in the sandbox{context}"));
    eprintln!("its command printed: {}", ran.output);
    assert!(!repo.join("made.txt").exists(), "the read-only sandbox refused the write{context}");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head, "and the commit{context}");
    assert!(!tools.iter().any(|t| t.contains("equipe") || t.starts_with("mcp__")), "no board tools, no MCP server at all{context}");
    assert!(!tools.iter().any(|t| t.contains("spawn_agent") || t.contains("apply_patch")), "no subagents, no patch tool{context}");
    let _ = std::fs::remove_dir_all(&codex_home);
}

#[tokio::test]
#[ignore = "spends tokens on the real Token Factory: run by hand with NEBIUS_API_KEY or a saved key"]
async fn real_a_helper_on_token_factory_reads_answers_with_evidence_and_changes_nothing() {
    let home = equipe_providers::credentials::equipe_home();
    let cfg = equipe_core::nemotron::Config::from_env_or_saved(&home).expect("NEBIUS_API_KEY, or a key saved with `equipe login nebius`");
    let model = equipe_core::nemotron::agent_model(&cfg).await.expect("a Nemotron model");
    let Some((codex, codex_home)) = real_codex() else { panic!("no codex installed") };
    let repo = repo();
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let mut env = vec![cfg.codex_env()];
    env.push(("CODEX_HOME".to_owned(), codex_home.to_string_lossy().into_owned()));
    let provider = cfg.codex_provider();
    let key = cfg.codex_env().1;
    let scrub = |text: &str| if key.len() >= 4 { text.replace(&key, "[redacted]") } else { text.to_owned() };
    let brief = "Find where a session's lifetime is set in this repository, and its value. Then also try to create a file \
named probe.txt with the word hi in it, and say whether you could.";
    let started = std::time::Instant::now();
    let (turn, shown) = helper_turn(&codex, &repo, brief, &model, &env, &provider).await;
    let turn = turn.map_err(|e| scrub(&e.to_string())).expect("the turn runs");
    let steps: Vec<String> = shown.iter().filter_map(|a| if let Activity::Tool { name, target, .. } = a { Some(format!("{name} {}", target.clone().unwrap_or_default())) } else { None }).collect();
    println!("REAL RUN on Token Factory, model {model}, {} ms, {} request(s), usage {:?}", started.elapsed().as_millis(), turn.provider_requests.len(), turn.result.usage);
    println!("steps: {}", scrub(&format!("{steps:#?}")));
    println!("answer:\n{}", scrub(&turn.result.text));
    assert!(turn.result.text.contains("src/auth.rs"), "path evidence: {}", turn.result.text);
    assert!(!repo.join("probe.txt").exists(), "read-only: nothing written");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&repo, &["status", "--porcelain"]), "", "the worktree as it was");
    let _ = std::fs::remove_dir_all(&codex_home);
}
