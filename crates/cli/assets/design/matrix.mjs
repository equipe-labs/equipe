// `equipe design matrix`: a component's docs page drawn in the Mac's own
// Chrome, headless, and measured — nothing judged here. The judging lives
// in equipe's core (design/system/matrix.rs).
//
//   matrix.mjs --plan <plan.json> --out <dir> --json <file> [--axe <axe.min.js>]
//
// For each theme and width: every example on the page (`[data-example]`)
// put in each state the plan names (hover, focus by the keyboard, pressed,
// disabled, invalid, checked, open) and pictured; the look and the text
// colours of each state; axe-core's WCAG rules over the examples; and, at
// the first width, a keyboard pass (Tab reaches it, focus shows, Enter and
// Space act, Esc closes and focus returns, focus stays inside a modal, the
// arrow keys move). Then a contact sheet per width: examples by states,
// light and dark.
import { chromium } from 'playwright';
import { resolve, join } from 'node:path';
import { mkdirSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const at = args.indexOf(name);
  return at < 0 ? fallback : args[at + 1];
};
const planFile = opt('--plan', null);
const out = resolve(opt('--out', '.'));
const json = opt('--json', null);
const axePath = opt('--axe', null);
if (!planFile || !json) {
  console.error('usage: matrix.mjs --plan <plan.json> --out <dir> --json <file> [--axe <axe.min.js>]');
  process.exit(2);
}
const plan = JSON.parse(readFileSync(planFile, 'utf8'));
const url = /^(https?|file|data):/.test(plan.url) ? plan.url : pathToFileURL(resolve(plan.url)).href;
const heightFor = (w) => (w <= 480 ? 844 : w <= 800 ? 1024 : w <= 1100 ? 768 : w <= 1300 ? 800 : w <= 1600 ? 900 : 1080);
const axeSource = axePath && existsSync(axePath) ? readFileSync(axePath, 'utf8') : null;
const pause = (ms) => new Promise((r) => setTimeout(r, ms));

// In the page: helpers each evaluation can reach.
const helpers = () => {
  if (window.__eq) return;
  const cv = document.createElement('canvas');
  cv.width = cv.height = 1;
  const cx = cv.getContext('2d', { willReadFrequently: true });
  const rgba = (c) => {
    cx.clearRect(0, 0, 1, 1);
    cx.fillStyle = '#000';
    cx.fillStyle = c;
    cx.fillRect(0, 0, 1, 1);
    const d = cx.getImageData(0, 0, 1, 1).data;
    return { r: d[0], g: d[1], b: d[2], a: d[3] / 255 };
  };
  const shown = (el) => {
    if (!el) return false;
    const st = getComputedStyle(el);
    if (st.display === 'none' || st.visibility === 'hidden' || parseFloat(st.opacity) === 0) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  };
  const backdrop = (el) => {
    const layers = [];
    for (let p = el; p; p = p.parentElement) {
      const st = getComputedStyle(p);
      if (st.backgroundImage && st.backgroundImage !== 'none' && !st.backgroundImage.startsWith('linear-gradient(transparent')) return null;
      const c = rgba(st.backgroundColor);
      if (c.a > 0) {
        layers.push(c);
        if (c.a >= 1) break;
      }
    }
    let bg = rgba(getComputedStyle(document.documentElement).backgroundColor || '#fff');
    if (bg.a < 1) bg = { r: 255, g: 255, b: 255, a: 1 };
    for (const c of layers.reverse()) bg = { r: c.r * c.a + bg.r * (1 - c.a), g: c.g * c.a + bg.g * (1 - c.a), b: c.b * c.a + bg.b * (1 - c.a), a: 1 };
    return `rgb(${Math.round(bg.r)}, ${Math.round(bg.g)}, ${Math.round(bg.b)})`;
  };
  const look = (el) => {
    if (!el) return '';
    const st = getComputedStyle(el);
    return [st.outlineStyle, st.outlineWidth, st.outlineColor, st.boxShadow, st.borderColor, st.backgroundColor, st.color, st.textDecorationLine, st.opacity, st.transform].join('|');
  };
  const texts = (roots) => {
    const found = [];
    for (const root of roots) {
      if (!root) continue;
      for (const el of [root, ...root.querySelectorAll('*')]) {
        if (found.length >= 24) break;
        const own = [...el.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent).join(' ').replace(/\s+/g, ' ').trim();
        if (!own || !shown(el)) continue;
        const st = getComputedStyle(el);
        const c = rgba(st.color);
        found.push({ text: own.slice(0, 40), fg: `rgba(${c.r}, ${c.g}, ${c.b}, ${c.a.toFixed(3)})`, bg: backdrop(el), size: parseFloat(st.fontSize), weight: parseInt(st.fontWeight, 10) || 400 });
      }
    }
    return found;
  };
  const examples = () => [...document.querySelectorAll('[data-example]')];
  const target = (ex, selector) => {
    if (!ex) return null;
    const marked = ex.matches('[data-target]') ? ex : ex.querySelector('[data-target]');
    if (marked) return marked;
    if (!selector) return null;
    if (ex.matches(selector)) return ex;
    return ex.querySelector(selector);
  };
  const surface = (selector) => (selector ? [...document.querySelectorAll(selector)].find(shown) || null : null);
  const state = (el, sel) => {
    if (!el) return '';
    const s = [el.getAttribute('aria-expanded'), el.getAttribute('aria-checked'), el.getAttribute('aria-pressed'), el.getAttribute('aria-selected'), el.getAttribute('data-state'), el.getAttribute('aria-valuenow'), el.checked, el.value, el.__eqClicks || 0, !!surface(sel)];
    return JSON.stringify(s);
  };
  window.__eq = { rgba, shown, backdrop, look, texts, examples, target, surface, state };
};

