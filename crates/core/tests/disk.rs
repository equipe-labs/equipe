//! The daemon's look at the disk and its clean, through the runtime: off the
//! loop (a debug build would stop on the core loop's assert), answered to the
//! asker, and to the screens when a screen asked. Only temporary folders.

#![cfg(unix)]

use std::path::PathBuf;
use std::time::Duration;

use equipe_core::disk::{DiskKind, DiskWhy};
use equipe_core::protocol::{Command, Event};
use equipe_core::runtime::{self, Config};
use equipe_core::squads;
use equipe_providers::CredentialStore;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("equipe-disk-rt-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

#[tokio::test]
async fn a_look_and_a_clean_go_through_the_daemon_and_off_its_loop() {
    let home = scratch("home");
    let project = scratch("project");
    // The person's caches are looked at under $HOME: an empty one here, not the real one.
    // SAFETY: this binary's one test, before anything else runs.
    unsafe { std::env::set_var("HOME", scratch("person")) };
    squads::create(&home, "acme", &project).unwrap();
    // A member's build folder nobody builds in, and one a link out of equipe's folders.
    let builds = home.join("cache").join("build").join("acme");
    std::fs::create_dir_all(builds.join("frontend")).unwrap();
    std::fs::write(builds.join("frontend").join("blob"), vec![1u8; 2_000_000]).unwrap();
    let outside = scratch("outside");
    std::fs::write(outside.join("keep"), b"the person's").unwrap();
    std::os::unix::fs::symlink(&outside, builds.join("backend")).unwrap();
    let config = Config {
        store: CredentialStore::new(&home),
        home: home.clone(),
        client_version: "test".into(),
        claude_binary: "/bin/false".into(),
        persist: true,
        retention_days: 90,
        equipe_binary: None,
        npx: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        lang: equipe_core::i18n::Lang::En,
    };
    let (handle, mut events) = runtime::start(config);

    let report = handle.disk_report().await.expect("the daemon answers");
    assert!(report.status.total > 0);
    let item = report.items.iter().find(|i| i.kind == DiskKind::BuildCache).expect("the build folder");
    assert!(item.safe && item.why == DiskWhy::Rebuildable, "{item:?}");
    assert!(!report.items.iter().any(|i| i.path.starts_with(&outside) || i.path == builds.join("backend")));

    // A screen's clean: told to the screens, then a fresh look.
    handle.send(Command::DiskClean { ids: vec![item.id.clone(), format!("build_cache:{}", outside.display())] });
    let (mut cleaned, mut looked) = (None, false);
    let wait = async {
        while let Some(event) = events.recv().await {
            match event {
                Event::DiskCleaned { cleaned: c } => cleaned = Some(c),
                Event::DiskReport { .. } if cleaned.is_some() => looked = true,
                _ => {}
            }
            if looked {
                return;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), wait).await.expect("cleaned and looked again");
    let cleaned = cleaned.unwrap();
    assert_eq!(cleaned.removed, vec![item.id.clone()]);
    assert!(cleaned.freed >= 2_000_000);
    assert!(!builds.join("frontend").exists());
    assert!(outside.join("keep").exists(), "nothing outside equipe's folders");
    // The loop still answers.
    let busy = tokio::time::timeout(Duration::from_secs(5), handle.busy()).await.expect("busy answers");
    assert_eq!(busy.stalled, None);
}
