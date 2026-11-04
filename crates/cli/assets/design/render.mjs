// `equipe design render`: HTML or SVG files drawn to PNG, as a person sees
// them — at each width, in light and (with --dark) dark — through the Mac's
// own Chrome, headless, speaking en-US. Prints one line per picture written.
// With --plan <file.json> (`equipe design flows shoot`), the pictures are
// named in it instead: [{ "url", "path", "width", "scheme", and how the state
// is shot — "locale", "js", "reducedMotion", "blockFonts", "fullPage",
// "selector", "permissions", "actions" }, …] (see the core's design/flows.rs).
import { chromium } from 'playwright';
import { basename, extname, resolve, join, dirname } from 'node:path';
import { mkdirSync, readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const at = args.indexOf(name);
  if (at < 0) return fallback;
  const value = args[at + 1];
  args.splice(at, 2);
  return value;
};
const flag = (name) => {
  const at = args.indexOf(name);
  if (at < 0) return false;
  args.splice(at, 1);
  return true;
};
const widths = opt('--widths', '1280').split(',').map((w) => parseInt(w, 10)).filter((w) => w > 0);
const out = resolve(opt('--out', '.'));
const dark = flag('--dark');
const full = !flag('--viewport');
const plan = opt('--plan', null);
const files = args;
if (!files.length && !plan) {
  console.error('usage: render <file.html|svg>… [--widths 390,1280] [--dark] [--out dir] [--viewport] | render --plan <shots.json>');
  process.exit(2);
}
const pageUrl = (file) => (/^(https?|file|data):/.test(file) ? file : pathToFileURL(resolve(file)).href);
const shots = [];
if (plan) {
  // A state of the map waits for what moves as the page opens.
  for (const shot of JSON.parse(readFileSync(plan, 'utf8'))) shots.push({ settle: 1200, ...shot, url: pageUrl(shot.url), path: resolve(shot.path) });
}
for (const file of files) {
  const stem = basename(file, extname(file)) || 'page';
  for (const width of widths) {
    for (const scheme of dark ? ['light', 'dark'] : ['light']) {
      shots.push({ url: pageUrl(file), path: join(out, `${stem}-${width}${dark ? '-' + scheme : ''}.png`), width, scheme, fullPage: full });
    }
  }
}
// A phone's screen at a phone's width, a laptop's otherwise.
const height = (width) => (width <= 480 ? 844 : 800);
// Chrome draws no taller than this, in device pixels: a longer page is shot at 1×.
const TALLEST = 16384;
const SCALE = 2;
const fonts = /\.(woff2?|ttf|otf)(\?.*)?$/;

// Through the whole page, so what appears as it scrolls has, then a moment
// for what counts up.
const scrollThrough = async (page, width) => {
  const h = await page.evaluate(() => document.documentElement.scrollHeight);
  for (let y = 0; y <= h; y += Math.round(height(width) * 0.6)) {
    await page.evaluate((v) => window.scrollTo(0, v), y);
    await page.waitForTimeout(120);
  }
  await page.waitForTimeout(1500);
};

// Headers stuck to the top of the window go back into the page's flow, so
// they don't sit over the element shot.
const unstick = (page, selector) =>
  page.evaluate((selector) => {
    const target = document.querySelector(selector);
    if (!target) return;
    for (const el of document.querySelectorAll('body *')) {
      if (el.contains(target) || target.contains(el)) continue;
      const style = getComputedStyle(el);
      const header = style.position === 'sticky' || (style.position === 'fixed' && el.getBoundingClientRect().height < window.innerHeight * 0.4);
      if (header && style.top === '0px') el.style.setProperty('position', 'static', 'important');
    }
  }, selector);

const act = async (page, step) => {
  if ('go' in step) await page.goto(new URL(step.go, page.url()).href, { waitUntil: 'networkidle', timeout: 45000 }).catch(() => page.waitForLoadState('load'));
  else if ('click' in step) await page.click(step.click);
  else if ('fill' in step) await page.fill(step.fill[0], step.fill[1]);
  else if ('press' in step) await page.keyboard.press(step.press);
  else if ('wait' in step) await page.waitForTimeout(step.wait);
};

const browser = await chromium.launch({ channel: 'chrome', headless: true });
try {
  for (const shot of shots) {
    const { url, path, width, scheme, selector } = shot;
    const locale = shot.locale || 'en-US';
    const js = shot.js !== false;
    const whole = !selector && shot.fullPage !== false;
    mkdirSync(dirname(path), { recursive: true });
    const context = await browser.newContext({
      viewport: { width, height: height(width) },
      deviceScaleFactor: SCALE,
      colorScheme: scheme,
      locale,
      extraHTTPHeaders: { 'Accept-Language': locale },
      javaScriptEnabled: js,
      reducedMotion: shot.reducedMotion ? 'reduce' : 'no-preference',
    });
    if (shot.permissions?.length) await context.grantPermissions(shot.permissions, { origin: new URL(url).origin }).catch(() => context.grantPermissions(shot.permissions));
    const page = await context.newPage();
    if (shot.blockFonts) await page.route(fonts, (route) => route.abort());
    await page.goto(url, { waitUntil: 'networkidle', timeout: 45000 }).catch(() => page.waitForLoadState('load'));
    if (!shot.blockFonts) await page.evaluate(() => document.fonts && document.fonts.ready);
    if (shot.settle) await page.waitForTimeout(shot.settle);
    if (plan && (whole || (selector && js))) await scrollThrough(page, width);
    if (selector) {
      await unstick(page, selector);
      await page.locator(selector).first().scrollIntoViewIfNeeded();
      await page.waitForTimeout(500);
    }
    for (const step of shot.actions || []) await act(page, step);
    if (selector) {
      await page.locator(selector).first().screenshot({ path });
    } else if (whole) {
      if (plan) {
        await page.evaluate(() => window.scrollTo(0, 0));
        await page.waitForTimeout(600);
      }
      const tall = (await page.evaluate(() => document.documentElement.scrollHeight)) * SCALE > TALLEST;
      await page.screenshot({ path, fullPage: true, scale: tall ? 'css' : 'device' });
    } else {
      await page.screenshot({ path });
    }
    console.log(path);
    await context.close();
  }
} finally {
  await browser.close();
}
