// `equipe design refs | palette | type`: pages drawn to PNG the way a person
// opens them, through the Mac's own Chrome, headless, saying so in its user
// agent (nothing hides that it is automated). Reads a plan (--plan <file>):
// { "pace": seconds between two pages of one site, "shots": [{ "url",
// "path", "width", "height", "scheme", "full", "maxHeight", "capture",
// "inspect", "css" }] } — with "inspect", the colour variables the page
// declares come back (in "vars"); with "css", that stylesheet is laid over
// the page first (a palette tried on the page being designed);
// and writes what happened to --result <file>: for each shot its status,
// title and final address, and — with "capture" — the stylesheets and font
// files the page loaded from Google Fonts (their text, and their sizes), for
// a type specimen's coverage and weight. A page answering with a check for
// a person is reported as it is and never answered: equipe decides what next.
import { chromium } from 'playwright';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const args = process.argv.slice(2);
const opt = (name) => {
  const at = args.indexOf(name);
  return at < 0 ? null : args[at + 1];
};
const planFile = opt('--plan');
const resultFile = opt('--result');
if (!planFile || !resultFile) {
  console.error('usage: refs --plan <plan.json> --result <result.json>');
  process.exit(2);
}
const plan = JSON.parse(readFileSync(planFile, 'utf8'));
const pace = Math.max(0, Number(plan.pace || 0)) * 1000;
const pageUrl = (u) => (/^(https?|file|data):/.test(u) ? u : pathToFileURL(resolve(u)).href);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const lastAt = new Map();
// Sites that answered with a check or a refusal: none of their other pages is opened.
const stopped = new Set();
const wall = (status, title) => [401, 403, 429, 503].includes(status) || /just a moment|attention required|verify you are human|checking your browser|access denied/i.test(title);
const results = [];

const browser = await chromium.launch({ channel: 'chrome', headless: true });
try {
  for (const shot of plan.shots) {
    const url = pageUrl(shot.url);
    const remote = /^https?:/.test(url);
    const host = remote ? new URL(url).host : '';
    if (stopped.has(host)) {
      results.push({ url: shot.url, path: shot.path, ok: false, status: 0, title: '', finalUrl: '', error: 'the site asked for a check earlier: not opened', css: [], fonts: [] });
      continue;
    }
    // A pause between two pages of one site, as a person reading them would take.
    if (remote && lastAt.has(host)) {
      const wait = lastAt.get(host) + pace - Date.now();
      if (wait > 0) await sleep(wait);
    }
    const out = { url: shot.url, path: shot.path, ok: false, status: 0, title: '', finalUrl: '', error: '', css: [], fonts: [], vars: [] };
    const context = await browser.newContext({
      viewport: { width: shot.width || 1280, height: shot.height || 800 },
      deviceScaleFactor: 1,
      colorScheme: shot.scheme === 'dark' ? 'dark' : 'light',
      locale: 'en-US',
    });
    const page = await context.newPage();
    const pending = [];
    if (shot.capture) {
      page.on('response', (response) => {
        const u = response.url();
        if (u.startsWith('https://fonts.googleapis.com/')) pending.push(response.text().then((text) => out.css.push(text)).catch(() => {}));
        else if (u.startsWith('https://fonts.gstatic.com/')) pending.push(response.body().then((b) => out.fonts.push({ url: u, bytes: b.length })).catch(() => {}));
      });
    }
    try {
      const response = await page.goto(url, { waitUntil: 'load', timeout: 45000 });
      out.status = response ? response.status() : 0;
      await page.waitForLoadState('networkidle', { timeout: 8000 }).catch(() => {});
      // A palette tried on the page itself: its colour variables, then the palette's over them.
      if (shot.inspect) {
        out.vars = await page.evaluate(() => {
          const names = new Set();
          for (const sheet of document.styleSheets) {
            let rules;
            try { rules = sheet.cssRules; } catch { continue; }
            const walk = (list) => {
              for (const r of list) {
                if (r.style) for (const p of r.style) if (p.startsWith('--')) names.add(p);
                if (r.cssRules) walk(r.cssRules);
              }
            };
            walk(rules);
          }
          const cs = getComputedStyle(document.documentElement);
          const probe = document.createElement('i');
          document.body.appendChild(probe);
          const colour = (v) => { probe.style.color = ''; probe.style.color = v.trim(); return probe.style.color !== ''; };
          const out = [...names].filter((n) => { const v = cs.getPropertyValue(n); return v && colour(v); });
          probe.remove();
          return out;
        });
      }
      if (shot.css) await page.addStyleTag({ content: shot.css });
      await page.evaluate(() => document.fonts && document.fonts.ready).catch(() => {});
      await page.waitForTimeout(shot.capture ? 600 : 1200);
      out.title = await page.title();
      out.finalUrl = page.url();
      mkdirSync(dirname(resolve(shot.path)), { recursive: true });
      if (shot.full) {
        const height = await page.evaluate(() => document.documentElement.scrollHeight);
        const clipHeight = Math.min(height, shot.maxHeight || 6000);
        await page.setViewportSize({ width: shot.width || 1280, height: clipHeight });
        await page.waitForTimeout(400);
        await page.screenshot({ path: resolve(shot.path), clip: { x: 0, y: 0, width: shot.width || 1280, height: clipHeight } });
      } else {
        await page.screenshot({ path: resolve(shot.path) });
      }
      await Promise.all(pending);
      out.ok = true;
    } catch (e) {
      out.error = String(e && e.message ? e.message : e).split('\n')[0];
    }
    if (remote) lastAt.set(host, Date.now());
    if (remote && wall(out.status, out.title)) stopped.add(host);
    await context.close();
    results.push(out);
    console.error(`${out.ok ? 'shot' : 'missed'} ${shot.url}${out.status ? ` (${out.status})` : ''}`);
  }
} finally {
  await browser.close();
  writeFileSync(resultFile, JSON.stringify(results));
}
