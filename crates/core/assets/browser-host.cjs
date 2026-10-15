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
//   {"op":"newtab","url":"…"}  {"op":"closetab","page":3}  {"op":"control","lane":"squad/name","by":"person"|null}
//   {"op":"goto","lane":"squad/name","url":"…"}  {"op":"dialog","page":3,"accept":true,"text":"…"}
//   {"op":"popout","page":3,"bounds":{"left":0,"top":0,"width":900,"height":800}}
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

// Google's sign-in is this browser's own; the sign-ins file never carries it.
const own = (domain) => /(^|\.)(google\.[a-z]{2,3}(\.[a-z]{2})?|youtube\.com|googleusercontent\.com)$/.test(String(domain).replace(/^\./, ''));

const PERSON = '';
const PLACEHOLDER = 'data:text/html;charset=utf-8,' + encodeURIComponent(
  '<!doctype html><title>equipe</title><meta name="color-scheme" content="light dark">'
  + '<body style="font: 14px -apple-system, system-ui; display: grid; place-items: center; height: 90vh; color: #888">'
  + 'equipe — members’ tabs are behind this one</body>');
const BUSY = 'The person is using this browser; wait or ask them.';
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
  const lanes = new Map(); // lane id -> { events, member, color, shown, control, lastInput, agent }
  const ids = new Map(); // page -> page id
  const pages = new Map(); // page id -> page
  const cdps = new Map(); // page -> CDP session, for watching and input
  const dialogs = new Map(); // page id -> the dialog waiting for an answer
  const watches = new Map(); // page id -> { cdp, quality, maxW, maxH, fps, seq, lastAck }
  let nextId = 1;
  let claim = null;
  let queue = Promise.resolve();
  let parking = null; // { page, cdp, windowId }

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
  const describe = async (page) => ({
    id: idOf(page),
    url: page.url(),
    title: await page.title().catch(() => ''),
    loading: !!page.__equipeLoading,
  });
  // A title a page changes in place (a count of unread mail, a dialog's
  // answer) is told too: looked at every moment or so.
  const titles = new Map();
  setInterval(async () => {
    for (const page of context.pages()) {
      if (!owner.has(page)) continue;
      const title = await page.title().catch(() => null);
      if (title !== null && titles.get(page) !== title) {
        if (titles.has(page)) report(owner.get(page));
        titles.set(page, title);
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

  // Who a page works for, as the overlay asks it.
  await context.exposeBinding('__equipeWho', ({ page }) => {
    const lane = lanes.get(owner.get(page));
    return lane ? { member: lane.member, color: lane.color } : null;
  });
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
  let personShown = null; // a page the person asked for on screen
  const restack = async () => {
    const park = await ensureParking();
    const shown = [...lanes.entries()].filter(([, l]) => l.shown).map(([id]) => id);
    const front = (personShown && !personShown.isClosed() ? personShown : null)
      || shown.map((id) => pages.get(lanes.get(id).agent) || tabsOf(id).at(-1)).find((p) => p && !p.isClosed());
    if (front) {
      const { windowId } = await windowOf(front).catch(() => ({}));
      if (windowId) await setState(windowId, 'normal');
      await raise(front);
      return;
    }
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
      if (frame !== page.mainFrame()) return;
      page.__equipeLoading = true;
      report(id);
    });
    page.on('load', () => { page.__equipeLoading = false; report(id); });
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
      cdps.get(page)?.then((c) => c.detach()).catch(() => {});
      cdps.delete(page);
      if (dialogs.delete(pid)) emit({ type: 'dialog_closed', page: pid });
      const lane = lanes.get(id);
      if (lane && lane.agent === pid) lane.agent = null;
      if (personShown === page) personShown = null;
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

  const openLane = async ({ id, member, color, port, output, shown }) => {
    if (lanes.has(id)) return show({ id, shown });
    const lane = { events: new EventEmitter(), member, color, shown: !!shown, control: null, lastInput: 0, agent: null };
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
        // While the person uses this member's browser its tools wait for them;
        // after each call, the tab it worked in is the one the screen follows.
        const call = backend.callTool.bind(backend);
        backend.callTool = async (name, args, signal) => {
          if (lane.control === 'person') return { content: [{ type: 'text', text: `### Error\n${BUSY}` }], isError: true };
          const result = await call(name, args, signal);
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
    await restack();
  };
  const close = async ({ id }) => {
    for (const page of tabsOf(id)) await page.close().catch(() => {});
  };
  const person = async ({ url }) => {
    const page = await openTab(PERSON, url);
    personShown = page;
    await restack();
  };

  // ── watching a page: a screencast, paced by when frames are taken ──

  const stopWatch = (pid) => {
    const w = watches.get(pid);
    if (!w) return;
    watches.delete(pid);
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
    await cdp.send('Page.startScreencast', { format: 'jpeg', quality, maxWidth: maxW, maxHeight: maxH, everyNthFrame: 1, maxFramesInFlight: 1 }).catch(() => stopWatch(pid));
  };
  const unwatch = ({ page }) => stopWatch(page);

  // ── the person's hand on a page ──

  const setControl = async (laneId, by) => {
    const lane = lanes.get(laneId);
    if (!lane || lane.control === by) return;
    lane.control = by;
    lane.lastInput = Date.now();
    for (const page of tabsOf(laneId)) {
      const cdp = await cdpOf(page);
      await cdp.send('Emulation.setFocusEmulationEnabled', { enabled: true }).catch(() => {});
    }
    emit({ type: 'control', lane: laneId, by });
  };
  setInterval(() => {
    for (const [id, lane] of lanes) {
      if (lane.control === 'person' && Date.now() - lane.lastInput > IDLE_CONTROL_MS) setControl(id, null);
    }
  }, 10000);

  const input = async ({ page: pid, ev }) => {
    const page = pages.get(pid);
    if (!page || !ev) return;
    const laneId = owner.get(page);
    const lane = lanes.get(laneId);
    if (lane) {
      if (lane.control !== 'person') await setControl(laneId, 'person');
      lane.lastInput = Date.now();
    }
    const cdp = await cdpOf(page);
    switch (ev.t) {
      case 'mouse':
        return cdp.send('Input.dispatchMouseEvent', {
          type: ev.type, x: ev.x, y: ev.y, button: ev.button || 'none', buttons: ev.buttons || 0,
          clickCount: ev.clickCount || 0, modifiers: ev.modifiers || 0,
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
  const control = ({ lane, by }) => setControl(lane, by || null);
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

  const ops = { lane: openLane, show, close, person, watch, unwatch, input, nav, goto, newtab, closetab, control, dialog, popout };
  // Input goes in order, each after the one before; the rest as they come.
  let inputs = Promise.resolve();
  readline.createInterface({ input: process.stdin }).on('line', (line) => {
    let request;
    try { request = JSON.parse(line); } catch { return; }
    const run = () => Promise.resolve(ops[request.op]?.(request)).catch((err) => console.error(`equipe browser host: ${request.op}: ${err && err.message}`));
    if (request.op === 'input') inputs = inputs.then(run);
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
