// A lab for "verify you are human" checks in equipe's browser panel — with
// no real check involved. Two local sites stand in: a page on 127.0.0.1 that
// holds a cross-site iframe from localhost (Chrome puts it in its own process,
// as it does Cloudflare's challenge frame), and in that frame a box that, like
// such widgets, only says "passed" for a click it saw as a person's: the page
// visible, drawing (requestAnimationFrame), focused, the pointer moved onto
// the box before pressing. Both frames say what they observe — visibility,
// focus, frames drawn, timers, intersection, lifecycle, each mouse/pointer
// event — to the lab's server.
//
// It drives equipe's browser host (crates/core/assets/browser-host.cjs) the
// way the daemon and the panel do: a member's lane navigates there with its
// tools, the panel watches it and lays it out at its box, the person takes it
// (직접 하기), their pointer moves over the picture and clicks the box, then
// hands it back. Every phase prints what the frames saw.
//
//   npx -y -p playwright@1.64.0 node scripts/browser-check-lab.cjs [path/to/browser-host.cjs]
//
// Opens Google Chrome with a throwaway profile. Exits 0 when the click passed
// and the page was visible while the person drove it. Not run in CI (it needs
// Chrome and a desktop session); docs/BROWSER-CHECKS.md has its findings.
const path = require('node:path');
const fs = require('node:fs');
const os = require('node:os');
const net = require('node:net');
const http = require('node:http');
const { spawn } = require('node:child_process');

