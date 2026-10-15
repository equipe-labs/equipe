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
fs.writeFileSync(script, fs.readFileSync(path.join(assets, 'browser-host.cjs'), 'utf8').replace('__DRIVING__', fs.readFileSync(path.join(assets, 'driving.js'), 'utf8')));
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
    res.end(`<title>Page ${req.url}</title><h1>${req.headers.host}${req.url}</h1>`);
  }).listen(0);
  await new Promise((r) => site.once('listening', r));
  const base = `http://127.0.0.1:${site.address().port}`;
  const other = `http://localhost:${site.address().port}`;

  const host = spawn(process.execPath, [script, path.join(dir, 'chrome'), path.join(dir, 'state.json')], { stdio: ['pipe', 'pipe', 'inherit'] });
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
  check(busy.isError && /person is using this browser/.test(busy.content[0].text), 'a member\'s tool call waits while the person has the page');
  op({ op: 'control', lane, by: null });
  await until('control given back', () => messages.find((m) => m.type === 'control' && m.lane === lane && m.by === null));
  const typed = await mcp('browser_evaluate', { function: '() => document.getElementById("q").value' });
  const text = typed.content.map((c) => c.text).join('\n');
  check(/한글a/.test(text), `what the person typed reached the page (${/"([^"]*)"/.exec(text)?.[1]})`);
  if (!/한글a/.test(text)) console.log(text);

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
