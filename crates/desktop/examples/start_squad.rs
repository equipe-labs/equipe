//! Brings a PM and a frontend member into a squad on a real daemon, so the
//! desktop app has someone to talk to when it is tried against the real thing
//! (`EQUIPE_HOME` pointing at a scratch home, so no one's own squads are touched).
//!
//! ```text
//! export EQUIPE_HOME=$(mktemp -d)
//! equipe daemon start
//! cargo run -p equipe-desktop --example start_squad -- demo /path/to/a/git/repo [/another/repo…]
//! EQUIPE_SQUAD=demo cargo run -p equipe-desktop
//! ```

use std::path::PathBuf;
use std::time::Duration;

use equipe_core::gateway::{self, Client};
use equipe_core::squads;
use equipe_core::protocol::{Access, Command, Engine, Event, SessionSpec, new_session_id};
use equipe_providers::credentials::equipe_home;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let squad = args.next().unwrap_or_else(|| "demo".into());
    let folders: Vec<PathBuf> = args.map(|a| PathBuf::from(a).canonicalize().expect("a folder that exists")).collect();
    let cwd = folders.first().cloned().expect("at least one repository");
    let home = equipe_home();
    // The squad and its projects are on disk; the daemon reads them when we attach.
    for (n, folder) in folders.iter().enumerate() {
        let done = if n == 0 && !squads::exists(&home, &squad) { squads::create(&home, &squad, folder) } else { squads::add_project(&home, &squad, folder) };
        done.expect("the squad's projects");
    }
    let socket = gateway::socket_path(&home);
    let (client, hello) = Client::connect(&socket, "start_squad", &squad).await.expect("a running daemon (equipe daemon start)");
    eprintln!("daemon {} · {} members already", hello.version, hello.snapshot.sessions.len());
    let (to_daemon, mut events) = client.split();
    for (role, access) in [("pm", Access::ReadOnly), ("frontend", Access::EditFiles)] {
        if hello.snapshot.sessions.iter().any(|s| s.member.as_ref().is_some_and(|m| m.role == role)) {
            continue;
        }
        let spec = SessionSpec { engine: Some(Engine::Claude), model: None, access, cwd: cwd.clone(), role: Some(role.into()), squad: Some(squad.clone()) };
        to_daemon.send(Command::Start { session: new_session_id(), spec });
    }
    let wait = tokio::time::sleep(Duration::from_secs(15));
    tokio::pin!(wait);
    loop {
        tokio::select! {
            () = &mut wait => break,
            event = events.recv() => match event {
                Some(Event::SessionStarted { member, .. }) => eprintln!("started {member:?}"),
                Some(Event::Error { message, .. }) => eprintln!("error: {message}"),
                Some(_) => {}
                None => break,
            },
        }
    }
}
