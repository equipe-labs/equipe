// equipe's browser host: one Google Chrome, one profile, for every member of
// every squad. The person signs in once — Google too, which ends a sign-in it
// sees copied to another browser — and every member works signed in.
//
// Each member gets a lane: an MCP endpoint of Playwright's own browser tools
// that sees only the tabs that member opened (and the popups they opened), so
// members work side by side without touching each other's pages. The person's
// own tabs belong to no lane ("" below).
//
// Where pages live. Chrome stops drawing the tab in front of a minimized
// window — no screenshot, no screencast — once it navigates to another site;
// a tab *behind* another one keeps drawing for DevTools. So members' tabs open
// behind an equipe placeholder, in one minimized "parking" window: no window
// comes up, nothing takes the person's keyboard, and every tab can be watched
// and pictured. Putting a member's browser on screen brings the window back
// with that member's tab in front; out of sight, the placeholder goes back in
// front and the window down again.
//
// Run by the daemon under `npx -p playwright` (its package is found on PATH):
// `node browser-host.cjs <profile> <sign-ins file>`, then one JSON request per
// line on stdin:
//   {"op":"lane","id":"squad/name","member":"name","color":"r, g, b","port":1234,"output":"/dir","shown":false}
//   {"op":"show","id":"squad/name","shown":true}
//   {"op":"close","id":"squad/name"}
//   {"op":"person","url":"https://…"}
//   {"op":"watch","page":3,"maxW":1200,"maxH":900,"quality":70,"fps":30}  {"op":"unwatch","page":3}
//   {"op":"input","page":3,"ev":{…}}   {"op":"nav","page":3,"action":"back|forward|reload|stop|go","url":"…"}
//   {"op":"fit","page":3,"w":600,"h":800,"dpr":2}  (lay a page a panel watches out at its picture — the largest panel's)
//   {"op":"unfit","page":3}  (no panel watches it: its own size again)
//   {"op":"newtab","url":"…"}  {"op":"closetab","page":3}
//   {"op":"control","lane":"squad/name","by":"person"|null,"why":"check"?,"ifUntouched":true?}
//   {"op":"goto","lane":"squad/name","url":"…"}  {"op":"dialog","page":3,"accept":true,"text":"…"}
//   {"op":"popout","page":3,"bounds":{"left":0,"top":0,"width":900,"height":800}}
// Page ids start at $EQUIPE_FIRST_PAGE. A member's tab is told with what the
// daemon reads to tell whether it waits for a person (`signals`), and its icon.
// Its standard output is binary: messages of `u32be length | u8 kind | payload`
// — kind 1 a JSON object (`tabs`, `control`, `dialog`, …), kind 2 a frame of a
// watched page: `u16be header length | header JSON | JPEG`. Anything else that
// would write there (a library's console.log) goes to standard error.
const path = require('node:path');
const fs = require('node:fs');
const readline = require('node:readline');
const { EventEmitter } = require('node:events');
const { execFile, execFileSync } = require('node:child_process');

// The binary channel is ours alone.
const stdout = process.stdout;
const writeOut = stdout.write.bind(stdout);
process.stdout.write = (chunk, encoding, callback) => process.stderr.write(chunk, encoding, callback);

const roots = (process.env.PATH || '').split(path.delimiter).filter((p) => p.endsWith(path.join('node_modules', '.bin'))).map((p) => path.dirname(p));
const from = (id) => require.resolve(id, { paths: roots });
const { chromium } = require(from('playwright'));
const { tools } = require(path.join(path.dirname(from('playwright-core/package.json')), 'lib', 'coreBundle.js'));

const [profile, STATE] = process.argv.slice(2);

__DRIVING__

__GATE__

// Google's sign-in is this browser's own; the sign-ins file never carries it.
const own = (domain) => /(^|\.)(google\.[a-z]{2,3}(\.[a-z]{2})?|youtube\.com|googleusercontent\.com)$/.test(String(domain).replace(/^\./, ''));

const PERSON = '';
const PLACEHOLDER = 'data:text/html;charset=utf-8,' + encodeURIComponent(
  '<!doctype html><title>equipe</title><meta name="color-scheme" content="light dark">'
  + '<body style="font: 14px -apple-system, system-ui; display: grid; place-items: center; height: 90vh; color: #888">'
  + 'equipe — members’ tabs are behind this one</body>');
// A person who stops touching a member's page hands it back after this long.
const IDLE_CONTROL_MS = 120000;

/// One message on the binary channel; `false` when the pipe is full.
const send = (kind, payload) => {
  const head = Buffer.alloc(5);
  head.writeUInt32BE(payload.length + 1, 0);
  head.writeUInt8(kind, 4);
  return writeOut(Buffer.concat([head, payload]));
};
const emit = (message) => send(1, Buffer.from(JSON.stringify(message)));

/// A JPEG's size, from its frame header.
const jpegSize = (buf) => {
  let i = 2;
  while (i + 9 < buf.length) {
    if (buf[i] !== 0xff) { i++; continue; }
    const marker = buf[i + 1];
    if (marker === 0xff || (marker >= 0xd0 && marker <= 0xd9) || marker === 0x01) { i += marker === 0xff ? 1 : 2; continue; }
    const len = buf.readUInt16BE(i + 2);
    if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc) {
      return { h: buf.readUInt16BE(i + 5), w: buf.readUInt16BE(i + 7) };
    }
    i += 2 + len;
  }
  return { w: 0, h: 0 };
};

/// The app in front on this Mac, to give it back the keyboard after Chrome took it.
const frontApp = () => {
  if (process.platform !== 'darwin') return null;
  try {
    const asn = execFileSync('lsappinfo', ['front'], { timeout: 1000 }).toString().trim();
    const info = execFileSync('lsappinfo', ['info', '-only', 'bundleid', asn], { timeout: 1000 }).toString();
    return /"CFBundleIdentifier"="([^"]+)"/.exec(info)?.[1] || null;
  } catch { return null; }
};
const CHROME = 'com.google.Chrome';
const giveBack = (bundle) => {
  if (!bundle || bundle === CHROME) return;
  setTimeout(() => { if (frontApp() === CHROME) execFile('open', ['-g', '-b', bundle], () => {}); }, 150);
};

