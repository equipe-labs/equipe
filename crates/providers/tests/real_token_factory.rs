//! A REAL run, not a mock: a member's turn on Nebius Token Factory, through
//! the same path the daemon takes — the person's installed `codex
//! app-server`, driven by [`codex_app::run_turn`], with Token Factory as its
//! provider behind equipe's relay, asked in Chat Completions.
//!
//! Ignored by default: it spends tokens. Run it by hand with the key in the
//! environment (never printed):
//!
//! ```sh
//! NEBIUS_API_KEY=… EQUIPE_NEBIUS_MODEL=nvidia/… \
//!   cargo test -p equipe-providers --test real_token_factory -- --ignored --nocapture
//! ```
//!
//! It prints the evidence: the model, each request's status and Token
//! Factory's request id, the time, the tokens, and what the member did.

#![cfg(unix)]

use std::time::{Duration, Instant};

use equipe_providers::claude_code::Access;
use equipe_providers::codex_app::{self, TurnRequest};
use equipe_providers::relay::Wire;
use equipe_providers::turn::Activity;

const BASE_URL: &str = "https://api.tokenfactory.nebius.com/v1";

#[tokio::test]
#[ignore = "spends tokens on the real Token Factory: run by hand with NEBIUS_API_KEY"]
async fn a_member_turn_on_the_real_token_factory() {
    let key = std::env::var("NEBIUS_API_KEY").ok().filter(|k| !k.trim().is_empty()).expect("NEBIUS_API_KEY");
    let model = std::env::var("EQUIPE_NEBIUS_MODEL").ok().filter(|m| !m.trim().is_empty()).expect("EQUIPE_NEBIUS_MODEL");
    let base_url = std::env::var("EQUIPE_NEBIUS_BASE_URL").ok().filter(|b| !b.trim().is_empty()).unwrap_or_else(|| BASE_URL.to_owned());
    let wire = std::env::var("EQUIPE_NEBIUS_WIRE").ok().and_then(|w| Wire::parse(&w)).unwrap_or_default();
    let codex = codex_app::find(codex_app::DEFAULT_BINARY).expect("codex installed").to_string_lossy().into_owned();

    let root = std::env::temp_dir().join(format!("equipe-real-{}", std::process::id()));
    let (cwd, codex_home) = (root.join("work"), root.join("codex-home"));
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&codex_home).unwrap();
    let cwd = cwd.canonicalize().unwrap();
    // An empty Codex home: no ChatGPT sign-in, none of the person's config.
    let env = vec![("CODEX_HOME".to_owned(), codex_home.to_string_lossy().into_owned()), ("NEBIUS_API_KEY".to_owned(), key.clone())];
    let provider = codex_app::Provider {
        id: "nebius".into(),
        name: "Nebius Token Factory".into(),
        base_url: base_url.trim_end_matches('/').to_owned(),
        env_key: "NEBIUS_API_KEY".into(),
        config: vec!["model_context_window=131072".into()],
        wire,
    };
    let prompt = "In the current folder, create a file named hello.py that prints exactly `Hello from Nemotron`, then run `python3 hello.py` to check it. Use shell commands. Reply with one short sentence when done.";
    let request = TurnRequest { prompt, model: Some(&model), access: Access::EditFiles, env: &env, provider: Some(&provider), ..Default::default() };

    let started = Instant::now();
    let mut steps = Vec::new();
    let run = codex_app::run_turn(&codex, &cwd, request, None, None, None, |a| match a {
        Activity::Tool { name, target, .. } => steps.push(format!("tool {name} {}", target.unwrap_or_default())),
        Activity::ToolDone(d) => steps.push(format!("done {} ok={} exit={:?}", d.name, d.ok, d.exit)),
        Activity::Model(m) => steps.push(format!("model {m}")),
        _ => {}
    });
    let outcome = tokio::time::timeout(Duration::from_secs(300), run).await;
    let wall = started.elapsed();
    let made = std::fs::read_to_string(cwd.join("hello.py")).ok();
    let ran = std::process::Command::new("python3").arg("hello.py").current_dir(&cwd).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());

    let scrub = |s: String| if key.len() >= 4 { s.replace(&key, "[redacted]") } else { s };
    println!("REAL RUN on {base_url} (wire {wire:?}), model {model}, codex {codex}");
    println!("wall time: {} ms", wall.as_millis());
    println!("steps: {}", scrub(format!("{steps:#?}")));
    match &outcome {
        Ok(Ok(turn)) => {
            let statuses: Vec<u16> = turn.provider_requests.iter().map(|r| r.status).collect();
            let ids: Vec<&str> = turn.provider_requests.iter().filter_map(|r| r.request_id.as_deref()).collect();
            let ms: u64 = turn.provider_requests.iter().map(|r| r.ms).sum();
            println!("requests: {} {statuses:?}", turn.provider_requests.len());
            println!("request ids: {ids:?}");
            println!("ms on requests: {ms}");
            println!("usage: {:?}", turn.result.usage);
            println!("status: {}", turn.result.status);
            println!("answer: {}", scrub(turn.result.text.clone()));
        }
        Ok(Err(err)) => println!("turn failed: {}", scrub(err.to_string())),
        Err(_) => println!("turn didn't end in 300 s"),
    }
    println!("hello.py: {}", scrub(format!("{made:?}")));
    println!("python3 hello.py: {ran:?}");
    let _ = std::fs::remove_dir_all(&root);
    let turn = outcome.expect("in time").expect("the turn ran");
    assert_eq!(turn.result.status, "completed");
    assert!(turn.provider_requests.iter().all(|r| r.status == 200));
    assert_eq!(ran.as_deref(), Some("Hello from Nemotron"));
}
