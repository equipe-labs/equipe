// A smoke test of equipe's browser host (crates/core/assets/browser-host.cjs)
// against a throwaway Chrome profile: a lane, its tools, the tabs it reports,
// a watched page's frames, the person's input (Korean composition included),
// the agent waiting while the person has the page, navigation, a JS dialog,
// and a screenshot of a member's tab while its window is out of sight.
//
//   npx -y -p playwright@1.64.0 node scripts/browser-host-smoke.cjs
//
// Opens Google Chrome (minimized); exits 0 when every check passed.
const path = require('node:path');
const fs = require('node:fs');
const os = require('node:os');
const net = require('node:net');
const http = require('node:http');
const { spawn } = require('node:child_process');

const assets = path.join(__dirname, '..', 'crates', 'core', 'assets');
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'equipe-host-smoke-'));
const script = path.join(dir, 'browser-host.cjs');
fs.writeFileSync(script, fs.readFileSync(path.join(assets, 'browser-host.cjs'), 'utf8').replace('__DRIVING__', fs.readFileSync(path.join(assets, 'driving.js'), 'utf8')).replace('__GATE__', fs.readFileSync(path.join(assets, 'gate.js'), 'utf8')));
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

(async () => {
  const site = http.createServer((req, res) => {
    res.setHeader('content-type', 'text/html; charset=utf-8');
    if (req.url === '/form') return res.end('<title>Form</title><input id=q autofocus style="font-size:30px">');
    if (req.url === '/dialog') return res.end('<title>Dialog</title><button id=b onclick="document.title = confirm(\'Sure?\') ? \'yes\' : \'no\'">go</button>');
    // A stand-in for Cloudflare's wall (no real site is touched): its title, its
    // words, a challenge box that passes when clicked, then the page.
    if (req.url === '/wall') {
      return res.end('<title>Just a moment...</title><h1>example.test</h1><p>Verify you are human by completing the action below.</p>'
        + '<div id=challenge-form style="width:300px;height:65px;border:1px solid #ccc" onclick="location.href=\'/passed\'">I am human</div>'
        + '<input type=hidden name="cf-turnstile-response" value="">');
    }
    if (req.url === '/signin') return res.end('<title>Sign in</title><link rel=icon href="/icon.png"><input name=user><input type=password name=pw>');
    res.end(`<title>Page ${req.url}</title><h1>${req.headers.host}${req.url}</h1>`);
  }).listen(0);
  await new Promise((r) => site.once('listening', r));
  const base = `http://127.0.0.1:${site.address().port}`;
  const other = `http://localhost:${site.address().port}`;

  const host = spawn(process.execPath, [script, path.join(dir, 'chrome'), path.join(dir, 'state.json')], { stdio: ['pipe', 'pipe', 'inherit'], env: { ...process.env, EQUIPE_FIRST_PAGE: String((Math.floor(Date.now() / 1000) % 2 ** 32) * 2 ** 20 + 1) } });
  const messages = [];
  const frames = [];
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
      else if (kind === 2) {
        const hlen = payload.readUInt16BE(0);
        const header = JSON.parse(payload.subarray(2, 2 + hlen).toString());
        frames.push({ header, jpeg: payload.subarray(2 + hlen) });
      }
    }
  });
  const op = (o) => host.stdin.write(JSON.stringify(o) + '\n');
  const until = async (what, test, ms = 15000) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      const found = test();
      if (found) return found;
      await sleep(100);
    }
    check(false, `${what} (timed out)`);
    return null;
  };
  const lastTabs = (lane) => [...messages].reverse().find((m) => m.type === 'tabs' && m.lane === lane);

  check(!!(await until('host ready', () => messages.find((m) => m.type === 'ready'), 60000)), 'the host says it is ready');

  // A lane, and a call of its tools as a member's turn makes.
  const port = await freePort();
  const lane = 'acme/frontend';
  op({ op: 'lane', id: lane, member: 'frontend', color: '120, 190, 240', port, output: path.join(dir, 'out'), shown: false });
  // One turn of a member: calls on one session, which keeps the tab it is on.
  const turn = async () => {
    const url = `http://localhost:${port}/mcp`;
    const post = async (body, session) => {
      const headers = { 'content-type': 'application/json', accept: 'application/json, text/event-stream' };
      if (session) headers['mcp-session-id'] = session;
      return fetch(url, { method: 'POST', headers, body: JSON.stringify(body) });
    };
    const init = await post({ jsonrpc: '2.0', id: 0, method: 'initialize', params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'smoke', version: '1' } } });
    const session = init.headers.get('mcp-session-id');
    await init.text();
    await (await post({ jsonrpc: '2.0', method: 'notifications/initialized' }, session)).text();
    let n = 1;
    return async (tool, args) => {
      const raw = await (await post({ jsonrpc: '2.0', id: n++, method: 'tools/call', params: { name: tool, arguments: args } }, session)).text();
      const line = raw.split('\n').find((l) => l.startsWith('data:'));
      return JSON.parse(line ? line.slice(5) : raw).result;
    };
  };
  const mcp = async (tool, args) => {
    const url = `http://localhost:${port}/mcp`;
    const post = async (body, session) => {
      const headers = { 'content-type': 'application/json', accept: 'application/json, text/event-stream' };
      if (session) headers['mcp-session-id'] = session;
      return fetch(url, { method: 'POST', headers, body: JSON.stringify(body) });
    };
    let init;
    for (let i = 0; i < 100; i++) {
      try { init = await post({ jsonrpc: '2.0', id: 0, method: 'initialize', params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'smoke', version: '1' } } }); break; } catch { await sleep(200); }
    }
    const session = init.headers.get('mcp-session-id');
    await init.text();
    await (await post({ jsonrpc: '2.0', method: 'notifications/initialized' }, session)).text();
    const raw = await (await post({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: tool, arguments: args } }, session)).text();
    const line = raw.split('\n').find((l) => l.startsWith('data:'));
    return JSON.parse(line ? line.slice(5) : raw).result;
  };
  const went = await mcp('browser_navigate', { url: `${base}/form` });
  check(!went.isError, 'a member navigates in its lane');
  const tabs = await until('the lane reports its tab', () => { const t = lastTabs(lane); return t && t.pages.some((p) => p.url.endsWith('/form') && p.title === 'Form') && t.agent ? t : null; });
  const page = tabs?.agent;
  check(!!page, `the tab the member works in is reported (page ${page})`);
  check(page >= 1_000_000_000, `page ids go on from where the daemon says, not from 1 (${page})`);
  const reported = tabs?.pages.find((p) => p.id === page);
  check(reported?.favicon === `${base}/favicon.ico` && reported?.signals && reported.signals.password === false, 'a member\'s tab comes with its icon and what is read off it');

  // Watched: a frame comes, with the page's size in CSS pixels.
  op({ op: 'watch', page, maxW: 800, maxH: 800, quality: 60, fps: 30 });
  const first = await until('a first frame', () => frames.find((f) => f.header.page === page));
  check(first && first.jpeg[0] === 0xff && first.jpeg[1] === 0xd8 && first.header.w > 0 && first.header.cssW > 0, `a JPEG frame ${first?.header.w}×${first?.header.h} of a ${first?.header.cssW}×${first?.header.cssH} page`);

  // The person's hand: a click into the field and Korean typed with composition.
  const before = frames.length;
  const box = { x: 40, y: 20 };
  op({ op: 'input', page, ev: { t: 'mouse', type: 'mousePressed', x: box.x, y: box.y, button: 'left', buttons: 1, clickCount: 1 } });
  op({ op: 'input', page, ev: { t: 'mouse', type: 'mouseReleased', x: box.x, y: box.y, button: 'left', buttons: 0, clickCount: 1 } });
  op({ op: 'input', page, ev: { t: 'ime', text: 'ㅎ' } });
  op({ op: 'input', page, ev: { t: 'ime', text: '하' } });
  op({ op: 'input', page, ev: { t: 'ime', text: '한' } });
  op({ op: 'input', page, ev: { t: 'insert', text: '한' } });
  op({ op: 'input', page, ev: { t: 'insert', text: '글' } });
  op({ op: 'input', page, ev: { t: 'key', type: 'keyDown', key: 'a', code: 'KeyA', vk: 65, text: 'a' } });
  op({ op: 'input', page, ev: { t: 'key', type: 'keyUp', key: 'a', code: 'KeyA', vk: 65 } });
  check(!!(await until('control goes to the person', () => messages.find((m) => m.type === 'control' && m.lane === lane && m.by === 'person'))), 'the person\'s input takes the page');
  await until('frames while typing', () => frames.length > before);
  check(frames.length > before, 'the page keeps sending frames as it changes');

  // The member waits meanwhile.
  const busy = await mcp('browser_snapshot', {});
  check(busy.isError && /person is using this tab/.test(busy.content[0].text), 'a member\'s tool call waits while the person has the page');
  // Only that tab is theirs: another tab its member opens, uses and closes.
  const call = await turn();
  await call('browser_tabs', { action: 'list' });
  const opened = await call('browser_tabs', { action: 'new' });
  check(!opened.isError, 'meanwhile its member opens another tab');
  const there = await call('browser_navigate', { url: `${base}/other` });
  check(!there.isError, 'and works there');
  if (there.isError) console.log(JSON.stringify(there).slice(0, 300));
  const shut = await call('browser_tabs', { action: 'close', index: 0 });
  check(shut.isError && /using this tab/.test(shut.content[0].text), 'but can\'t close the tab the person is on');
  check(!!(await until('the person asked about closing', () => messages.find((m) => m.type === 'wants' && m.lane === lane && m.page === page && m.action === 'close'))), 'the person is asked whether to let it close');
  const closedOwn = await call('browser_tabs', { action: 'close', index: 1 });
  check(!closedOwn.isError, 'its own other tab it closes');
  if (closedOwn.isError) console.log(JSON.stringify(closedOwn).slice(0, 300));
  await until('the other tab closed', () => !lastTabs(lane)?.pages.some((p) => p.url.endsWith('/other')));
  op({ op: 'control', lane, by: null });
  await until('control given back', () => messages.find((m) => m.type === 'control' && m.lane === lane && m.by === null));
  const typed = await mcp('browser_evaluate', { function: '() => document.getElementById("q").value' });
  const text = typed.content.map((c) => c.text).join('\n');
  check(/한글a/.test(text), `what the person typed reached the page (${/"([^"]*)"/.exec(text)?.[1]})`);
  if (!/한글a/.test(text)) console.log(text);

  // The pointer passing over a member's page (or a wheel) doesn't take it.
  messages.length = 0;
  for (const x of [10, 30, 60]) op({ op: 'input', page, ev: { t: 'mouse', type: 'mouseMoved', x, y: 20, button: 'none', buttons: 0, clickCount: 0 } });
  op({ op: 'input', page, ev: { t: 'wheel', x: 60, y: 20, dx: 0, dy: 40 } });
  await sleep(500);
  const free = await mcp('browser_evaluate', { function: '() => document.title' });
  check(!messages.some((m) => m.type === 'control') && !free.isError, 'a pointer passing over a member\'s page doesn\'t take it');

  // Out of sight, a member's screenshot still works — across sites, too.
  await mcp('browser_navigate', { url: `${other}/elsewhere` });
  const shot = await mcp('browser_take_screenshot', {});
  check(!shot.isError && shot.content.some((c) => c.type === 'image'), 'a member\'s screenshot of its hidden tab works after a cross-site navigation');
  if (shot.isError) console.log(JSON.stringify(shot).slice(0, 600));
  const after = frames.length;
  op({ op: 'nav', page, action: 'back' });
  await until('back', () => { const t = lastTabs(lane); return t?.pages.find((p) => p.id === page)?.url.endsWith('/form'); });
  check(lastTabs(lane).pages.find((p) => p.id === page).url.endsWith('/form'), 'back goes back');
  check(!!(await until('frames after navigation', () => frames.length > after)), 'frames keep coming after a cross-site navigation, window minimized');
  op({ op: 'nav', page, action: 'go', url: `${base.replace('http://', '')}/dialog` });
  await until('typed address', () => lastTabs(lane)?.pages.find((p) => p.id === page)?.url.endsWith('/dialog'));
  check(lastTabs(lane).pages.find((p) => p.id === page).url.endsWith('/dialog'), 'an address typed without its scheme opens');

  // A JS dialog: the screen is told, and answers it.
  op({ op: 'input', page, ev: { t: 'mouse', type: 'mousePressed', x: 20, y: 15, button: 'left', buttons: 1, clickCount: 1 } });
  op({ op: 'input', page, ev: { t: 'mouse', type: 'mouseReleased', x: 20, y: 15, button: 'left', buttons: 0, clickCount: 1 } });
  const asked = await until('a dialog', () => messages.find((m) => m.type === 'dialog' && m.page === page));
  check(asked?.kind === 'confirm' && asked?.message === 'Sure?', 'a confirm() reaches the screen');
  op({ op: 'dialog', page, accept: true });
  await until('the dialog answered', () => lastTabs(lane)?.pages.find((p) => p.id === page)?.title === 'yes');
  check(lastTabs(lane).pages.find((p) => p.id === page).title === 'yes', 'the screen\'s answer reaches the page');
  if (lastTabs(lane).pages.find((p) => p.id === page).title !== 'yes') { console.log(JSON.stringify(messages.filter((m) => m.type !== 'tabs').slice(-5)), JSON.stringify(lastTabs(lane))); console.log(JSON.stringify(await mcp('browser_evaluate', { function: '() => document.title' })).slice(0, 800)); }

  // A page that waits for a person: what the daemon decides it from is told.
  // (The dialog's click took the page; it goes back to the member first.)
  op({ op: 'control', lane, by: null });
  await until('given back after the dialog', () => [...messages].reverse().find((m) => m.type === 'control' && m.lane === lane)?.by === null);
  messages.length = 0;
  await mcp('browser_navigate', { url: `${base}/wall` });
  const wall = await until('the wall\'s signals', () => lastTabs(lane)?.pages.find((p) => p.url.endsWith('/wall') && p.signals?.widget));
  check(wall?.title === 'Just a moment...' && wall.signals.widget && !wall.signals.solved && /verify you are human/.test(wall.signals.text), 'a challenge page is told with its box in sight and its words');
  // equipe puts it in the person's hands for it: the member's tools say why, and wait.
  op({ op: 'control', lane, by: 'person', why: 'check' });
  await until('held for the check', () => messages.find((m) => m.type === 'control' && m.lane === lane && m.by === 'person'));
  const waiting = await mcp('browser_snapshot', {});
  check(waiting.isError && /waits for a person/.test(waiting.content[0].text), 'a member\'s tool call is told a page waits for the person');
  // Nobody's hand on it: its member may still leave the page it no longer needs.
  const away = await mcp('browser_navigate', { url: `${base}/wall#again` });
  check(!away.isError, 'a hold from detection alone never stops its member going elsewhere in the tab');
  // Passed by itself (no hand on it): back to the member, which is told it was turned away.
  messages.length = 0;
  op({ op: 'control', lane, by: null, ifUntouched: true });
  const back = await until('handed back untouched', () => messages.find((m) => m.type === 'control' && m.lane === lane && m.by === null));
  check(back?.refused === true, 'handed back by itself, saying its tools were turned away');
  // Touched, it stays the person's.
  op({ op: 'control', lane, by: 'person', why: 'check' });
  await until('held again', () => messages.find((m) => m.type === 'control' && m.lane === lane && m.by === 'person'));
  // A pointer passing over it is no hand on it; a click is.
  op({ op: 'input', page, ev: { t: 'mouse', type: 'mouseMoved', x: 5, y: 5, button: 'none', buttons: 0, clickCount: 0 } });
  op({ op: 'input', page, ev: { t: 'mouse', type: 'mousePressed', x: 5, y: 5, button: 'left', buttons: 1, clickCount: 1 } });
  op({ op: 'input', page, ev: { t: 'mouse', type: 'mouseReleased', x: 5, y: 5, button: 'left', buttons: 0, clickCount: 1 } });
  // (Its window comes up for the person meanwhile: input waits for it.)
  await sleep(1500);
  messages.length = 0;
  op({ op: 'control', lane, by: null, ifUntouched: true });
  await sleep(800);
  check(!messages.some((m) => m.type === 'control' && m.lane === lane), 'a page the person touched isn\'t handed back by itself');
  // Watched by a panel, laid out at its picture — in the person's hands or its member's.
  const own = frames.at(-1)?.header;
  op({ op: 'fit', page, w: 500, h: 700, dpr: 2 });
  const fitted = await until('a frame at the panel\'s size', () => frames.slice(-3).find((f) => f.header.page === page && f.header.cssW === 500 && f.header.cssH === 700));
  if (!fitted) console.log(JSON.stringify(frames.slice(-3).map((f) => f.header)));
  check(!!fitted, `a page the person drives fills the panel (${fitted?.header.cssW}×${fitted?.header.cssH} css, ${fitted?.header.w}×${fitted?.header.h} px)`);
  op({ op: 'control', lane, by: null });
  await until('given back', () => [...messages].reverse().find((m) => m.type === 'control' && m.lane === lane)?.by === null);
  const width = await mcp('browser_evaluate', { function: '() => innerWidth' });
  check(/500/.test(width.content.map((c) => c.text).join('')), 'given back and still watched, its member works at the panel\'s size');
  // A member's step under way is never resized under it: the size waits for its end.
  const step = mcp('browser_evaluate', { function: '() => new Promise((r) => setTimeout(() => r(innerWidth), 2000))' });
  await sleep(500);
  op({ op: 'fit', page, w: 640, h: 720, dpr: 1 });
  await sleep(800);
  check(!frames.slice(-3).some((f) => f.header.page === page && f.header.cssW === 640), 'a new size waits while its member\'s step is under way');
  const during = (await step).content.map((c) => c.text).join('');
  check(/Result\n500/.test(during), 'the step saw the page at one size from start to end');
  const resized = await until('the new size after the step', () => frames.slice(-3).find((f) => f.header.page === page && f.header.cssW === 640 && f.header.cssH === 720));
  check(!!resized, 'and comes once the step is done');
  // Nobody watching any more: its own size again.
  op({ op: 'unfit', page });
  const unfitted = await until('its own size again', () => frames.slice(-3).find((f) => f.header.page === page && f.header.cssW === own?.cssW));
  check(!!unfitted, 'unwatched, the member\'s page is at its own size again');
  await mcp('browser_navigate', { url: `${base}/signin` });
  const signin = await until('the sign-in page', () => lastTabs(lane)?.pages.find((p) => p.url.endsWith('/signin') && p.signals?.password));
  check(signin?.favicon === `${base}/icon.png`, 'a page\'s own icon is told');

  // The person's own tab.
  op({ op: 'newtab', url: `${base}/mine` });
  const mine = await until('the person\'s tab', () => lastTabs('')?.pages.find((p) => p.url.endsWith('/mine')));
  check(!!mine, 'a tab of the person\'s own opens, in no lane');
  op({ op: 'closetab', page: mine.id });
  await until('closed', () => !lastTabs('')?.pages.some((p) => p.id === mine.id));
  check(!lastTabs('')?.pages.some((p) => p.id === mine.id), 'and closes');

  op({ op: 'unwatch', page });
  host.stdin.end();
  await new Promise((r) => host.once('exit', r));
  site.close();
  fs.rmSync(dir, { recursive: true, force: true });
  console.log(failed ? `${failed} check(s) failed` : 'all checks passed');
  process.exit(failed ? 1 : 0);
})().catch((err) => {
  console.error(err);
  process.exit(1);
});