const assets = path.join(__dirname, '..', 'crates', 'core', 'assets');
const hostSource = process.argv[2] ? path.resolve(process.argv[2]) : path.join(assets, 'browser-host.cjs');
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'equipe-check-lab-'));
const script = path.join(dir, 'browser-host.cjs');
fs.writeFileSync(script, fs.readFileSync(hostSource, 'utf8').replace('__DRIVING__', fs.readFileSync(path.join(assets, 'driving.js'), 'utf8')));
fs.writeFileSync(path.join(dir, 'state.json'), '{"cookies":[],"origins":[]}');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failed = 0;
const check = (ok, what) => {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`);
  if (!ok) failed++;
};
const freePort = () => new Promise((resolve) => {
  const s = net.createServer().listen(0, () => { const { port } = s.address(); s.close(() => resolve(port)); });
});

// What each frame observes, run in it: posted to /report every 250 ms.
const OBSERVER = (who) => `
(() => {
  const who = ${JSON.stringify(who)};
  const t0 = performance.now();
  let raf = 0, ticks = 0, io = 0, intersecting = null, lifecycle = [];
  const events = [];
  (function f() { raf++; requestAnimationFrame(f); })();
  setInterval(() => ticks++, 20);
  const box = document.getElementById('box') || document.body;
  new IntersectionObserver((es) => { io++; intersecting = es[es.length - 1].isIntersecting; }).observe(box);
  for (const t of ['visibilitychange', 'freeze', 'resume', 'pageshow', 'pagehide']) document.addEventListener(t, () => lifecycle.push(t + ':' + document.visibilityState), true);
  for (const t of ['focus', 'blur']) window.addEventListener(t, () => lifecycle.push('window-' + t));
  const kinds = ['pointerover', 'pointerenter', 'pointermove', 'pointerdown', 'pointerup', 'mouseover', 'mouseenter', 'mousemove', 'mousedown', 'mouseup', 'click'];
  for (const t of kinds) {
    box.addEventListener(t, (e) => {
      if (events.length > 200) return;
      events.push({ t, trusted: e.isTrusted, pointerType: e.pointerType, x: Math.round(e.clientX), y: Math.round(e.clientY),
        sx: Math.round(e.screenX), sy: Math.round(e.screenY), pressure: e.pressure, buttons: e.buttons, detail: e.detail,
        visible: document.visibilityState, focus: document.hasFocus() });
    });
  }
  // The stand-in check: a click it believes, or why not.
  let verdict = null, lastRaf = 0, rafAt = 0, moved = false;
  setInterval(() => { if (raf !== lastRaf) { lastRaf = raf; rafAt = performance.now(); } }, 50);
  box.addEventListener('pointermove', () => { moved = true; });
  box.addEventListener('click', (e) => {
    const why = [];
    if (!e.isTrusted) why.push('untrusted');
    if (document.visibilityState !== 'visible') why.push('hidden');
    if (!document.hasFocus()) why.push('unfocused');
    if (performance.now() - rafAt > 500) why.push('not-drawing');
    if (!moved) why.push('no-pointer-move');
    if (intersecting !== true) why.push('not-intersecting');
    verdict = why.length ? { passed: false, why } : { passed: true };
    box.textContent = verdict.passed ? 'passed' : 'refused: ' + why.join(', ');
  });
  // Whether a DevTools client listens to this frame's console (its Runtime
  // domain on): logging an error reads its stack only then. Measured, not hidden.
  const devtools = () => { let read = false; const e = new Error(); Object.defineProperty(e, 'stack', { get() { read = true; return ''; } }); console.debug(e); return read; };
  let lastR = 0, lastT = 0, lastAt = performance.now();
  const report = () => {
    const now = performance.now();
    const dt = (now - lastAt) / 1000;
    const r = box.getBoundingClientRect();
    const body = {
      who, at: Math.round(now - t0), visibility: document.visibilityState, hidden: document.hidden, focus: document.hasFocus(),
      rafPerSec: Math.round((raf - lastR) / dt), ticksPerSec: Math.round((ticks - lastT) / dt), io, intersecting,
      lifecycle: lifecycle.slice(), events: events.slice(), verdict, cookie: document.cookie,
      box: { x: r.left, y: r.top, w: r.width, h: r.height },
      frame: window.frameElement ? null : (() => { const f = document.querySelector('iframe'); if (!f) return null; const b = f.getBoundingClientRect(); return { x: b.left, y: b.top, w: b.width, h: b.height }; })(),
      win: { x: window.screenX, y: window.screenY, innerW: innerWidth, innerH: innerHeight, outerW: outerWidth, outerH: outerHeight },
      screen: { w: screen.width, h: screen.height, availW: screen.availWidth, availH: screen.availHeight, dpr: devicePixelRatio },
      pointerFine: matchMedia('(pointer: fine)').matches, hover: matchMedia('(hover: hover)').matches,
      webdriver: navigator.webdriver, devtools: devtools(),
      globals: Object.getOwnPropertyNames(window).filter((n) => /^__|playwright|equipe|cdc_/i.test(n)),
    };
    lastR = raf; lastT = ticks; lastAt = now;
    fetch('/report', { method: 'POST', body: JSON.stringify(body), keepalive: true }).catch(() => {});
  };
  // Timers are what a hidden page throttles: a report goes from each, so one keeps coming.
  setInterval(report, 250);
})();
`;

(async () => {
  const reports = new Map(); // who -> last report
  const history = []; // every report, in order
  const serve = (handler) => http.createServer((req, res) => {
    if (req.method === 'POST' && req.url === '/report') {
      let body = '';
      req.on('data', (c) => { body += c; });
      req.on('end', () => { try { const r = JSON.parse(body); reports.set(r.who, r); history.push(r); } catch { /* a cut report */ } res.end(); });
      return;
    }
    handler(req, res);
  });
  const widgetPort = await freePort();
  const widget = serve((req, res) => {
    res.setHeader('content-type', 'text/html; charset=utf-8');
    // A cookie set in the cross-site frame, as a challenge frame keeps its own.
    res.setHeader('set-cookie', 'lab3p=1; SameSite=None; Secure; Path=/');
    res.end(`<!doctype html><title>widget</title><style>body{margin:0;font:13px system-ui;background:#f6f6f6}
      #box{position:absolute;left:16px;top:16px;width:200px;height:40px;border:2px solid #888;display:grid;place-items:center}</style>
      <div id=box>I am human</div><div id=spin style="position:absolute;left:230px;top:26px"></div>
      <script>setInterval(() => { spin.textContent = '|/-+'[Math.floor(Date.now() / 100) % 4]; }, 100);</script><script>${OBSERVER('widget')}</script>`);
  }).listen(widgetPort, 'localhost');
  const site = serve((req, res) => {
    res.setHeader('content-type', 'text/html; charset=utf-8');
    res.end(`<!doctype html><title>Just a moment...</title><style>body{margin:0;font:15px system-ui}</style>
      <h1 style="margin:24px">example.test</h1><p style="margin:24px">Verify you are human by completing the action below.</p>
      <div id=box style="position:absolute;left:0;top:0;width:100%;height:100%;pointer-events:none"></div>
      <iframe src="http://localhost:${widgetPort}/" style="position:absolute;left:40px;top:160px;width:300px;height:80px;border:0"></iframe>
      <script>${OBSERVER('top')}</script>`);
  }).listen(0, '127.0.0.1');
  await new Promise((r) => site.once('listening', r));
  const base = `http://127.0.0.1:${site.address().port}`;

  const host = spawn(process.execPath, [script, path.join(dir, 'chrome'), path.join(dir, 'state.json')], {
    stdio: ['pipe', 'pipe', 'inherit'],
    env: { ...process.env, EQUIPE_FIRST_PAGE: String((Math.floor(Date.now() / 1000) % 2 ** 32) * 2 ** 20 + 1) },
  });
  const messages = [];
  let frames = 0;
  let buf = Buffer.alloc(0);
  host.stdout.on('data', (chunk) => {
    buf = Buffer.concat([buf, chunk]);
    while (buf.length >= 5) {
      const len = buf.readUInt32BE(0);
      if (buf.length < 4 + len) break;
      const kind = buf.readUInt8(4);
      const payload = buf.subarray(5, 4 + len);
      buf = buf.subarray(4 + len);
      if (kind === 1) messages.push(JSON.parse(payload.toString()));
      else if (kind === 2) frames++;
    }
  });
  const op = (o) => host.stdin.write(JSON.stringify(o) + '\n');
  const until = async (what, test, ms = 20000) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      const found = test();
      if (found) return found;
      await sleep(100);
    }
    check(false, `${what} (timed out)`);
    return null;
  };
  if (!(await until('host ready', () => messages.find((m) => m.type === 'ready'), 60000))) process.exit(1);

  const port = await freePort();
  const lane = 'lab/pm';
  op({ op: 'lane', id: lane, member: 'pm', color: '120, 190, 240', port, output: path.join(dir, 'out'), shown: false });
  const mcp = async (tool, args) => {
    const url = `http://localhost:${port}/mcp`;
    const post = async (body, session) => {
      const headers = { 'content-type': 'application/json', accept: 'application/json, text/event-stream' };
      if (session) headers['mcp-session-id'] = session;
      return fetch(url, { method: 'POST', headers, body: JSON.stringify(body) });
    };
    let init;
    for (let i = 0; i < 100; i++) {
      try { init = await post({ jsonrpc: '2.0', id: 0, method: 'initialize', params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'lab', version: '1' } } }); break; } catch { await sleep(200); }
    }
    const session = init.headers.get('mcp-session-id');
    await init.text();
    await (await post({ jsonrpc: '2.0', method: 'notifications/initialized' }, session)).text();
    const raw = await (await post({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: tool, arguments: args } }, session)).text();
    const line = raw.split('\n').find((l) => l.startsWith('data:'));
    return JSON.parse(line ? line.slice(5) : raw).result;
  };
  await mcp('browser_navigate', { url: `${base}/` });
  const tabs = await until('the lane reports its tab', () => [...messages].reverse().find((m) => m.type === 'tabs' && m.lane === lane && m.agent));
  const page = tabs.agent;
  await until('both frames report', () => reports.get('top') && reports.get('widget'));


  const phase = async (name, ms = 2000) => {
    const before = frames;
    reports.clear();
    const from = history.length;
    await sleep(ms);
    await until(`${name}: reports`, () => reports.get('top') && reports.get('widget'), 8000);
    const t = reports.get('top') || {};
    const w = reports.get('widget') || {};
    const fps = Math.round((frames - before) / (ms / 1000));
    console.log(`\n── ${name}`);
    console.log(`   top:    visibility=${t.visibility} focus=${t.focus} rAF/s=${t.rafPerSec} timer/s=${t.ticksPerSec}`);
    console.log(`           window=${JSON.stringify(t.win)} screen=${JSON.stringify(t.screen)}`);
    console.log(`   widget: visibility=${w.visibility} focus=${w.focus} rAF/s=${w.rafPerSec} timer/s=${w.ticksPerSec} intersecting=${w.intersecting} cookie=${JSON.stringify(w.cookie)} lifecycle=${(w.lifecycle || []).join(',')}`);
    console.log(`   widget: window=${JSON.stringify(w.win)} screen=${JSON.stringify(w.screen)} pointer:fine=${w.pointerFine} hover=${w.hover}`);
    console.log(`           webdriver=${w.webdriver} devtools-console=${w.devtools} globals=${JSON.stringify(w.globals)}`);
    console.log(`   screencast frames/s=${fps}`);
    const series = (who, k) => history.slice(from).filter((r) => r.who === who).map((r) => r[k]).join(' ');
    console.log(`   widget rAF/s each 250 ms: ${series('widget', 'rafPerSec')}`);
    console.log(`   top timer/s each 250 ms:  ${series('top', 'ticksPerSec')}`);
    return { t, w, fps };
  };

  // Their pointer comes in over the picture and clicks the box, as the panel sends it.
  const clickBox = async (top) => {
    const target = { x: top.frame.x + 16 + 100, y: top.frame.y + 16 + 20 };
    const steps = 12;
    for (let i = 0; i <= steps; i++) {
      const x = 5 + ((target.x - 5) * i) / steps;
      const y = 5 + ((target.y - 5) * i) / steps;
      op({ op: 'input', page, ev: { t: 'mouse', type: 'mouseMoved', x, y, button: 'none', buttons: 0, clickCount: 0 } });
      await sleep(16);
    }
    await sleep(120);
    op({ op: 'input', page, ev: { t: 'mouse', type: 'mousePressed', x: target.x, y: target.y, button: 'left', buttons: 1, clickCount: 1 } });
    await sleep(90);
    op({ op: 'input', page, ev: { t: 'mouse', type: 'mouseReleased', x: target.x, y: target.y, button: 'left', buttons: 0, clickCount: 1 } });
  };
  const clickPhase = async (name) => {
    const r = await phase(name, 1500);
    const events = r.w.events || [];
    console.log(`   events in the frame: ${events.map((e) => e.t).join(' ')}`);
    const click = [...events].reverse().find((e) => e.t === 'click');
    const down = [...events].reverse().find((e) => e.t === 'pointerdown');
    if (down) console.log(`   pointerdown: ${JSON.stringify(down)}`);
    if (click) console.log(`   click:       ${JSON.stringify(click)}`);
    console.log(`   verdict: ${JSON.stringify(r.w.verdict)}`);
    return { ...r, events, click, down };
  };

  const unwatched = await phase('a member drives it, nobody watching');

  // The panel watches it, laid out at its box (as `Watch { fit }` asks).
  op({ op: 'watch', page, maxW: 1120, maxH: 1400, quality: 60, fps: 30 });
  op({ op: 'fit', page, w: 560, h: 700, dpr: 2 });
  const watched = await phase('a member drives it, watched in the panel');

  // 직접 하기: the person takes it.
  op({ op: 'control', lane, by: 'person' });
  await until('control to the person', () => messages.find((m) => m.type === 'control' && m.lane === lane && m.by === 'person'));
  const held = await phase('the person has it in the panel');
  await clickBox(held.t);
  const clicked = await clickPhase('after the person\'s click in the panel');

  // For comparison: the same page in a real window (Chrome에서 열기), clicked the same way.
  if (process.env.LAB_POPOUT) {
    op({ op: 'popout', page, bounds: { left: 80, top: 80, width: 700, height: 760 } });
    const out = await phase('in a real window (popout)');
    await clickBox(out.t);
    await clickPhase('after a click in the real window');
  }

  // Handed back: parked again.
  op({ op: 'control', lane, by: null });
  await until('control back', () => messages.filter((m) => m.type === 'control' && m.lane === lane && m.by === null).length >= 1);
  const back = await phase('handed back to the member');

  console.log('\n── summary');
  console.log(`     nobody watching: visibility=${unwatched.w.visibility} rAF/s=${unwatched.w.rafPerSec}; watched: visibility=${watched.w.visibility} rAF/s=${watched.w.rafPerSec}`);
  const { click, down, events } = clicked;
  check(held.w.visibility === 'visible', `while the person drives it the challenge frame is visible (${held.w.visibility})`);
  check(held.w.rafPerSec >= 20, `while the person drives it the challenge frame draws (${held.w.rafPerSec} rAF/s)`);
  check(held.t.focus === true, `while the person drives it the page has focus (${held.t.focus})`);
  check(held.fps >= 5, `the panel's picture keeps coming while the person drives it (${held.fps} frames/s)`);
  check(!!click && click.trusted && down?.pointerType === 'mouse', 'the person\'s click reaches the cross-site frame, trusted, as a mouse');
  check(!!click && Math.abs(click.x - (16 + 100)) <= 2 && Math.abs(click.y - (16 + 20)) <= 2, `it lands where they clicked (${click?.x},${click?.y} in the frame)`);
  check(!!click && click.focus === true && click.visible === 'visible', 'at the click the frame is visible and focused');
  const over = events.findIndex((e) => e.t === 'pointerover');
  check(over >= 0 && over < events.findIndex((e) => e.t === 'pointerdown'), 'the pointer came over the box before pressing');
  check(clicked.w.verdict?.passed === true, `the stand-in check passes (${JSON.stringify(clicked.w.verdict)})`);
  const s = held.w.screen || {};
  const sy = click ? click.sy : 0;
  check(!!click && sy >= 0 && sy <= s.h, `the click's screen position is on the page's screen (screenY ${sy} of ${s.h})`);
  const win = held.t.win || {};
  check(win.outerH > win.innerH && win.y >= 0 && win.y < s.h, `while the person drives it the page is in a real window on the screen (at y ${win.y}, ${win.outerH - win.innerH}px of toolbar)`);
  check(win.innerW === 560 && win.innerH === 700, `laid out at the panel's box (${win.innerW}×${win.innerH})`);
  check(down?.pressure === 0.5 && click?.pressure === 0, `pressed, the pointer's pressure is a mouse's (${down?.pressure}, then ${click?.pressure})`);
  const again = await mcp('browser_evaluate', { function: '() => document.title' });
  check(!again.isError, 'handed back, its member works in it again');
  check(back.w.visibility === 'visible' && back.fps > 0, `parked again, it still runs and is pictured (${back.w.visibility}, ${back.fps} frames/s)`);

  host.stdin.end();
  await sleep(1500);
  host.kill();
  site.close();
  widget.close();
  fs.rmSync(dir, { recursive: true, force: true });
  console.log(failed ? `\n${failed} failed` : '\nall passed');
  process.exit(failed ? 1 : 0);
})().catch((err) => {
  console.error(err);
  process.exit(1);
});
