// Each profile's Chrome, started when a lane first asks for it — a pure step,
// written into the browser host (`browser-host.cjs`) and tested on its own
// (`browser::tests`).
//
// The person's own window for a sign-in (`window_handoff.rs`) hands a
// profile's folder to a plain Chrome; when they are done, that Chrome is
// closed and the automated one starts again on the folder. A plain Chrome
// still on its way out holds the folder for a few seconds more, and
// Playwright then refuses it ("the profile is already in use"). The host
// used to keep that refusal: every later lane of the profile waited on a
// Chrome that never came, no member's browser answered on its port, and
// Codex — which gives up on a server that doesn't come up — started members
// with no browser tools at all until the host was restarted.
//
// So: a start refused because the folder is still another Chrome's is tried
// again for a while, and a start that fails is forgotten, so the next lane
// that asks starts the profile's Chrome again.

/// Whether Chrome refused a profile because another Chrome has its folder open.
function profileBusy(err) {
  return /already in use|existing browser session|ProcessSingleton|SingletonLock/i.test(String((err && err.message) || err));
}

/// `start()` again while the profile's folder is still another Chrome's, up
/// to `tries` times, `wait` ms apart; any other failure at once.
async function startWhenFree(start, tries, wait) {
  for (let attempt = 1; ; attempt++) {
    try {
      return await start();
    } catch (err) {
      if (!profileBusy(err) || attempt >= tries) throw err;
      await new Promise((resolve) => setTimeout(resolve, wait));
    }
  }
}

/// Profile `name`'s Chrome in `profiles` (name → promise), started with
/// `start` when there is none. One that failed to start is forgotten.
function startOnce(profiles, name, start) {
  if (!profiles.has(name)) {
    const started = start();
    profiles.set(name, started);
    started.catch(() => {
      if (profiles.get(name) === started) profiles.delete(name);
    });
  }
  return profiles.get(name);
}