const theme = async (page, name) => {
  await page.evaluate((t) => {
    const html = document.documentElement;
    html.classList.toggle('dark', t === 'dark');
    html.classList.toggle('light', t === 'light');
    html.setAttribute('data-theme', t);
    html.style.colorScheme = t;
  }, name);
};

const openPage = async (browser, width, themeName) => {
  const context = await browser.newContext({ viewport: { width, height: heightFor(width) }, deviceScaleFactor: 1, locale: 'en-US', colorScheme: themeName === 'dark' ? 'dark' : 'light', reducedMotion: 'reduce' });
  const page = await context.newPage();
  await page.goto(url, { waitUntil: 'networkidle', timeout: 45000 }).catch(() => page.waitForLoadState('load'));
  await theme(page, themeName);
  await page.evaluate(helpers);
  await page.evaluate(() => document.fonts && document.fonts.ready);
  await pause(300);
  return { context, page };
};

const reset = async (page) => {
  await page.keyboard.press('Escape').catch(() => {});
  await page.mouse.move(0, 0);
  await page.evaluate(() => {
    if (document.activeElement && document.activeElement !== document.body) document.activeElement.blur();
    for (const el of document.querySelectorAll('[data-equipe-forced]')) {
      const was = JSON.parse(el.getAttribute('data-equipe-forced'));
      for (const [k, v] of Object.entries(was)) {
        if (k === 'disabled') el.disabled = v;
        else if (v === null) el.removeAttribute(k);
        else el.setAttribute(k, v);
      }
      el.removeAttribute('data-equipe-forced');
    }
  });
  await pause(120);
};

