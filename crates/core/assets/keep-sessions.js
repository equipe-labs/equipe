// Sign-ins that last only "for the session" kept across the browser host's
// restarts — a pure step, written into the browser host (`browser-host.cjs`)
// and tested on its own (`browser::tests`; a real Chrome in
// `tests/session_cookies_live.rs`).
//
// Chrome drops a profile's session cookies (cookies with no expiry date —
// many sites keep their sign-in in one) when it starts after a clean exit.
// It keeps them only after a crash, or with "Continue where you left off"
// (`--restore-last-session`), which also reopens every old tab and window.
// The host's Chrome exits cleanly each time the daemon stops — an update, a
// restart, the host started again — so each of those sign-ins was lost and
// a member had to ask the person again.
//
// Before Chrome starts, its last exit is marked the way Chrome marks a crash
// (`profile.exit_type` in the profile's `Preferences`): Chrome then restores
// the old session's cookies and nothing else — no tab or window comes back,
// and `--hide-crash-restore-bubble` keeps its "Restore pages?" bubble away.
// Only that one field changes. A profile Chrome never ran, or a file that
// isn't Chrome's JSON, is left as it is.
//
// `profile`: the user-data-dir. Answers what it did: `marked`, `kept` (marked
// already), `none` (no Preferences yet), `unreadable`, `unwritable`.
function keepSessions(profile, fs, path) {
  const file = path.join(profile, 'Default', 'Preferences');
  let text;
  try { text = fs.readFileSync(file, 'utf8'); } catch { return 'none'; }
  let prefs;
  try { prefs = JSON.parse(text); } catch { return 'unreadable'; }
  if (!prefs || typeof prefs !== 'object' || Array.isArray(prefs)) return 'unreadable';
  if (!prefs.profile || typeof prefs.profile !== 'object') prefs.profile = {};
  if (prefs.profile.exit_type === 'Crashed') return 'kept';
  prefs.profile.exit_type = 'Crashed';
  const tmp = `${file}.equipe-tmp`;
  try {
    fs.writeFileSync(tmp, JSON.stringify(prefs), { mode: 0o600 });
    fs.renameSync(tmp, file);
  } catch {
    try { fs.unlinkSync(tmp); } catch {}
    return 'unwritable';
  }
  return 'marked';
}
