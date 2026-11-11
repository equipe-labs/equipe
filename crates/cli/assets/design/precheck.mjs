// `equipe design precheck`: the screens a change touches, rendered in the
// Mac's own Chrome, headless, at each condition — width, light or dark, and
// normal size, text at 200% (`text-200`) or the page zoomed to 200%
// (`zoom-200`) — and measured. Nothing is judged here: the rules that read
// these measures live in equipe's core (design/precheck.rs).
//
//   precheck.mjs --plan <plan.json> --json <out.json> [--axe <axe.min.js>] [--jobs 3]
//
// The plan: [{ "key", "condition", "url", "path", "width", "scheme",
// "variant", and how the state is shot — "locale", "js", "reducedMotion",
// "blockFonts", "fullPage", "selector", "permissions", "actions" }, …].
// A normal picture is taken as `equipe design flows shoot` takes the map's
// (render.mjs), so the two compare pixel for pixel. In each page: whether it
// scrolls sideways and what reaches past the window; text left of it; text
// its box cuts off, with what it is (a control's label, a heading, a number,
// its whole text in a title, decoration); text drawn over text; and, with
// --axe, axe-core's WCAG 2.x A/AA rules. Writes one entry per picture.
import { chromium } from 'playwright';
import { dirname, resolve } from 'node:path';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const at = args.indexOf(name);
  if (at < 0) return fallback;
  const value = args[at + 1];
  args.splice(at, 2);
  return value;
};
const planFile = opt('--plan', null);
const json = opt('--json', null);
const axePath = opt('--axe', null);
const jobs = Math.max(1, parseInt(opt('--jobs', '3'), 10) || 1);
if (!planFile || !json) {
  console.error('usage: precheck.mjs --plan <plan.json> --json <out.json> [--axe <axe.min.js>] [--jobs 3]');
  process.exit(2);
}
const axeSource = axePath && existsSync(axePath) ? readFileSync(axePath, 'utf8') : null;
const pageUrl = (u) => (/^(https?|file|data):/.test(u) ? u : pathToFileURL(resolve(u)).href);
const plan = JSON.parse(readFileSync(planFile, 'utf8')).map((s) => ({ settle: 1200, ...s, url: pageUrl(s.url), path: resolve(s.path) }));

// As render.mjs: a phone's screen at a phone's width, a laptop's otherwise.
const height = (width) => (width <= 480 ? 844 : 800);
const TALLEST = 16384;
const SCALE = 2;
const fonts = /\.(woff2?|ttf|otf)(\?.*)?$/;

const scrollThrough = async (page, width) => {
  const h = await page.evaluate(() => document.documentElement.scrollHeight);
  for (let y = 0; y <= h; y += Math.round(height(width) * 0.6)) {
    await page.evaluate((v) => window.scrollTo(0, v), y);
    await page.waitForTimeout(120);
  }
  await page.waitForTimeout(1500);
};

const act = async (page, step) => {
  if ('go' in step) await page.goto(new URL(step.go, page.url()).href, { waitUntil: 'networkidle', timeout: 45000 }).catch(() => page.waitForLoadState('load'));
  else if ('click' in step) await page.click(step.click);
  else if ('fill' in step) await page.fill(step.fill[0], step.fill[1]);
  else if ('press' in step) await page.keyboard.press(step.press);
  else if ('wait' in step) await page.waitForTimeout(step.wait);
};

// Every text twice its size: the root's, and each element's that a fixed
// size kept from growing with it (WCAG 1.4.4's resize text, as a browser's
// text-only zoom does it).
const enlargeText = () => {
  const els = [...document.querySelectorAll('body, body *')];
  const before = els.map((el) => parseFloat(getComputedStyle(el).fontSize) || 0);
  const root = parseFloat(getComputedStyle(document.documentElement).fontSize) || 16;
  document.documentElement.style.setProperty('font-size', `${root * 2}px`, 'important');
  els.forEach((el, i) => {
    const now = parseFloat(getComputedStyle(el).fontSize) || 0;
    if (before[i] > 0 && now < before[i] * 1.9) el.style.setProperty('font-size', `${before[i] * 2}px`, 'important');
  });
};

