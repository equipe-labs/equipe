// `equipe design render`: HTML or SVG files drawn to PNG, as a person sees
// them — at each width, in light and (with --dark) dark — through the Mac's
// own Chrome, headless. Prints one line per picture written. With --plan
// <file.json> (`equipe design flows shoot`), the pictures are named in it
// instead: [{ "url", "path", "width", "scheme" }, …].
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
const pageUrl = (file) => (/^https?:/.test(file) ? file : pathToFileURL(resolve(file)).href);
const shots = [];
if (plan) {
  for (const shot of JSON.parse(readFileSync(plan, 'utf8'))) shots.push({ ...shot, url: pageUrl(shot.url), path: resolve(shot.path) });
}
for (const file of files) {
  const stem = basename(file, extname(file)) || 'page';
  for (const width of widths) {
    for (const scheme of dark ? ['light', 'dark'] : ['light']) {
      shots.push({ url: pageUrl(file), path: join(out, `${stem}-${width}${dark ? '-' + scheme : ''}.png`), width, scheme });
    }
  }
}
const browser = await chromium.launch({ channel: 'chrome', headless: true });
try {
  for (const { url, path, width, scheme } of shots) {
    mkdirSync(dirname(path), { recursive: true });
    const context = await browser.newContext({ viewport: { width, height: 900 }, deviceScaleFactor: 2, colorScheme: scheme });
    const page = await context.newPage();
    await page.goto(url, { waitUntil: 'networkidle', timeout: 45000 }).catch(() => page.waitForLoadState('load'));
    await page.evaluate(() => document.fonts && document.fonts.ready);
    await page.screenshot({ path, fullPage: full });
    console.log(path);
    await context.close();
  }
} finally {
  await browser.close();
}