/// What the person typed in the address field: an address, or a search.
const address = (text) => {
  const t = String(text || '').trim();
  if (!t) return 'about:blank';
  if (/^[a-z][a-z0-9+.-]*:/i.test(t) && !/^localhost:\d/i.test(t)) return t;
  if (!/\s/.test(t) && (/^localhost(:\d+)?(\/|$)/i.test(t) || /^[^/]+\.[a-z]{2,}(:\d+)?(\/.*)?$/i.test(t) || /^\d+\.\d+\.\d+\.\d+(:\d+)?(\/|$)/.test(t))) {
    return (/^(localhost|127\.|\d+\.\d+\.\d+\.\d+)/i.test(t) ? 'http://' : 'https://') + t;
  }
  return `https://www.google.com/search?q=${encodeURIComponent(t)}`;
};

(async () => {
  const context = await chromium.launchPersistentContext(profile, {
    channel: 'chrome',
    headless: false,
    viewport: null,
    // An ordinary browser, as far as sites can tell: Google refuses to sign
    // anyone in to one that says it is automated. --test-type keeps Chrome from
    // warning about the flag that hides it, in a bar across every window.
    ignoreDefaultArgs: ['--enable-automation'],
    args: ['--disable-blink-features=AutomationControlled', '--test-type'],
  });

  const owner = new Map(); // page -> lane id ("" for the person's own)
  const lanes = new Map(); // lane id -> { events, member, color, shown, control, reason, touched, refused, lastInput, agent }
  const ids = new Map(); // page -> page id
  const pages = new Map(); // page id -> page
  const cdps = new Map(); // page -> CDP session, for watching and input
  const dialogs = new Map(); // page id -> the dialog waiting for an answer
  const watches = new Map(); // page id -> { cdp, quality, maxW, maxH, fps, seq, lastAck }
  const fits = new Map(); // page id -> { w, h, dpr }: the largest box a panel shows it in
  // Ids go on from where the daemon says (from the clock): a host started again
  // never names a tab with an id a screen still holds for another.
  let nextId = Math.max(1, Number(process.env.EQUIPE_FIRST_PAGE) || 1);
  let claim = null;
  let queue = Promise.resolve();
  let parking = null; // { page, cdp, windowId }
  let personShown = null; // a page the person asked for on screen
  let staged = null; // the page the person drives, in front of a real window (`restage`)
  let onStage = null; // the page `restack` last brought forward for that
  let unsized = null; // { windowId, width, height }: the window's size before the stage sized it

  const idOf = (page) => {
    if (!ids.has(page)) {
      const id = nextId++;
      ids.set(page, id);
      pages.set(id, page);
    }
    return ids.get(page);
  };
  const cdpOf = async (page) => {
    if (!cdps.has(page)) cdps.set(page, context.newCDPSession(page));
    return cdps.get(page);
  };
  const isPlaceholder = (page) => parking && page === parking.page;

  // ── which tabs each lane has, as the daemon is told ──

  const pending = new Map();
  const tabsOf = (lane) => context.pages().filter((p) => owner.get(p) === lane && !p.isClosed());
  // What the daemon reads off a member's page to tell whether it waits for a
  // person (`human_check` in browser.rs): a CAPTCHA box in sight and whether
  // it was answered, a password or one-time-code field, the start of the
  // text — and the page's icon. Run in the page; reads, never touches.
  const PROBE = () => {
    const seen = (e) => {
      const r = e.getBoundingClientRect();
      if (r.width <= 0 || r.height <= 0) return false;
      const s = getComputedStyle(e);
      return s.visibility !== 'hidden' && s.display !== 'none' && Number(s.opacity) > 0.05;
    };
    const big = (e) => { const r = e.getBoundingClientRect(); return r.width >= 60 && r.height >= 40; };
    const all = (sel) => Array.from(document.querySelectorAll(sel));
    const captcha = /challenges\.cloudflare\.com|hcaptcha\.com|\/recaptcha\/(api2|enterprise)\/|recaptcha\.net\/recaptcha/;
    const widget = all('iframe').some((f) => captcha.test(f.src) && !/size=invisible/.test(f.src) && seen(f) && big(f))
      || all('.cf-turnstile, .g-recaptcha, .h-captcha, #challenge-form, #cf-challenge-running, #turnstile-wrapper')
        .some((e) => e.getAttribute('data-size') !== 'invisible' && seen(e) && big(e));
    const solved = all('[name="cf-turnstile-response"], [name="g-recaptcha-response"], [name="h-captcha-response"]').some((e) => !!e.value);
    const inputs = all('input').filter((i) => !i.disabled && seen(i));
    const password = inputs.some((i) => i.type === 'password');
    const otp = inputs.some((i) => i.autocomplete === 'one-time-code' || /(^|[^a-z])(otp|totp|mfa|2fa)([^a-z]|$)/i.test(`${i.name} ${i.id}`));
    const code = inputs.some((i) => ['text', 'tel', 'number'].includes(i.type) && ((i.maxLength >= 4 && i.maxLength <= 8) || i.inputMode === 'numeric'));
    const text = ((document.body && document.body.innerText) || '').slice(0, 600).toLowerCase();
    const link = document.querySelector('link[rel~="icon"], link[rel="shortcut icon"], link[rel="apple-touch-icon"]');
    return { widget, solved, password, otp, code, text, icon: (link && link.href) || null };
  };
  const probe = (page) => Promise.race([
    page.evaluate(PROBE).catch(() => null),
    new Promise((resolve) => setTimeout(() => resolve(null), 600)),
  ]);
  const describe = async (page) => {
    const url = page.url();
    const probed = /^https?:/.test(url) ? await probe(page) : null;
    const origin = (() => { try { return new URL(url).origin; } catch { return null; } })();
    const favicon = probed?.icon || (origin && /^https?:/.test(origin) ? `${origin}/favicon.ico` : null);
    const out = { id: idOf(page), url, title: await page.title().catch(() => ''), loading: !!page.__equipeLoading, favicon, crashed: !!page.__equipeCrashed };
    if (owner.get(page) !== PERSON && probed) {
      const frames = page.frames().slice(1).map((f) => f.url()).filter((u) => u && u !== 'about:blank').slice(0, 24);
      const { icon, ...signals } = probed;
      out.signals = { ...signals, frames };
      page.__equipeSig = signature(signals, frames);
    }
    return out;
  };
  // What, changing in place, may change whether a page waits for a person.
  const signature = (s, frames) => [s.widget, s.solved, s.password, s.otp, s.code, frames.filter((f) => /captcha|challenge/.test(f)).join(' ')].join('|');
  // A title a page changes in place (a count of unread mail, a dialog's
  // answer) is told too: looked at every moment or so — and a member's page
  // that starts or stops waiting for a person (a CAPTCHA answered), every two.
  const titles = new Map();
  let tick = 0;
  setInterval(async () => {
    tick++;
    for (const page of context.pages()) {
      if (!owner.has(page)) continue;
      const title = await page.title().catch(() => null);
      if (title !== null && titles.get(page) !== title) {
        if (titles.has(page)) report(owner.get(page));
        titles.set(page, title);
      }
      if (tick % 2 === 0 && owner.get(page) !== PERSON && /^https?:/.test(page.url())) {
        const s = await probe(page);
        if (!s) continue;
        const frames = page.frames().slice(1).map((f) => f.url()).filter((u) => u && u !== 'about:blank');
        if (signature(s, frames) !== page.__equipeSig) report(owner.get(page));
      }
    }
  }, 1000);
  const report = (lane) => {
    if (pending.has(lane)) return;
    pending.set(lane, setTimeout(async () => {
      pending.delete(lane);
      const info = lanes.get(lane);
      const list = await Promise.all(tabsOf(lane).map(describe));
      emit({ type: 'tabs', lane, member: info?.member ?? null, color: info?.color ?? null, shown: !!info?.shown, pages: list, agent: info?.agent ?? null });
    }, 80));
  };

  // Who a page works for, as the overlay asks it — quiet unless its window is
  // on the person's screen: in the panel, the panel says who drives it.
  const onScreen = (page) => {
    const lane = lanes.get(owner.get(page));
    return !!lane && (lane.shown || personShown === page) && lane.control !== 'person';
  };
  await context.exposeBinding('__equipeWho', ({ page }) => {
    const lane = lanes.get(owner.get(page));
    return lane ? { member: lane.member, color: lane.color, quiet: !onScreen(page) } : null;
  });
  const hush = (laneId) => {
    for (const page of tabsOf(laneId)) {
      const quiet = !onScreen(page);
      page.evaluate((q) => window.__equipeQuiet && window.__equipeQuiet(q), quiet).catch(() => {});
    }
  };
  await context.addInitScript(`(async () => {
    if (window.top !== window || !window.__equipeWho) return;
    const who = await window.__equipeWho().catch(() => null);
    if (who) (${driving.toString()})(who);
  })()`);

  // ── the parking window ──

  const windowOf = async (page) => (await cdpOf(page)).send('Browser.getWindowForTarget');
  const setState = async (windowId, windowState) => {
    if (!parking) return;
    await parking.cdp.send('Browser.setWindowBounds', { windowId, bounds: { windowState } }).catch(() => {});
  };
  // The parking window, made again if the person closed it. A window made now
  // is brought forward once, so Chrome opens the next tabs in it.
  const ensureParking = async () => {
    if (parking && !parking.page.isClosed()) return parking;
    const before = frontApp();
    let page = context.pages().find((p) => !owner.has(p));
    if (!page || page.url() !== 'about:blank') {
      const cdp = await context.newCDPSession(context.pages()[0] || (await context.newPage()));
      const created = new Promise((resolve) => context.once('page', resolve));
      claim = 'parking';
      await cdp.send('Target.createTarget', { url: 'about:blank', newWindow: true, background: true });
      page = await created;
      await cdp.detach().catch(() => {});
    }
    owner.delete(page);
    await page.goto(PLACEHOLDER).catch(() => {});
    const cdp = await context.newCDPSession(page);
    const { windowId } = await cdp.send('Browser.getWindowForTarget');
    parking = { page, cdp, windowId };
    await page.bringToFront().catch(() => {});
    await setState(windowId, 'minimized');
    giveBack(before);
    page.once('close', () => { if (parking?.page === page) parking = null; });
    return parking;
  };

  /// What should be in front of the parking window now: the tab of a member
  /// whose browser is on screen, the person's own tab, or nothing (it goes down).
  // One at a time: a hand taken and given back at once would otherwise leave
  // the window down with the stage's tab in front, or up with nothing.
  let stacking = Promise.resolve();
  const restack = () => (stacking = stacking.then(restackNow, restackNow));
  const restackNow = async () => {
    const park = await ensureParking();
    const shown = [...lanes.entries()].filter(([, l]) => l.shown).map(([id]) => id);
    const front = (personShown && !personShown.isClosed() ? personShown : null)
      || shown.map((id) => pages.get(lanes.get(id).agent) || tabsOf(id).at(-1)).find((p) => p && !p.isClosed());
    if (front) {
      onStage = null;
      await unsize();
      const { windowId } = await windowOf(front).catch(() => ({}));
      if (windowId) await setState(windowId, 'normal');
      await raise(front);
      return;
    }
    if (staged && !staged.isClosed()) return putOnStage(staged);
    onStage = null;
    await unsize();
    // Nothing to show: the placeholder in front, the window down — and the
    // keyboard back where it was. Already down, it stays as it is: bringing a
    // tab forward would bring the window up.
    const { bounds } = await park.cdp.send('Browser.getWindowBounds', { windowId: park.windowId }).catch(() => ({ bounds: {} }));
    if (bounds.windowState === 'minimized') return;
    const before = frontApp();
    await park.cdp.send('Target.activateTarget', { targetId: (await park.cdp.send('Target.getTargetInfo')).targetInfo.targetId }).catch(() => {});
    await setState(park.windowId, 'minimized');
    giveBack(before);
  };
  const raise = async (page) => {
    const original = page.bringToFront.original || page.bringToFront;
    await original.call(page).catch(() => {});
  };

  // ── the stage: a page the person drives, where a page is ──
  //
  // A tab behind the placeholder is drawn for DevTools, and runs as a shown
  // page does (visible, drawing, its timers at pace) — but it sits on no
  // screen: its window.screenY is past the foot of the screen and it has no
  // toolbar (outerHeight = innerHeight), so every click the person makes
  // through the panel lands, as far as the page can tell, below the screen.
  // A "verify you are human" check reads that as no person (see
  // docs/BROWSER-CHECKS.md). So while the person drives a page — a member's in
  // their hands, or their own once they put a hand on it — and the panel shows
  // it, it comes to the front of a normal window sized to the panel's box,
  // behind the app the person is in; given back, it is parked again.
  const stageable = (page) => {
    if (!page || page.isClosed() || !watches.has(idOf(page))) return false;
    const id = owner.get(page);
    return id === PERSON ? !!page.__equipeHand : lanes.get(id)?.control === 'person';
  };
  const restage = async () => {
    const want = stageable(staged) ? staged : [...watches.keys()].map((pid) => pages.get(pid)).find(stageable) || null;
    if (want === staged) return;
    const was = staged;
    staged = want;
    // Laid out by its window on the stage, by the panel's box off it.
    if (was && !was.isClosed()) await applyFit(was);
    if (want) await applyFit(want);
    await restack();
  };
  const putOnStage = async (page) => {
    const { windowId } = await windowOf(page).catch(() => ({}));
    if (!windowId) return;
    const { bounds } = await parking.cdp.send('Browser.getWindowBounds', { windowId }).catch(() => ({ bounds: {} }));
    if (onStage === page && bounds.windowState === 'normal') return sizeStage(page, windowId);
    // Brought up without taking the person's keyboard: whatever app they are
    // in gets it back, the window behind it.
    const before = frontApp();
    if (bounds.windowState !== 'normal') await setState(windowId, 'normal');
    await raise(page);
    onStage = page;
    await sizeStage(page, windowId);
    giveBack(before);
  };
  /// Off the stage, the window the stage sized at its own size again (resized
  /// while it is still up: Chrome resizes no minimized window).
  const unsize = async () => {
    if (!unsized || !parking) return;
    const { windowId, width, height } = unsized;
    unsized = null;
    const { bounds } = await parking.cdp.send('Browser.getWindowBounds', { windowId }).catch(() => ({}));
    if (!bounds) return;
    if (bounds.windowState !== 'normal') await setState(windowId, 'normal');
    await parking.cdp.send('Browser.setWindowBounds', { windowId, bounds: { width, height } }).catch(() => {});
  };
  /// The stage's window, sized so the page's viewport is the panel's box.
  const sizeStage = async (page, windowId) => {
    const f = fits.get(idOf(page));
    if (!f || !windowId) return;
    // Its toolbar's height, once it is in front (a tab behind has none).
    let edge = null;
    for (let i = 0; i < 10; i++) {
      edge = await page.evaluate(() => [outerWidth - innerWidth, outerHeight - innerHeight, innerWidth, innerHeight]).catch(() => null);
      if (!edge || edge[1] > 0) break;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    if (!edge || edge[1] <= 0 || (edge[2] === f.w && edge[3] === f.h)) return;
    // The window is every parked tab's: its own size comes back off the stage.
    if (unsized?.windowId !== windowId) {
      await unsize();
      const { bounds } = await parking?.cdp.send('Browser.getWindowBounds', { windowId }).catch(() => ({})) || {};
      if (bounds?.width && bounds?.height) unsized = { windowId, width: bounds.width, height: bounds.height };
    }
    await parking?.cdp.send('Browser.setWindowBounds', { windowId, bounds: { width: f.w + edge[0], height: f.h + edge[1] } }).catch(() => {});
    // A page that doesn't change draws no new frame: its screencast starts
    // again, which sends one at the new size at once.
    const w = watches.get(idOf(page));
    if (w) {
      await w.cdp.send('Page.stopScreencast').catch(() => {});
      await w.cdp.send('Page.startScreencast', cast(w)).catch(() => {});
    }
  };

  // ── pages ──

  context.on('page', async (page) => {
    let id = claim;
    claim = null;
    if (id === 'parking') return;
    if (id === null) {
      const opener = await page.opener().catch(() => null);
      id = opener ? owner.get(opener) : undefined;
      if (id === undefined && opener === null && parking) id = PERSON; // a tab the person opened in Chrome
    }
    if (id === undefined || (id !== PERSON && !lanes.has(id))) return;
    owner.set(page, id);
    idOf(page);
    // A member's tab never comes forward on its own while its browser is out of
    // sight: Chrome would bring the window up, and that tab would stop drawing.
    const bringToFront = page.bringToFront.bind(page);
    page.bringToFront = async () => {
      const lane = lanes.get(owner.get(page));
      if (!lane || lane.shown) return bringToFront();
    };
    page.bringToFront.original = bringToFront;
    page.on('framenavigated', (frame) => {
      // A frame coming or going may be a CAPTCHA's: told too (in a moment, with the rest).
      if (frame !== page.mainFrame()) return report(id);
      page.__equipeLoading = true;
      report(id);
    });
    page.on('framedetached', () => report(id));
    page.on('load', () => { page.__equipeLoading = false; page.__equipeCrashed = false; report(id); });
    // "Aw, Snap!": the screen says so, and offers to reload it.
    page.on('crash', () => { page.__equipeCrashed = true; page.__equipeLoading = false; report(id); });
    page.on('domcontentloaded', () => report(id));
    page.on('dialog', (dialog) => {
      const pid = idOf(page);
      dialogs.set(pid, dialog);
      emit({ type: 'dialog', page: pid, lane: id, kind: dialog.type(), message: dialog.message(), default: dialog.defaultValue() || null });
    });
    page.once('close', () => {
      const pid = idOf(page);
      owner.delete(page);
      ids.delete(page);
      pages.delete(pid);
      stopWatch(pid);
      fits.delete(pid);
      cdps.get(page)?.then((c) => c.detach()).catch(() => {});
      cdps.delete(page);
      if (dialogs.delete(pid)) emit({ type: 'dialog_closed', page: pid });
      const lane = lanes.get(id);
      if (lane && lane.agent === pid) lane.agent = null;
      if (personShown === page) personShown = null;
      if (onStage === page) onStage = null;
      if (staged === page) {
        staged = null;
        restage().then(restack).catch(() => {});
      }
      titles.delete(page);
      report(id);
    });
    if (id !== PERSON) lanes.get(id).events.emit('page', page);
    report(id);
    // A popup that came up in front of a member's hidden tabs goes behind again.
    const lane = lanes.get(id);
    if (lane && !lane.shown && parking) {
      const { windowId } = await windowOf(page).catch(() => ({}));
      if (windowId === parking.windowId) {
        const { windowState } = (await parking.cdp.send('Browser.getWindowBounds', { windowId }).catch(() => ({ bounds: {} }))).bounds || {};
        if (windowState !== 'minimized') await restack();
      }
    }
  });

  /// A new tab for `lane`, behind the placeholder (or in front, if the lane is on screen).
  const openTab = (lane, url = 'about:blank') => (queue = queue.then(async () => {
    const park = await ensureParking();
    const created = new Promise((resolve) => {
      const check = (p) => {
        if (owner.get(p) === lane) {
          context.off('page', check);
          setImmediate(() => resolve(p));
        }
      };
      context.on('page', check);
    });
    claim = lane;
    await park.cdp.send('Target.createTarget', { url, newWindow: false, background: true });
    const page = await created;
    if (lane !== PERSON && lanes.get(lane)?.shown) await restack().catch(() => {});
    return page;
  }));

  const laneContext = (id) => {
    const lane = lanes.get(id);
    return new Proxy(context, {
      get(target, prop, receiver) {
        if (prop === 'pages') return () => tabsOf(id);
        if (prop === 'newPage') return () => openTab(id);
        if (['on', 'addListener', 'once', 'off', 'removeListener'].includes(prop)) {
          return (event, handler) => {
            (event === 'page' ? lane.events : target)[prop](event, handler);
            return receiver;
          };
        }
        const value = Reflect.get(target, prop, target);
        return typeof value === 'function' ? value.bind(target) : value;
      },
    });
  };

  // The tabs a member's tool call acts on, by page id: a tab closed by its
  // index, every tab for closing the browser, none for listing or opening one,
  // else the tab it is on.
  const targetsOf = (laneId, backend, name, args) => {
    let current = null;
    try {
      const page = backend._context?.currentTab?.()?.page;
      if (page && !page.isClosed()) current = idOf(page);
    } catch {}
    const tabs = tabsOf(laneId).map(idOf);
    // A turn's first call has no tab of its own yet: Playwright starts it on the first.
    current = current ?? tabs[0] ?? null;
    if (name === 'browser_close') return tabs;
    if (name === 'browser_tabs') {
      if (args?.action !== 'close') return [];
      return typeof args.index === 'number' ? [tabs[args.index]].filter((p) => p !== undefined) : [current].filter((p) => p !== null);
    }
    return current !== null ? [current] : [];
  };

  const openLane = async ({ id, member, color, port, output, shown }) => {
    if (lanes.has(id)) return show({ id, shown });
    // control: 'person' while the person has it — `reason` 'check' when equipe put
    // it in their hands for a page that waits for a person, 'hand' when they took
    // it; `touched`, they put a hand on it since; `refused`, its tools were
    // turned away meanwhile; `page`, the tab the hold is about (the waiting page,
    // or the one the person is on) and `since`, when it began (`gate.js`).
    const lane = { events: new EventEmitter(), member, color, shown: !!shown, control: null, reason: null, touched: false, refused: false, lastInput: 0, agent: null, busy: 0, page: null, since: 0 };
    lanes.set(id, lane);
    const config = await tools.resolveCLIConfigForMCP({ outputDir: output }, process.env);
    const toolset = tools.filteredTools(config);
    const ctx = laneContext(id);
    await tools.start({
      name: 'api',
      nameInConfig: 'browser',
      version: '0',
      toolSchemas: toolset.map((t) => t.schema),
      create: async () => {
        const backend = new tools.BrowserBackend(config, ctx, toolset);
        // While the person uses a tab of this member's browser its tools wait
        // for them there (`gate.js`: a hold from detection alone never stops it
        // leaving the page, and its other tabs stay its own); after each call,
        // the tab it worked in is the one the screen follows.
        const call = backend.callTool.bind(backend);
        backend.callTool = async (name, args, signal) => {
          const no = gate(lane, { tool: name, args, targets: targetsOf(id, backend, name, args) });
          if (no) {
            lane.refused = true;
            if (no.wants && no.page) emit({ type: 'wants', lane: id, page: no.page, action: no.wants });
            return { content: [{ type: 'text', text: `### Error\n${no.text}` }], isError: true };
          }
          // A step under way is never resized under it: a panel's new size
          // waits for the step to finish (`applyFit`).
          lane.busy++;
          let result;
          try {
            result = await call(name, args, signal);
          } finally {
            lane.busy--;
            if (!lane.busy) for (const page of tabsOf(id)) if (page.__equipeFitWaits) applyFit(page).catch(() => {});
          }
          const page = backend._context?.currentTab?.()?.page;
          if (page && owner.get(page) === id) {
            const pid = idOf(page);
            if (lane.agent !== pid) {
              lane.agent = pid;
              report(id);
            }
          }
          return result;
        };
        return backend;
      },
    }, { port, host: 'localhost' });
    report(id);
    if (lane.shown) await restack();
  };
  const show = async ({ id, shown }) => {
    const lane = lanes.get(id);
    if (!lane || lane.shown === !!shown) return;
    lane.shown = !!shown;
    report(id);
    hush(id);
    await restack();
    // On this Mac's screen the window's size rules; out of it, a panel's again.
    for (const page of tabsOf(id)) await applyFit(page);
  };
  const close = async ({ id }) => {
    for (const page of tabsOf(id)) await page.close().catch(() => {});
  };
  const person = async ({ url }) => {
    const page = await openTab(PERSON, url);
    personShown = page;
    await applyFit(page);
    await restack();
  };

  // ── watching a page: a screencast, paced by when frames are taken ──

  const stopWatch = (pid) => {
    const w = watches.get(pid);
    if (!w) return;
    watches.delete(pid);
    clearTimeout(w.restTimer);
    w.cdp.send('Page.stopScreencast').catch(() => {});
    w.cdp.off('Page.screencastFrame', w.onFrame);
  };
  const watch = async ({ page: pid, maxW = 1280, maxH = 1280, quality = 70, fps = 30 }) => {
    const page = pages.get(pid);
    if (!page) return;
    const old = watches.get(pid);
    if (old && old.quality === quality && old.maxW === maxW && old.maxH === maxH) {
      old.fps = fps;
      return;
    }
    stopWatch(pid);
    const cdp = await cdpOf(page);
    const w = { cdp, quality, maxW, maxH, fps, seq: old?.seq || 0, lastAck: 0, onFrame: null };
    w.onFrame = (frame) => {
      if (watches.get(pid) !== w) return;
      const jpeg = Buffer.from(frame.data, 'base64');
      const { w: width, h: height } = jpegSize(jpeg);
      const m = frame.metadata;
      const header = Buffer.from(JSON.stringify({
        page: pid, seq: ++w.seq, w: width, h: height, cssW: m.deviceWidth, cssH: m.deviceHeight,
        sx: m.scrollOffsetX, sy: m.scrollOffsetY, top: m.offsetTop, ts: m.timestamp,
      }));
      const hlen = Buffer.alloc(2);
      hlen.writeUInt16BE(header.length, 0);
      const flowing = send(2, Buffer.concat([hlen, header, jpeg]));
      // The next frame only once this one is out and its time has come.
      const ack = () => {
        const wait = Math.max(0, 1000 / Math.max(1, w.fps) - (Date.now() - w.lastAck));
        setTimeout(() => {
          w.lastAck = Date.now();
          cdp.send('Page.screencastFrameAck', { sessionId: frame.sessionId }).catch(() => {});
        }, wait);
      };
      if (flowing) ack();
      else stdout.once('drain', ack);
    };
    cdp.on('Page.screencastFrame', w.onFrame);
    watches.set(pid, w);
    await cdp.send('Page.startScreencast', cast(w)).catch(() => stopWatch(pid));
    await restage();
  };
  const unwatch = async ({ page }) => {
    stopWatch(page);
    await restage();
  };
  // While the wheel turns, pictures at half the size: a quarter of the bytes,
  // so more of them get through a slow line (docs/PERF-2026-10.md); the
  // usual ones again once it has rested a moment.
  const SCROLL_REST_MS = 300;
  const cast = (w) => {
    const k = w.light ? 0.5 : 1;
    return { format: 'jpeg', quality: w.quality, maxWidth: Math.round(w.maxW * k), maxHeight: Math.round(w.maxH * k), everyNthFrame: 1, maxFramesInFlight: 1 };
  };
  const recast = async (pid, w) => {
    if (watches.get(pid) !== w) return;
    await w.cdp.send('Page.stopScreencast').catch(() => {});
    await w.cdp.send('Page.startScreencast', cast(w)).catch(() => {});
  };
  const scrolling = (pid) => {
    const w = watches.get(pid);
    if (!w) return;
    clearTimeout(w.restTimer);
    w.restTimer = setTimeout(() => {
      w.light = false;
      pictures = pictures.then(() => recast(pid, w));
    }, SCROLL_REST_MS);
    if (!w.light) {
      w.light = true;
      pictures = pictures.then(() => recast(pid, w));
    }
  };

  // ── the size a page is laid out at ──

  // A page a panel watches — the person's own tab, a member's in their hands,
  // or a member's it drives itself — is laid out at the panel's picture (the
  // largest, with several screens watching), so it fills it, readable and
  // crisp at the screen's scale, rather than the member's wide window shrunk
  // into it. The member's tools work at whatever size the page has. Nobody
  // watching, its own size comes back; a page out in a real window, or a
  // member's window on this Mac's screen, keeps the window's. A member's step
  // under way is never resized under it: the size waits for the step's end.
  const inWindow = (page) => personShown === page || lanes.get(owner.get(page))?.shown === true;
  const applyFit = async (page) => {
    if (!page || page.isClosed()) return;
    const lane = lanes.get(owner.get(page));
    if (lane?.busy && lane.control !== 'person') {
      page.__equipeFitWaits = true;
      return;
    }
    page.__equipeFitWaits = false;
    const f = fits.get(idOf(page));
    const cdp = await cdpOf(page);
    if (f && !inWindow(page) && staged !== page) {
      const key = `${f.w}x${f.h}@${f.dpr}`;
      if (page.__equipeFit === key) return;
      page.__equipeFit = key;
      await cdp.send('Emulation.setDeviceMetricsOverride', { width: f.w, height: f.h, deviceScaleFactor: f.dpr, mobile: false }).catch(() => {});
    } else if (page.__equipeFit) {
      page.__equipeFit = null;
      await cdp.send('Emulation.clearDeviceMetricsOverride').catch(() => {});
    } else {
      return;
    }
    // A page that doesn't change draws no new frame: its screencast starts
    // again, which sends one at the new size at once.
    const w = watches.get(idOf(page));
    if (w) {
      await w.cdp.send('Page.stopScreencast').catch(() => {});
      await w.cdp.send('Page.startScreencast', cast(w)).catch(() => {});
    }
  };
  const fit = async ({ page: pid, w, h, dpr }) => {
    if (!(w >= 64 && h >= 64)) return;
    fits.set(pid, { w: Math.round(w), h: Math.round(h), dpr: Math.min(4, Math.max(1, Number(dpr) || 2)) });
    await applyFit(pages.get(pid));
    if (staged && pages.get(pid) === staged) await sizeStage(staged, (await windowOf(staged).catch(() => ({}))).windowId);
  };
  const unfit = async ({ page: pid }) => {
    if (!fits.delete(pid)) return;
    await applyFit(pages.get(pid));
  };

  // ── the person's hand on a page ──

  // `why` 'check': equipe puts it in the person's hands for a page waiting for
  // a person; anything else, the person took it.
  const setControl = async (laneId, by, why, page) => {
    const lane = lanes.get(laneId);
    if (!lane) return;
    if (by && why !== 'check') lane.touched = true;
    // What the hold is about: the page that waits (from the daemon), else the
    // one the person is on — its member's, as the panel shows it.
    if (by) {
      if (page) lane.page = page;
      else if (lane.control !== by || !lane.page) lane.page = lane.agent;
      if (lane.control !== by) lane.since = Date.now();
    } else {
      lane.page = null;
      lane.since = 0;
    }
    if (lane.control === by) return;
    lane.control = by;
    lane.lastInput = Date.now();
    const refused = lane.refused;
    if (by) {
      lane.reason = why === 'check' ? 'check' : 'hand';
      if (why === 'check') lane.touched = false;
    } else {
      lane.reason = null;
      lane.touched = false;
    }
    lane.refused = false;
    // The in-page "… is working in this browser" label says nothing while the
    // person drives: one state at a time.
    hush(laneId);
    for (const page of tabsOf(laneId)) {
      const cdp = await cdpOf(page);
      await cdp.send('Emulation.setFocusEmulationEnabled', { enabled: true }).catch(() => {});
      await applyFit(page);
    }
    emit({ type: 'control', lane: laneId, by, refused: !by && refused });
    await restage();
  };
  setInterval(() => {
    for (const [id, lane] of lanes) {
      // Waiting for a person who hasn't come yet: not handed back for that.
      if (lane.reason === 'check' && !lane.touched) continue;
      if (lane.control === 'person' && Date.now() - lane.lastInput > IDLE_CONTROL_MS) setControl(id, null);
    }
  }, 10000);

  const input = async ({ page: pid, ev }) => {
    const page = pages.get(pid);
    if (!page || !ev) return;
    const laneId = owner.get(page);
    const lane = lanes.get(laneId);
    // Only a press or a key takes a member's page into the person's hands.
    // Before that nothing of theirs reaches it — not the pointer passing over
    // the picture, nor a wheel — and moves never count as a hand on it (nor
    // keep it from going back when idle).
    const move = ev.t === 'mouse' && ev.type === 'mouseMoved';
    const takes = (ev.t === 'mouse' && ev.type === 'mousePressed') || ['key', 'ime', 'insert'].includes(ev.t);
    if (lane) {
      if (lane.control !== 'person') {
        if (!takes) return undefined;
        await setControl(laneId, 'person', undefined, pid);
        // The moves that brought the pointer here weren't sent: it moves to
        // where it presses first, as a mouse does.
        if (ev.t === 'mouse') {
          await (await cdpOf(page)).send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: ev.x, y: ev.y, button: 'none', buttons: 0, modifiers: ev.modifiers || 0, pointerType: 'mouse' }).catch(() => {});
        }
      }
      if (!move) {
        lane.touched = true;
        lane.lastInput = Date.now();
        // Their hand is on this tab: it is the one that is theirs.
        lane.page = pid;
      }
    } else if (laneId === PERSON && !move && !page.__equipeHand) {
      page.__equipeHand = true;
      await restage();
    }
    const cdp = await cdpOf(page);
    switch (ev.t) {
      case 'mouse':
        // As a mouse is: a pointer of type "mouse" whose pressure, while a
        // button is down, is the 0.5 the Pointer Events spec gives a device
        // that can't tell pressure (DevTools' default is 0, which no mouse
        // reports). The person's own press; members' clicks are Playwright's.
        return cdp.send('Input.dispatchMouseEvent', {
          type: ev.type, x: ev.x, y: ev.y, button: ev.button || 'none', buttons: ev.buttons || 0,
          clickCount: ev.clickCount || 0, modifiers: ev.modifiers || 0, pointerType: 'mouse',
          force: Number.isFinite(ev.force) ? ev.force : (ev.buttons ? 0.5 : 0),
        });
      case 'wheel':
        return cdp.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: ev.x, y: ev.y, deltaX: ev.dx || 0, deltaY: ev.dy || 0, modifiers: ev.modifiers || 0 });
      case 'key': {
        const params = { type: ev.type, modifiers: ev.modifiers || 0, key: ev.key, code: ev.code, windowsVirtualKeyCode: ev.vk || 0, autoRepeat: !!ev.repeat };
        if (ev.text !== undefined) { params.text = ev.text; params.unmodifiedText = ev.unmodifiedText ?? ev.text; }
        if (ev.commands?.length) params.commands = ev.commands;
        if (ev.location) params.location = ev.location;
        return cdp.send('Input.dispatchKeyEvent', params);
      }
      case 'ime':
        return cdp.send('Input.imeSetComposition', { text: ev.text || '', selectionStart: ev.start ?? (ev.text || '').length, selectionEnd: ev.end ?? (ev.text || '').length });
      case 'insert':
        return cdp.send('Input.insertText', { text: ev.text || '' });
      default:
        return undefined;
    }
  };

  const nav = async ({ page: pid, action, url }) => {
    const page = pages.get(pid);
    if (!page) return;
    const cdp = await cdpOf(page);
    if (action === 'reload') return cdp.send('Page.reload', {});
    if (action === 'stop') return cdp.send('Page.stopLoading');
    if (action === 'go') return cdp.send('Page.navigate', { url: address(url) });
    const { currentIndex, entries } = await cdp.send('Page.getNavigationHistory');
    const to = entries[currentIndex + (action === 'back' ? -1 : 1)];
    if (to) await cdp.send('Page.navigateToHistoryEntry', { entryId: to.id });
  };
  const goto = async ({ lane: laneId, url }) => {
    const lane = lanes.get(laneId);
    if (!lane) return;
    const page = pages.get(lane.agent) || tabsOf(laneId).at(-1) || (await openTab(laneId));
    await page.goto(address(url)).catch(() => {});
  };
  const newtab = async ({ url }) => { await openTab(PERSON, url ? address(url) : 'about:blank'); };
  const closetab = async ({ page }) => pages.get(page)?.close().catch(() => {});
  // `ifUntouched`: handed back only if the person never put a hand on it (a
  // check that passed by itself).
  const control = ({ lane, by, why, ifUntouched, page }) => {
    if (!by && ifUntouched && lanes.get(lane)?.touched) return undefined;
    return setControl(lane, by || null, why, page);
  };
  const dialog = async ({ page, accept, text }) => {
    const d = dialogs.get(page);
    if (!d) return;
    dialogs.delete(page);
    await (accept ? d.accept(text ?? undefined) : d.dismiss()).catch(() => {});
    emit({ type: 'dialog_closed', page });
  };
  // The real window, beside the app: for what a picture of the page can't do
  // (a passkey, a permission prompt, a <select>'s list).
  const popout = async ({ page: pid, bounds }) => {
    const page = pages.get(pid);
    if (!page) return;
    personShown = page;
    await applyFit(page);
    if (owner.get(page) !== PERSON) hush(owner.get(page));
    const { windowId } = await windowOf(page);
    await setState(windowId, 'normal');
    if (bounds) await parking?.cdp.send('Browser.setWindowBounds', { windowId, bounds }).catch(() => {});
    await raise(page);
  };

  // The sign-ins members' own browsers keep, and the old sign-in window wrote:
  // taken up here too — what is new to this browser, or changed in the file
  // since it last looked while its own copy stayed as it was.
  const key = (c) => `${c.name}\u0000${c.domain}\u0000${c.path}`;
  const stamp = (c) => `${c.value}\u0000${c.expires}`;
  const agreed = new Map();
  const read = new Map();
  for (const c of await context.cookies()) agreed.set(key(c), stamp(c));
  const take = async () => {
    let kept;
    try { kept = JSON.parse(fs.readFileSync(STATE, 'utf8')); } catch { return; }
    const now = Date.now() / 1000;
    const mine = new Map((await context.cookies()).map((c) => [key(c), c]));
    const fresh = (kept.cookies || []).filter((c) => {
      if (own(c.domain) || (c.expires !== -1 && c.expires <= now)) return false;
      const before = read.get(key(c));
      read.set(key(c), stamp(c));
      const have = mine.get(key(c));
      return !have || (before !== undefined && before !== stamp(c) && stamp(have) !== stamp(c) && agreed.get(key(c)) === stamp(have));
    });
    if (fresh.length) await context.addCookies(fresh).catch(() => {});
    for (const c of fresh) agreed.set(key(c), stamp(c));
  };
  await take();
  fs.watchFile(STATE, { interval: 2000 }, () => take().catch(() => {}));

  await ensureParking();

  const ops = { lane: openLane, show, close, person, watch, unwatch, fit, unfit, input, nav, goto, newtab, closetab, control, dialog, popout };
  // Input goes in order, each after the one before; so do watching and sizing
  // (two screencasts of a page started at once would leave one running
  // unseen); the rest as they come.
  let inputs = Promise.resolve();
  let pictures = Promise.resolve();
  // A wheel turn still waiting its go: the turns that come meanwhile add to it.
  // Chrome takes a frame (~17 ms) to answer each, a trackpad sends 60–120 a
  // second, so one by one they queued for seconds behind the picture
  // (docs/PERF-2026-10.md); added up, the page goes as far, at once.
  let wheel = null;
  readline.createInterface({ input: process.stdin }).on('line', (line) => {
    let request;
    try { request = JSON.parse(line); } catch { return; }
    const run = () => Promise.resolve(ops[request.op]?.(request)).catch((err) => console.error(`equipe browser host: ${request.op}: ${err && err.message}`));
    const ev = request.op === 'input' ? request.ev : null;
    if (ev?.t === 'wheel') scrolling(request.page);
    if (ev?.t === 'wheel' && wheel && wheel.page === request.page && (wheel.ev.modifiers || 0) === (ev.modifiers || 0)) {
      wheel.ev.dx = (wheel.ev.dx || 0) + (ev.dx || 0);
      wheel.ev.dy = (wheel.ev.dy || 0) + (ev.dy || 0);
      wheel.ev.x = ev.x;
      wheel.ev.y = ev.y;
      return;
    }
    if (request.op === 'input') wheel = ev?.t === 'wheel' ? request : null;
    if (request.op === 'input') inputs = inputs.then(() => { if (wheel === request) wheel = null; return run(); });
    else if (['watch', 'unwatch', 'fit', 'unfit'].includes(request.op)) pictures = pictures.then(run);
    else run();
  });
  // The daemon gone, the browser goes with it.
  process.stdin.on('end', () => context.close().finally(() => process.exit(0)));
  context.on('close', () => process.exit(0));
  emit({ type: 'ready' });
  console.error('equipe browser host: ready');
})().catch((err) => {
  console.error(`equipe browser host: ${err && err.message}`);
  process.exit(1);
});