// Put the i-th example's part in `state`; whether it could be, and why not.
const apply = async (page, i, stateName) => {
  const box = await page.evaluate(({ i, sel }) => {
    const ex = window.__eq.examples()[i];
    const t = window.__eq.target(ex, sel) || ex;
    t.scrollIntoView({ block: 'center', inline: 'center' });
    const r = t.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
  }, { i, sel: plan.target });
  const force = (attrs) =>
    page.evaluate(({ i, sel, attrs }) => {
      const ex = window.__eq.examples()[i];
      const t = window.__eq.target(ex, sel) || ex;
      const was = {};
      for (const [k, v] of Object.entries(attrs)) {
        if (k === 'disabled') {
          if (!('disabled' in t)) continue;
          was.disabled = t.disabled;
          t.disabled = true;
        } else {
          was[k] = t.getAttribute(k);
          t.setAttribute(k, v);
        }
      }
      t.setAttribute('data-equipe-forced', JSON.stringify(was));
      return Object.keys(was).length > 0;
    }, { i, sel: plan.target, attrs });
  switch (stateName) {
    case 'default':
      return { applied: true };
    case 'hover':
      await page.mouse.move(box.x, box.y);
      return { applied: true };
    case 'focus-visible':
      await page.keyboard.press('Shift');
      await page.evaluate(({ i, sel }) => (window.__eq.target(window.__eq.examples()[i], sel) || window.__eq.examples()[i]).focus({ focusVisible: true }), { i, sel: plan.target });
      return { applied: await page.evaluate(({ i, sel }) => { const t = window.__eq.target(window.__eq.examples()[i], sel); return !!t && (t === document.activeElement || t.contains(document.activeElement)); }, { i, sel: plan.target }), why: 'it takes no focus (not a control, or tabindex missing)' };
    case 'active':
      await page.mouse.move(box.x, box.y);
      await page.mouse.down();
      return { applied: true, release: true };
    case 'disabled':
      return { applied: await force({ disabled: true, 'aria-disabled': 'true', 'data-disabled': '' }), why: 'its part has no disabled state to set' };
    case 'error':
      return { applied: await force({ 'aria-invalid': 'true' }) };
    case 'checked': {
      const before = await page.evaluate(({ i, sel }) => window.__eq.state(window.__eq.target(window.__eq.examples()[i], sel)), { i, sel: plan.target });
      await page.mouse.click(box.x, box.y);
      await pause(200);
      const after = await page.evaluate(({ i, sel }) => window.__eq.state(window.__eq.target(window.__eq.examples()[i], sel)), { i, sel: plan.target });
      return { applied: before !== after, why: 'clicking it changed nothing (aria-checked, aria-pressed, aria-selected, data-state)' };
    }
    case 'open': {
      const how = plan.open ? plan.open.how : 'click';
      if (how === 'hover') await page.mouse.move(box.x, box.y);
      else if (how === 'focus') {
        await page.keyboard.press('Shift');
        await page.evaluate(({ i, sel }) => (window.__eq.target(window.__eq.examples()[i], sel) || window.__eq.examples()[i]).focus(), { i, sel: plan.target });
      } else if (how === 'contextmenu') await page.mouse.click(box.x, box.y, { button: 'right' });
      else await page.mouse.click(box.x, box.y);
      await pause(700);
      const open = await page.evaluate((s) => !!window.__eq.surface(s), plan.open ? plan.open.surface : null);
      return { applied: open || !plan.open, why: `nothing matching ${plan.open ? plan.open.surface : ''} appeared` };
    }
    default:
      return { applied: false, why: `equipe doesn't know the state ${stateName}` };
  }
};

const picture = async (page, i, file, withSurface) => {
  const clip = await page.evaluate(({ i, s }) => {
    const rects = [window.__eq.examples()[i].getBoundingClientRect()];
    const surf = s && window.__eq.surface(s);
    if (surf) rects.push(surf.getBoundingClientRect());
    const x = Math.max(0, Math.min(...rects.map((r) => r.left)) - 12);
    const y = Math.max(0, Math.min(...rects.map((r) => r.top)) - 12);
    const right = Math.min(window.innerWidth, Math.max(...rects.map((r) => r.right)) + 12);
    const bottom = Math.min(window.innerHeight, Math.max(...rects.map((r) => r.bottom)) + 12);
    return { x, y, width: Math.max(1, right - x), height: Math.max(1, bottom - y) };
  }, { i, s: withSurface && plan.open ? plan.open.surface : null });
  await page.screenshot({ path: file, clip });
};

// ── the keyboard pass ──

