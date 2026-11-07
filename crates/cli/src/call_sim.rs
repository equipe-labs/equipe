//! `equipe call-sim`: the call's loop (`equipe_core::call`) run in-process,
//! without a daemon, against a fixed roster — type what you'd say, see where
//! it goes. Nothing is sent anywhere: submits are only printed.

use std::collections::VecDeque;
use std::io::Write as _;

use equipe_core::call::judge::{self, LlmConfig, RosterMember};
use equipe_core::call::{self, Call, Effect};
use tokio::io::{AsyncBufReadExt as _, BufReader};

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn roster() -> Vec<RosterMember> {
    ["pm", "designer", "backend", "frontend", "qa"]
        .iter()
        .enumerate()
        .map(|(i, role)| RosterMember { session: i as u64 + 1, name: (*role).to_owned(), role: (*role).to_owned(), tickets: Vec::new() })
        .collect()
}

fn prompt() {
    print!("you> ");
    let _ = std::io::stdout().flush();
}

pub async fn run(squad: String, no_llm: bool) -> Result<(), String> {
    let llm = if no_llm { LlmConfig::off() } else { LlmConfig::from_env() };
    let roster = roster();
    let (mut call, effects) = Call::new(call::new_id(), squad, now_ms());
    let mut queue: VecDeque<Effect> = effects.into();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut eof = false;
    loop {
        // Everything the call asked for so far, judging in-line.
        while let Some(effect) = queue.pop_front() {
            match effect {
                Effect::Publish(event) => println!("event {}", serde_json::to_string(&event).map_err(|e| e.to_string())?),
                Effect::Submit { session, text } => {
                    let name = roster.iter().find(|m| m.session == session).map_or("?", |m| m.name.as_str());
                    println!("submit → {name} (session {session}): {text}");
                }
                Effect::Judge(input) => {
                    let judged = judge::judge(&input, &llm).await;
                    println!("judge: by={:?} ms={} input_bytes={}", judged.by, judged.millis, judged.input_bytes);
                    queue.extend(call.judged(judged, now_ms()));
                }
            }
        }
        if call.ended() || eof {
            return Ok(());
        }
        prompt();
        match lines.next_line().await.map_err(|e| e.to_string())? {
            Some(line) => queue.extend(call.hear(&line, roster.clone(), now_ms())),
            None => {
                println!();
                eof = true;
                queue.extend(call.hang_up());
            }
        }
    }
}
