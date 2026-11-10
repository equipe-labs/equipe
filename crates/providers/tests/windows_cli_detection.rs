//! The Windows runner installs the makers' current npm packages, then proves
//! equipe resolves their real `.cmd`/`.exe` launchers rather than only a fake.

#![cfg(windows)]

#[tokio::test]
async fn installed_claude_and_codex_are_detected_and_started() {
    if std::env::var_os("EQUIPE_TEST_REAL_WINDOWS_CLIS").is_none() {
        eprintln!("skipped: EQUIPE_TEST_REAL_WINDOWS_CLIS isn't set");
        return;
    }
    for binary in ["claude", "codex"] {
        let found = equipe_providers::tools_path::find(binary)
            .unwrap_or_else(|| panic!("the installed {binary} CLI was not found"));
        assert!(found.is_file(), "{}", found.display());
        assert!(
            matches!(found.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("cmd" | "exe" | "bat" | "com")),
            "Windows launcher: {}",
            found.display()
        );
        let output = equipe_providers::tools_path::command(binary)
            .arg("--version")
            .output()
            .await
            .unwrap_or_else(|err| panic!("the installed {binary} CLI did not start: {err}"));
        assert!(output.status.success(), "{binary} --version: {}", String::from_utf8_lossy(&output.stderr));
    }
}