// Everything measured in the page, in CSS px from the document's top left.
const measure = () => {
  const vw = document.documentElement.clientWidth;
  const sx = window.scrollX;
  const sy = window.scrollY;
  const box = (r) => ({ x: r.left + sx, y: r.top + sy, w: r.width, h: r.height });
  const name = (el) => {
    let s = el.tagName.toLowerCase();
    if (el.id) return `${s}#${el.id}`;
    const cls = [...el.classList].slice(0, 2);
    if (cls.length) s += '.' + cls.join('.');
    return s;
  };
  const shown = (el) => {
    const st = getComputedStyle(el);
    if (st.display === 'none' || st.visibility === 'hidden' || parseFloat(st.opacity) === 0) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  };
  // Visually hidden on purpose (sr-only): a 1px box that clips its text.
  const srOnly = (el) => {
    const r = el.getBoundingClientRect();
    const st = getComputedStyle(el);
    return (r.width <= 2 || r.height <= 2) && (st.overflow === 'hidden' || st.clip !== 'auto' || st.clipPath !== 'none');
  };
  const ownText = (el) => [...el.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent).join(' ').replace(/\s+/g, ' ').trim();
  const all = [...document.body.querySelectorAll('*')].filter((el) => !['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'svg', 'path', 'g'].includes(el.tagName));
  const scrolls = (o) => o === 'auto' || o === 'scroll';
  const contained = (el) => {
    for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
      const o = getComputedStyle(p).overflowX;
      if (scrolls(o) || o === 'hidden' || o === 'clip') return true;
    }
    return false;
  };
  const pinned = (el) => {
    for (let p = el; p && p !== document.body; p = p.parentElement) {
      const pos = getComputedStyle(p).position;
      if (pos === 'fixed' || pos === 'sticky') return true;
    }
    return false;
  };
  const wide = all
    .filter((el) => shown(el) && el.getBoundingClientRect().right > vw + 1 && !contained(el))
    .sort((a, b) => b.getBoundingClientRect().right - a.getBoundingClientRect().right)
    .slice(0, 5)
    .map((el) => ({ selector: name(el), text: (el.innerText || '').trim().slice(0, 60), rect: box(el.getBoundingClientRect()) }));
  const offscreen = all
    .filter((el) => {
      if (!ownText(el) || !shown(el) || srOnly(el) || contained(el) || pinned(el)) return false;
      // Partly cut off at the left edge; wholly off it is a drawer put away.
      const r = el.getBoundingClientRect();
      return r.left < -1 && r.right > 1;
    })
    .slice(0, 5)
    .map((el) => ({ selector: name(el), text: ownText(el).slice(0, 60), rect: box(el.getBoundingClientRect()) }));
  const kinds = (el, text) => {
    const interactive = !!el.closest('a[href], button, label, summary, select, option, [role=button], [role=link], [role=tab], [role=menuitem], [role=option], [role=switch], [role=checkbox], [role=radio]');
    const heading = !!el.closest('h1, h2, h3, h4, h5, h6, [role=heading], th, legend, caption');
    const numeric = /\d/.test(text);
    const full = (v) => !!v && v.replace(/\s+/g, ' ').trim().length >= text.length * 0.8;
    const holder = el.closest('[title], [aria-label]');
    const labelled = full(el.getAttribute('title')) || full(el.getAttribute('aria-label')) || (!!holder && (full(holder.getAttribute('title')) || full(holder.getAttribute('aria-label'))));
    const decorative = !!el.closest('[aria-hidden=true], [role=presentation], [role=none]');
    return { interactive, heading, numeric, labelled, decorative };
  };
  const clipped = [];
  for (const el of all) {
    if (clipped.length >= 40) break;
    const text = (el.innerText || '').replace(/\s+/g, ' ').trim();
    if (!text || !shown(el) || srOnly(el)) continue;
    const st = getComputedStyle(el);
    const hides = (o) => o === 'hidden' || o === 'clip';
    const x = hides(st.overflowX) && el.scrollWidth > el.clientWidth + 1;
    const y = hides(st.overflowY) && el.scrollHeight > el.clientHeight + 1;
    if (!x && !y) continue;
    // Only the innermost box that cuts.
    if (clipped.some((k) => el.contains(k.el))) continue;
    const ellipsis = st.textOverflow === 'ellipsis' || (st.webkitLineClamp && st.webkitLineClamp !== 'none');
    clipped.push({ el, selector: name(el), text: text.slice(0, 80), axis: x ? 'x' : 'y', ellipsis: !!ellipsis, ...kinds(el, text), rect: box(el.getBoundingClientRect()) });
  }
  // Text drawn over text: each text's own lines (its text nodes' boxes), and
  // each control, against the rest that aren't inside one another.
  const pieces = [];
  for (const el of all) {
    if (pieces.length >= 400) break;
    if (!shown(el) || srOnly(el) || pinned(el) || el.closest('[aria-hidden=true]')) continue;
    const text = ownText(el);
    const control = el.matches('a[href], button, input:not([type=hidden]), select, textarea, [role=button]');
    if (!text && !control) continue;
    let r = null;
    if (text) {
      for (const n of el.childNodes) {
        if (n.nodeType !== 3 || !n.textContent.trim()) continue;
        const range = document.createRange();
        range.selectNodeContents(n);
        const b = range.getBoundingClientRect();
        if (b.width <= 0 || b.height <= 0) continue;
        r = r ? { left: Math.min(r.left, b.left), top: Math.min(r.top, b.top), right: Math.max(r.right, b.right), bottom: Math.max(r.bottom, b.bottom) } : { left: b.left, top: b.top, right: b.right, bottom: b.bottom };
      }
    } else {
      const b = el.getBoundingClientRect();
      r = { left: b.left, top: b.top, right: b.right, bottom: b.bottom };
    }
    if (!r || r.right - r.left < 4 || r.bottom - r.top < 4) continue;
    pieces.push({ el, text: text.slice(0, 40), r });
  }
  const overlaps = [];
  for (let i = 0; i < pieces.length && overlaps.length < 20; i++) {
    for (let j = i + 1; j < pieces.length && overlaps.length < 20; j++) {
      const a = pieces[i];
      const b = pieces[j];
      if (a.el.contains(b.el) || b.el.contains(a.el)) continue;
      const w = Math.min(a.r.right, b.r.right) - Math.max(a.r.left, b.r.left);
      const h = Math.min(a.r.bottom, b.r.bottom) - Math.max(a.r.top, b.r.top);
      if (w <= 1 || h <= 1) continue;
      const area = (p) => (p.r.right - p.r.left) * (p.r.bottom - p.r.top);
      const share = (w * h) / Math.min(area(a), area(b));
      if (share < 0.15) continue;
      const left = Math.max(a.r.left, b.r.left);
      const top = Math.max(a.r.top, b.r.top);
      overlaps.push({ a: name(a.el), b: name(b.el), textA: a.text, textB: b.text, bothText: !!(a.text && b.text), share, rect: { x: left + sx, y: top + sy, w, h } });
    }
  }
  return {
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: vw,
    wide,
    offscreen,
    clipped: clipped.map(({ el, ...rest }) => rest),
    overlaps,
  };
};

const runAxe = async (page) => {
  if (!axeSource) return { ran: false, why: 'axe-core is not set up', violations: [] };
  try {
    await page.addScriptTag({ content: axeSource });
    const found = await page.evaluate(async () => {
      const r = await window.axe.run(document, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'] }, resultTypes: ['violations'] });
      return r.violations.map((v) => ({
        id: v.id,
        impact: v.impact || 'minor',
        tags: v.tags,
        help: v.help,
        nodes: v.nodes.slice(0, 10).map((n) => {
          const target = Array.isArray(n.target) ? n.target.join(' ') : String(n.target);
          let rect = null;
          try {
            const el = document.querySelector(Array.isArray(n.target) ? n.target[n.target.length - 1] : target);
            if (el) {
              const b = el.getBoundingClientRect();
              rect = { x: b.left + window.scrollX, y: b.top + window.scrollY, w: b.width, h: b.height };
            }
          } catch {}
          return { target, rect };
        }),
      }));
    });
    return { ran: true, why: null, violations: found };
  } catch (e) {
    return { ran: false, why: String(e.message || e).split('\n')[0], violations: [] };
  }
};

const take = async (browser, shot) => {
  const { url, path, width, scheme, variant, selector } = shot;
  const zoomed = variant === 'zoom-200';
  const normal = variant === 'normal';
  const cssWidth = zoomed ? Math.round(width / 2) : width;
  const locale = shot.locale || 'en-US';
  const js = shot.js !== false;
  const whole = !selector && shot.fullPage !== false;
  mkdirSync(dirname(path), { recursive: true });
  const context = await browser.newContext({
    viewport: { width: cssWidth, height: zoomed ? Math.round(height(width) / 2) : height(width) },
    deviceScaleFactor: SCALE,
    colorScheme: scheme === 'dark' ? 'dark' : 'light',
    locale,
    extraHTTPHeaders: { 'Accept-Language': locale },
    javaScriptEnabled: js,
    reducedMotion: shot.reducedMotion ? 'reduce' : 'no-preference',
  });
  try {
    if (shot.permissions?.length) await context.grantPermissions(shot.permissions, { origin: new URL(url).origin }).catch(() => context.grantPermissions(shot.permissions));
    const page = await context.newPage();
    if (shot.blockFonts) await page.route(fonts, (route) => route.abort());
    await page.goto(url, { waitUntil: 'networkidle', timeout: 45000 }).catch(() => page.waitForLoadState('load'));
    if (!shot.blockFonts) await page.evaluate(() => document.fonts && document.fonts.ready);
    if (shot.settle) await page.waitForTimeout(normal ? shot.settle : Math.min(shot.settle, 800));
    // The normal picture as the map's is taken; the others only need the layout.
    if (normal && (whole || (selector && js))) await scrollThrough(page, width);
    for (const step of shot.actions || []) await act(page, step);
    if (variant === 'text-200') {
      await page.evaluate(enlargeText);
      await page.waitForTimeout(300);
    }
    await page.evaluate(() => window.scrollTo(0, 0));
    await page.waitForTimeout(normal ? 600 : 200);
    const dom = await page.evaluate(measure);
    dom.axe = normal || variant === 'text-200' ? await runAxe(page) : null;
    if (selector) {
      await page.locator(selector).first().screenshot({ path });
    } else if (whole) {
      const tall = (await page.evaluate(() => document.documentElement.scrollHeight)) * SCALE > TALLEST;
      await page.screenshot({ path, fullPage: true, scale: tall ? 'css' : 'device' });
    } else {
      await page.screenshot({ path });
    }
    const tall = whole && (await page.evaluate(() => document.documentElement.scrollHeight)) * SCALE > TALLEST;
    return { key: shot.key, condition: shot.condition, path, dpr: tall ? 1 : SCALE, error: null, dom };
  } finally {
    await context.close();
  }
};

const browser = await chromium.launch({ channel: 'chrome', headless: true });
const results = new Array(plan.length);
try {
  let next = 0;
  const worker = async () => {
    while (next < plan.length) {
      const at = next++;
      const shot = plan[at];
      try {
        results[at] = await take(browser, shot);
        console.log(shot.path);
      } catch (e) {
        results[at] = { key: shot.key, condition: shot.condition, path: null, dpr: SCALE, error: String(e.message || e).split('\n')[0], dom: null };
        console.error(`✗ ${shot.key} ${shot.condition}: ${results[at].error}`);
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(jobs, plan.length) }, worker));
} finally {
  await browser.close();
}
writeFileSync(json, JSON.stringify(results));
