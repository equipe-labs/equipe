//! Whether a sign-in kept "for the session" (a cookie with no expiry date)
//! survives the browser host's Chrome restarting, on a real Chrome with a
//! profile of its own — never the person's. Chrome drops such cookies when
//! it starts after a clean exit, which is how the host's Chrome exits each
//! time the daemon stops; the host marks the last exit as a crash before
//! starting it (`assets/keep-sessions.js`) and Chrome brings them back, with
//! no old tab reopened. Needs Node, npx and Google Chrome (or set
//! `EQUIPE_BROWSER_CHANNEL`), so it runs only when asked:
//!
//! ```sh
//! cargo test -p equipe-core --test session_cookies_live -- --ignored --nocapture
//! ```

use std::io::Write as _;

/// Started as the browser host is (`npx -p playwright node …`): sets a
/// session cookie and a lasting one from a local server, closes Chrome as
/// the host does, starts it again — as before, and with the host's step —
/// and says which cookies came back and which tabs opened.
const SCRIPT: &str = r#"
const http = require('node:http');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const roots = (process.env.PATH || '').split(path.delimiter).filter((p) => p.endsWith(path.join('node_modules', '.bin'))).map((p) => path.dirname(p));
const { chromium } = require(require.resolve('playwright', { paths: roots }));
__KEEP_SESSIONS__
const server = http.createServer((req, res) => {
  if (req.url === '/sign-in') res.setHeader('Set-Cookie', ['session=1; Path=/', 'lasting=1; Path=/; Max-Age=86400']);
  res.end(req.headers.cookie || '');
});
const options = (extra) => ({ channel: process.env.EQUIPE_BROWSER_CHANNEL || 'chrome', headless: true, ignoreDefaultArgs: ['--enable-automation'], args: ['--disable-blink-features=AutomationControlled', '--test-type', ...extra] });
const once = async (keep) => {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'equipe-session-live-'));
  const base = `http://127.0.0.1:${server.address().port}`;
  try {
    let context = await chromium.launchPersistentContext(profile, options(keep ? ['--hide-crash-restore-bubble'] : []));
    await (context.pages()[0] || await context.newPage()).goto(`${base}/sign-in`);
    await context.close();
    const marked = keep ? keepSessions(profile, fs, path) : 'not asked';
    context = await chromium.launchPersistentContext(profile, options(keep ? ['--hide-crash-restore-bubble'] : []));
    const page = context.pages()[0] || await context.newPage();
    await page.goto(`${base}/`);
    const cookies = (await page.textContent('body')) || '';
    const tabs = context.pages().map((p) => p.url());
    await context.close();
    return { marked, cookies, tabs };
  } finally {
    fs.rmSync(profile, { recursive: true, force: true });
  }
};
server.listen(0, '127.0.0.1', async () => {
  try {
    process.stdout.write(JSON.stringify({ before: await once(false), after: await once(true) }));
  } catch (err) {
    process.stderr.write(String(err && err.stack));
    process.exitCode = 1;
  }
  server.close();
});
"#;

#[test]
#[ignore = "starts a real Chrome"]
fn a_session_sign_in_survives_the_host_chrome_restarting() {
    let dir = std::env::temp_dir().join(format!("equipe-session-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("check.cjs");
    std::fs::File::create(&script).unwrap().write_all(SCRIPT.replace("__KEEP_SESSIONS__", equipe_core::browser::KEEP_SESSIONS).as_bytes()).unwrap();
    let out = std::process::Command::new("npx").args(["-y", "-p", equipe_core::browser::PLAYWRIGHT_PACKAGE, "node"]).arg(&script).output();
    let _ = std::fs::remove_dir_all(&dir);
    let Ok(out) = out else {
        eprintln!("no npx here: nothing to test");
        return;
    };
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    eprintln!("{said:#}");
    // As Chrome is: a clean exit drops the session sign-in, keeps the lasting one.
    let before = said["before"]["cookies"].as_str().unwrap();
    assert!(before.contains("lasting=1") && !before.contains("session=1"), "{said}");
    // With the host's step: both come back, and no old tab with them.
    assert_eq!(said["after"]["marked"], "marked");
    let after = said["after"]["cookies"].as_str().unwrap();
    assert!(after.contains("lasting=1") && after.contains("session=1"), "{said}");
    let tabs = said["after"]["tabs"].as_array().unwrap();
    assert!(tabs.iter().all(|t| !t.as_str().unwrap_or_default().ends_with("/sign-in")), "the old tab isn't reopened: {said}");
}
