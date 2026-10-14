//! Dev tool: connect the way the window does and print what the link says.
//!
//!   cargo run -p equipe-desktop --example remote_probe -- <local | user@host[:port]> [--trust] [--seconds N]
//!
//! Questions from `ssh` are answered from the environment, never stored:
//! `EQUIPE_PROBE_PASSWORD` for a password / code, `--trust` to accept a first-seen
//! host key (the window would show its fingerprint and ask). `EQUIPE_SSH_OPTIONS`
//! adds `ssh` options (e.g. `-o UserKnownHostsFile=/tmp/probe-known-hosts`).
#![allow(dead_code)]

#[path = "../src/askpass.rs"]
mod askpass;
#[path = "../src/daemon.rs"]
mod daemon;
#[path = "../src/failure.rs"]
mod failure;
#[path = "../src/install.rs"]
mod install;
#[path = "../src/ssh.rs"]
mod ssh;
#[path = "../src/target.rs"]
mod target;
#[path = "../src/wire.rs"]
mod wire;

use std::time::Duration;

use daemon::{Backoff, Spawned, Target, Update};
use target::Endpoint;

fn main() {
    // ssh runs this same binary to ask a question.
    if let Some(code) = askpass::helper() {
        std::process::exit(code);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--make-squad") {
        // A squad with one project on disk (no members), for trying a real daemon in a scratch `EQUIPE_HOME`.
        let home = equipe_providers::credentials::equipe_home();
        let folder = std::path::PathBuf::from(&args[2]).canonicalize().expect("a folder");
        equipe_core::squads::create(&home, &args[1], &folder).expect("create the squad");
        return;
    }
    if args.first().is_some_and(|a| a == "--print-bridge-fixture") {
        // The three lines a bridge says, for a stand-in `equipe` script on a test machine.
        use equipe_core::protocol::rpc::{self, Attached, Hello, Message, SquadEntry, Squads};
        let attached = Attached { bridge: "fixture".into(), daemon: Some("fixture".into()), protocol: rpc::PROTOCOL, started: false };
        let squads = Squads { squads: vec![SquadEntry { name: "demo".into(), projects: vec![], home: "/tmp/demo".into() }], last: Some("demo".into()) };
        let hello = Hello { version: "fixture".into(), snapshot: equipe_core::protocol::Snapshot { squad: "demo".into(), ..Default::default() } };
        print!("{}{}{}", Message::notification(rpc::ATTACHED, &attached).to_line(), Message::response(1, squads).to_line(), Message::response(2, hello).to_line());
        return;
    }
    let trust = args.iter().any(|a| a == "--trust");
    let seconds = args.iter().position(|a| a == "--seconds").and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(20u64);
    let endpoint = Endpoint::parse(args.first().map_or("local", String::as_str)).expect("a target");
    let remote = endpoint.is_remote();
    println!("target: {}", endpoint.label());
    let backoff = if remote { Backoff::remote() } else { Backoff::default() };
    let Spawned { link, mut updates, mut prompts } = daemon::spawn(Target::new(endpoint.clone(), "probe"), backoff);
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async move {
        let deadline = tokio::time::sleep(Duration::from_secs(seconds));
        tokio::pin!(deadline);
        let mut events = 0;
        loop {
            tokio::select! {
                () = &mut deadline => break,
                prompt = prompts.recv() => {
                    let Some(prompt) = prompt else { break };
                    println!("ask: {:?}", prompt.ask);
                    let answer = match &prompt.ask {
                        askpass::Ask::HostKey { .. } | askpass::Ask::Confirm { .. } if trust => askpass::Answer::Yes,
                        askpass::Ask::Secret { .. } => match std::env::var("EQUIPE_PROBE_PASSWORD") {
                            Ok(p) => askpass::Answer::Text(p),
                            Err(_) => askpass::Answer::Cancel,
                        },
                        _ => askpass::Answer::Cancel,
                    };
                    println!("  -> {}", match &answer { askpass::Answer::Text(_) => "(secret)".to_owned(), other => format!("{other:?}") });
                    let _ = prompt.reply.send(answer);
                }
                update = updates.recv() => match update {
                    Some(Update::Attached(hello)) => println!("attached: daemon {} squad {} ({} sessions, {} tickets)", hello.version, hello.snapshot.squad, hello.snapshot.sessions.len(), hello.snapshot.tickets.len()),
                    Some(Update::Squads(s)) => println!("squads: {:?} last {:?}", s.squads.iter().map(|s| &s.name).collect::<Vec<_>>(), s.last),
                    Some(Update::Bridge(b)) => println!("bridge: {b:?}"),
                    Some(Update::Event(_)) => events += 1,
                    Some(Update::Down(f)) => { println!("down: {f:?}\n  guide: {}", f.guide(&endpoint).title); if f.waits_for_person() { break } }
                    None => break,
                },
            }
        }
        println!("events seen: {events}");
        drop(link);
    });
}