const keyboard = async (page, i, check) => {
  const sel = plan.target;
  const surfaceSel = plan.open ? plan.open.surface : null;
  const reach = async () => {
    await page.evaluate(({ i }) => {
      const ex = window.__eq.examples()[i];
      let start = document.querySelector('[data-equipe-start]');
      if (!start) {
        start = document.createElement('button');
        start.setAttribute('data-equipe-start', '');
        start.setAttribute('aria-hidden', 'true');
        start.style.cssText = 'position:absolute;width:1px;height:1px;opacity:0;';
      }
      ex.parentNode.insertBefore(start, ex);
      start.focus();
    }, { i });
    for (let n = 0; n < 16; n++) {
      await page.keyboard.press('Tab');
      const there = await page.evaluate(({ i, sel }) => {
        const ex = window.__eq.examples()[i];
        const t = window.__eq.target(ex, sel);
        const a = document.activeElement;
        return !!t && !!a && (a === t || t.contains(a) || (ex.contains(a) && a.matches(sel || '*')));
      }, { i, sel });
      if (there) return n + 1;
    }
    return 0;
  };
  const snap = () => page.evaluate(({ i, sel, s }) => {
    const t = window.__eq.target(window.__eq.examples()[i], sel);
    if (t && !t.__eqCounted) {
      t.__eqCounted = true;
      t.addEventListener('click', () => { t.__eqClicks = (t.__eqClicks || 0) + 1; });
    }
    return window.__eq.state(t, s);
  }, { i, sel, s: surfaceSel });
  const inSurface = () => page.evaluate((s) => { const surf = window.__eq.surface(s); return !!surf && surf.contains(document.activeElement); }, surfaceSel);
  const surfaceOpen = () => page.evaluate((s) => !!window.__eq.surface(s), surfaceSel);
  const openByKeys = async () => {
    if (!(await reach())) return false;
    if (plan.open && plan.open.how === 'focus') return surfaceOpen();
    await page.keyboard.press('Enter');
    await pause(500);
    return surfaceOpen();
  };
  const active = () => page.evaluate(() => { const a = document.activeElement; return a ? (a.id || '') + '|' + a.tagName + '|' + (a.textContent || '').trim().slice(0, 30) + '|' + [...(a.parentElement ? a.parentElement.children : [])].indexOf(a) : ''; });
  const value = () => page.evaluate(() => { const a = document.activeElement; return a ? JSON.stringify([a.getAttribute('aria-valuenow'), a.value, a.getAttribute('aria-checked'), a.getAttribute('aria-valuemax'), a.getAttribute('aria-valuemin')]) : ''; });
  await reset(page);
  switch (check) {
    case 'tab-reaches': {
      const n = await reach();
      return { pass: n > 0, detail: n ? `reached in ${n} Tab(s)` : '16 Tabs never reached it (tabindex, or not a control)' };
    }
    case 'focus-visible': {
      const before = await page.evaluate(({ i, sel }) => window.__eq.look(window.__eq.target(window.__eq.examples()[i], sel)), { i, sel });
      if (!(await reach())) return { pass: false, detail: 'Tab never reached it' };
      const now = await page.evaluate(() => { const a = document.activeElement; const st = getComputedStyle(a); return { look: window.__eq.look(a), outline: st.outlineStyle !== 'none' && parseFloat(st.outlineWidth) > 0 && window.__eq.rgba(st.outlineColor).a > 0 }; });
      return { pass: now.outline || now.look !== before, detail: 'no outline, ring or change of look' };
    }
    case 'enter-activates':
    case 'space-activates': {
      if (!(await reach())) return { pass: false, detail: 'Tab never reached it' };
      const before = await snap();
      await page.keyboard.press(check === 'enter-activates' ? 'Enter' : ' ');
      await pause(400);
      const after = await snap();
      return { pass: before !== after, detail: 'nothing happened: no click, nothing opened or changed (aria-expanded, aria-checked, aria-pressed, data-state)' };
    }
    case 'focus-moves-in': {
      if (!(await openByKeys())) return { pass: false, detail: 'Enter didn\'t open it' };
      return { pass: await inSurface(), detail: 'focus stayed outside what opened' };
    }
    case 'focus-trapped': {
      if (!(await openByKeys())) return { pass: false, detail: 'Enter didn\'t open it' };
      for (let n = 0; n < 12; n++) {
        await page.keyboard.press('Tab');
        if (!(await inSurface())) return { pass: false, detail: `the ${n + 1}th Tab left it` };
      }
      return { pass: true, detail: '' };
    }
    case 'esc-closes': {
      let open;
      if (plan.open && plan.open.how === 'contextmenu') {
        const box = await page.evaluate(({ i, sel }) => { const t = window.__eq.target(window.__eq.examples()[i], sel) || window.__eq.examples()[i]; const r = t.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; }, { i, sel });
        await page.mouse.click(box.x, box.y, { button: 'right' });
        await pause(400);
        open = await surfaceOpen();
      } else open = await openByKeys();
      if (!open) return { pass: false, detail: 'it couldn\'t be opened to try' };
      await page.keyboard.press('Escape');
      await pause(500);
      return { pass: !(await surfaceOpen()), detail: 'still open after Esc' };
    }
    case 'focus-returns': {
      if (!(await openByKeys())) return { pass: false, detail: 'Enter didn\'t open it' };
      await page.keyboard.press('Escape');
      await pause(500);
      const back = await page.evaluate(({ i, sel }) => { const t = window.__eq.target(window.__eq.examples()[i], sel); const a = document.activeElement; return !!t && (a === t || t.contains(a)); }, { i, sel });
      return { pass: back, detail: 'focus went elsewhere after it closed' };
    }
    case 'arrows-move': {
      if (plan.open && plan.open.how === 'contextmenu') {
        const box = await page.evaluate(({ i, sel }) => { const t = window.__eq.target(window.__eq.examples()[i], sel) || window.__eq.examples()[i]; const r = t.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; }, { i, sel });
        await page.mouse.click(box.x, box.y, { button: 'right' });
        await pause(400);
        await page.keyboard.press('ArrowDown');
      } else if (plan.open && plan.open.how === 'click' && !['menubar'].includes(plan.component)) {
        if (!(await openByKeys())) return { pass: false, detail: 'Enter didn\'t open it' };
      } else if (!(await reach())) return { pass: false, detail: 'Tab never reached it' };
      const before = await active();
      for (const key of ['ArrowDown', 'ArrowRight']) {
        await page.keyboard.press(key);
        await pause(200);
        const now = await active();
        if (now !== before && now !== '') return { pass: true, detail: '' };
      }
      return { pass: false, detail: 'ArrowDown and ArrowRight left focus where it was' };
    }
    case 'arrows-change': {
      if (!(await reach())) return { pass: false, detail: 'Tab never reached it' };
      const before = await value();
      for (const key of ['ArrowRight', 'ArrowUp']) {
        await page.keyboard.press(key);
        await pause(150);
        if ((await value()) !== before) return { pass: true, detail: '' };
      }
      return { pass: false, detail: 'ArrowRight and ArrowUp changed nothing' };
    }
    case 'home-end': {
      if (!(await reach())) return { pass: false, detail: 'Tab never reached it' };
      const start = [await active(), await value()].join();
      await page.keyboard.press('End');
      await pause(150);
      const end = [await active(), await value()].join();
      await page.keyboard.press('Home');
      await pause(150);
      const home = [await active(), await value()].join();
      return { pass: end !== home && (end !== start || home !== start), detail: 'Home and End went nowhere' };
    }
    case 'disabled-skipped': {
      await page.evaluate(({ i, sel }) => { const t = window.__eq.target(window.__eq.examples()[i], sel); if (t && 'disabled' in t) { t.setAttribute('data-equipe-forced', JSON.stringify({ disabled: t.disabled })); t.disabled = true; } }, { i, sel });
      const n = await reach();
      return { pass: n === 0, detail: 'Tab still lands on it when it is disabled' };
    }
    default:
      return { pass: true, detail: '' };
  }
};

