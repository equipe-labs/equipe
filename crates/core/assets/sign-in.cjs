// equipe browser: the person's sign-in window. Google Chrome, launched as an
// ordinary browser would be — without the automation switch that makes Google
// refuse a sign-in ("This browser or app may not be secure") — starting from the
// squad's sign-ins and keeping what the person signs in to, as they go and when
// the window closes. Run under `npx -p playwright`: its package is found on PATH.
const path = require('node:path');
const fs = require('node:fs');

const [state, url] = process.argv.slice(2);
const roots = (process.env.PATH || '').split(path.delimiter).filter((p) => p.endsWith(path.join('node_modules', '.bin'))).map((p) => path.dirname(p));
const { chromium } = require(require.resolve('playwright', { paths: roots }));

(async () => {
  const browser = await chromium.launch({
    channel: 'chrome',
    headless: process.env.EQUIPE_SIGN_IN_HEADLESS === '1',
    ignoreDefaultArgs: ['--enable-automation'],
    // The second keeps Chrome from warning about the first in a bar.
    args: ['--disable-blink-features=AutomationControlled', '--test-type'],
  });
  const context = await browser.newContext({ viewport: null, ...(fs.existsSync(state) ? { storageState: state } : {}) });
  // Only what the person changed here goes back, merged into the file as it is
  // now: writing all this window holds would put its stale copies over sessions
  // members renewed meanwhile, and sign them out.
  const key = (c) => `${c.name}\u0000${c.domain}\u0000${c.path}`;
  const stamp = (c) => `${c.value}\u0000${c.expires}`;
  const agreed = new Map((await context.cookies()).map((c) => [key(c), stamp(c)]));
  const agreedOrigins = new Map();
  const save = async () => {
    const mine = await context.storageState();
    const changed = mine.cookies.filter((c) => agreed.get(key(c)) !== stamp(c));
    const changedOrigins = mine.origins.filter((o) => agreedOrigins.get(o.origin) !== JSON.stringify(o.localStorage));
    if (!changed.length && !changedOrigins.length) return;
    let kept = { cookies: [], origins: [] };
    try { kept = JSON.parse(fs.readFileSync(state, 'utf8')); } catch {}
    const cookies = new Map(kept.cookies.map((c) => [key(c), c]));
    for (const c of changed) cookies.set(key(c), c);
    const origins = new Map(kept.origins.map((o) => [o.origin, o]));
    for (const o of changedOrigins) origins.set(o.origin, o);
    const tmp = `${state}.${process.pid}`;
    fs.writeFileSync(tmp, JSON.stringify({ cookies: [...cookies.values()], origins: [...origins.values()] }), { mode: 0o600 });
    fs.renameSync(tmp, state);
    for (const c of changed) agreed.set(key(c), stamp(c));
    for (const o of changedOrigins) agreedOrigins.set(o.origin, JSON.stringify(o.localStorage));
  };
  const every = setInterval(() => save().catch(() => {}), 3000);
  const done = async () => {
    clearInterval(every);
    await save().catch(() => {});
    await browser.close().catch(() => {});
    process.exit(0);
  };
  context.on('page', (p) => p.on('close', () => { if (context.pages().length === 0) done(); }));
  browser.on('disconnected', () => process.exit(0));
  const page = await context.newPage();
  page.on('close', () => { if (context.pages().length === 0) done(); });
  await page.goto(url).catch(() => {});
  if (process.env.EQUIPE_SIGN_IN_HEADLESS === '1') {
    console.log(JSON.stringify({ webdriver: await page.evaluate(() => navigator.webdriver) }));
    await done();
  }
})().catch((err) => {
  console.error(String(err && err.message || err));
  process.exit(1);
});
