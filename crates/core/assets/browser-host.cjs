// equipe's browser host: one Google Chrome, one profile, for every member of
// every squad. The person signs in once — Google too, which ends a sign-in it
// sees copied to another browser — and every member works signed in.
//
// Each member gets a lane: an MCP endpoint of Playwright's own browser tools
// that sees only the windows and tabs that member opened (and the popups they
// opened), so members work side by side without touching each other's pages.
// A member's pages open in a window of its own, behind whatever the person is
// doing; windows go on screen or out of sight (minimized) as the daemon says,
// and keep working either way. The person's own windows belong to no lane.
//
// Run by the daemon under `npx -p playwright` (its package is found on PATH):
// `node browser-host.cjs <profile> <sign-ins file>`, then one JSON request per
// line on stdin:
//   {"op":"lane","id":"squad/name","member":"name","color":"r, g, b","port":1234,"output":"/dir","shown":false}
//   {"op":"show","id":"squad/name","shown":true}
//   {"op":"close","id":"squad/name"}
//   {"op":"person","url":"https://…"}
const path = require('node:path');
const fs = require('node:fs');
const readline = require('node:readline');
const { EventEmitter } = require('node:events');

const roots = (process.env.PATH || '').split(path.delimiter).filter((p) => p.endsWith(path.join('node_modules', '.bin'))).map((p) => path.dirname(p));
const from = (id) => require.resolve(id, { paths: roots });
const { chromium } = require(from('playwright'));
const { tools } = require(path.join(path.dirname(from('playwright-core/package.json')), 'lib', 'coreBundle.js'));

const [profile, STATE] = process.argv.slice(2);

__DRIVING__

// Google's sign-in is this browser's own; the sign-ins file never carries it.
const own = (domain) => /(^|\.)(google\.[a-z]{2,3}(\.[a-z]{2})?|youtube\.com|googleusercontent\.com)$/.test(String(domain).replace(/^\./, ''));

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

  const owner = new Map(); // page -> lane id
  const lanes = new Map(); // lane id -> { events, member, color, shown }
  let claim = null;
  let queue = Promise.resolve();

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

  const windowOf = async (page) => {
    const cdp = await context.newCDPSession(page);
    try {
      const { windowId } = await cdp.send('Browser.getWindowForTarget');
      return { cdp, windowId };
    } catch (err) {
      await cdp.detach().catch(() => {});
      throw err;
    }
  };
  const place = async (page, shown) => {
    const { cdp, windowId } = await windowOf(page);
    try {
      if (shown) {
        await cdp.send('Browser.setWindowBounds', { windowId, bounds: { windowState: 'normal' } });
        await page.bringToFront();
      } else {
        await cdp.send('Browser.setWindowBounds', { windowId, bounds: { windowState: 'minimized' } });
      }
    } finally {
      await cdp.detach().catch(() => {});
    }
  };

  context.on('page', async (page) => {
    let id = claim;
    claim = null;
    if (!id) {
      const opener = await page.opener().catch(() => null);
      id = opener ? owner.get(opener) : undefined;
    }
    if (!id || !lanes.has(id)) return; // the person's own page: no member sees it
    owner.set(page, id);
    page.once('close', () => owner.delete(page));
    lanes.get(id).events.emit('page', page);
  });

  const laneContext = (id) => {
    const lane = lanes.get(id);
    const newPage = () => (queue = queue.then(async () => {
      const any = context.pages()[0] || (await context.newPage());
      const cdp = await context.newCDPSession(any);
      const created = new Promise((resolve) => {
        const check = (p) => {
          if (owner.get(p) === id) {
            context.off('page', check);
            setImmediate(() => resolve(p));
          }
        };
        context.on('page', check);
      });
      claim = id;
      // A window of its own, opened behind whatever the person is doing.
      await cdp.send('Target.createTarget', { url: 'about:blank', newWindow: true, background: true });
      const page = await created;
      await cdp.detach().catch(() => {});
      await place(page, lane.shown).catch(() => {});
      return page;
    }));
    return new Proxy(context, {
      get(target, prop, receiver) {
        if (prop === 'pages') return () => target.pages().filter((p) => owner.get(p) === id);
        if (prop === 'newPage') return newPage;
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
    lanes.set(id, { events: new EventEmitter(), member, color, shown: !!shown });
    const config = await tools.resolveCLIConfigForMCP({ outputDir: output }, process.env);
    const toolset = tools.filteredTools(config);
    const ctx = laneContext(id);
    await tools.start({
      name: 'api',
      nameInConfig: 'browser',
      version: '0',
      toolSchemas: toolset.map((t) => t.schema),
      create: async () => new tools.BrowserBackend(config, ctx, toolset),
    }, { port, host: 'localhost' });
  };
  const show = async ({ id, shown }) => {
    const lane = lanes.get(id);
    if (!lane) return;
    lane.shown = !!shown;
    for (const page of context.pages().filter((p) => owner.get(p) === id)) await place(page, lane.shown).catch(() => {});
  };
  const close = async ({ id }) => {
    for (const page of context.pages().filter((p) => owner.get(p) === id)) await page.close().catch(() => {});
  };
  const person = async ({ url }) => {
    const page = await context.newPage();
    await page.goto(url).catch(() => {});
    await page.bringToFront().catch(() => {});
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

  const ops = { lane: openLane, show, close, person };
  readline.createInterface({ input: process.stdin }).on('line', (line) => {
    let request;
    try { request = JSON.parse(line); } catch { return; }
    Promise.resolve(ops[request.op]?.(request)).catch((err) => console.error(`equipe browser host: ${request.op}: ${err && err.message}`));
  });
  // The daemon gone, the browser goes with it.
  process.stdin.on('end', () => context.close().finally(() => process.exit(0)));
  context.on('close', () => process.exit(0));
  console.error('equipe browser host: ready');
})().catch((err) => {
  console.error(`equipe browser host: ${err && err.message}`);
  process.exit(1);
});