const browser = await chromium.launch({ channel: 'chrome', headless: true });
const result = { url, runs: [], sheets: [] };
try {
  mkdirSync(out, { recursive: true });
  const cells = {};
  for (const width of plan.widths) {
    for (const themeName of plan.themes) {
      const { context, page } = await openPage(browser, width, themeName);
      const ids = await page.evaluate(() => window.__eq.examples().map((e, n) => e.getAttribute('data-example') || `example ${n + 1}`));
      const run = { width, theme: themeName, examples: [], axe: { ran: false, why: null, violations: [] }, keyboard: [] };
      for (let i = 0; i < ids.length; i++) {
        const shot = { example: ids[i], target: await page.evaluate(({ i, sel }) => !!window.__eq.target(window.__eq.examples()[i], sel), { i, sel: plan.target }), states: [] };
        const restLook = await page.evaluate(({ i, sel }) => window.__eq.look(window.__eq.target(window.__eq.examples()[i], sel)), { i, sel: plan.target });
        for (const stateName of plan.states) {
          await reset(page);
          const done = await apply(page, i, stateName).catch((e) => ({ applied: false, why: String(e.message || e).split('\n')[0] }));
          const s = { state: stateName, applied: !!done.applied, why: done.applied ? null : done.why || null, picture: null, look: '', texts: [], focus_shows: null };
          if (done.applied) {
            await pause(stateName === 'open' ? 50 : 180);
            const m = await page.evaluate(({ i, sel, surf }) => {
              const ex = window.__eq.examples()[i];
              const t = window.__eq.target(ex, sel);
              const a = document.activeElement;
              const st = a ? getComputedStyle(a) : null;
              return {
                look: window.__eq.look(t),
                texts: window.__eq.texts([ex, surf ? window.__eq.surface(surf) : null]),
                outline: !!st && st.outlineStyle !== 'none' && parseFloat(st.outlineWidth) > 0 && window.__eq.rgba(st.outlineColor).a > 0,
              };
            }, { i, sel: plan.target, surf: stateName === 'open' && plan.open ? plan.open.surface : null });
            s.look = m.look;
            s.texts = m.texts;
            if (stateName === 'focus-visible') s.focus_shows = m.outline || m.look !== restLook;
            const dir = join(out, String(width), themeName);
            mkdirSync(dir, { recursive: true });
            s.picture = join(dir, `${i + 1}-${stateName}.png`);
            await picture(page, i, s.picture, stateName === 'open');
            (cells[width] ||= []).push({ example: ids[i], theme: themeName, state: stateName, picture: s.picture });
          }
          if (done.release) await page.mouse.up();
          shot.states.push(s);
        }
        run.examples.push(shot);
      }
      await reset(page);
      if (axeSource && ids.length) {
        try {
          await page.addScriptTag({ content: axeSource });
          const found = await page.evaluate(async () => {
            const r = await window.axe.run({ include: [['[data-example]']] }, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'] }, resultTypes: ['violations'] });
            return r.violations.map((v) => ({ id: v.id, impact: v.impact, help: v.help, helpUrl: v.helpUrl, nodes: v.nodes.slice(0, 5).map((n) => ({ target: n.target.map(String), failureSummary: n.failureSummary })) }));
          });
          run.axe = { ran: true, why: null, violations: found };
        } catch (e) {
          run.axe = { ran: false, why: String(e.message || e).split('\n')[0], violations: [] };
        }
      } else if (!axeSource) {
        run.axe = { ran: false, why: 'axe-core isn\'t installed in the design kit (it is set up with the first matrix run that has a network)', violations: [] };
      }
      if (width === plan.widths[0]) {
        for (let i = 0; i < ids.length; i++) {
          if (!(await page.evaluate(({ i, sel }) => !!window.__eq.target(window.__eq.examples()[i], sel), { i, sel: plan.target }))) continue;
          for (const check of plan.keyboard) {
            const r = await keyboard(page, i, check).catch((e) => ({ pass: false, detail: String(e.message || e).split('\n')[0] }));
            run.keyboard.push({ example: ids[i], check, pass: !!r.pass, detail: r.pass ? '' : r.detail });
          }
        }
      }
      result.runs.push(run);
      await context.close();
    }
    // The contact sheet: each example's states in a row, light then dark.
    const list = cells[width] || [];
    if (list.length) {
      const rows = [];
      for (const ex of [...new Set(list.map((c) => c.example))]) {
        for (const themeName of plan.themes) {
          const cellsOf = plan.states.map((st) => list.find((c) => c.example === ex && c.theme === themeName && c.state === st));
          rows.push(`<tr><th>${ex.replace(/</g, '&lt;')}<br><small>${themeName}</small></th>${cellsOf.map((c) => `<td class="${themeName}">${c ? `<img src="data:image/png;base64,${readFileSync(c.picture).toString('base64')}">` : '<span>—</span>'}</td>`).join('')}</tr>`);
        }
      }
      const html = `<!doctype html><meta charset="utf-8"><style>body{font:13px system-ui,sans-serif;margin:16px;background:#fff;color:#111}table{border-collapse:collapse}th,td{border:1px solid #ddd;padding:8px;vertical-align:middle;text-align:left}td.dark{background:#111}img{max-width:320px;display:block}h1{font-size:16px}</style><h1>${plan.component} — ${width}px</h1><table><tr><th></th>${plan.states.map((s) => `<th>${s}</th>`).join('')}</tr>${rows.join('')}</table>`;
      const context = await browser.newContext({ viewport: { width: 1600, height: 900 } });
      const page = await context.newPage();
      await page.setContent(html);
      const sheet = join(out, `sheet-${width}.png`);
      await page.screenshot({ path: sheet, fullPage: true });
      await context.close();
      result.sheets.push(sheet);
      console.log(sheet);
    }
  }
  writeFileSync(json, JSON.stringify(result));
} finally {
  await browser.close();
}
