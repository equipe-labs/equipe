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
    args: ['--disable-blink-features=AutomationControlled'],
  });
  const context = await browser.newContext({ viewport: null, ...(fs.existsSync(state) ? { storageState: state } : {}) });
  const save = async () => {
    const kept = await context.storageState();
    const tmp = `${state}.${process.pid}`;
    fs.writeFileSync(tmp, JSON.stringify(kept), { mode: 0o600 });
    fs.renameSync(tmp, state);
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
