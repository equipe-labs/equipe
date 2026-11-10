// `equipe design lint --render`: a page drawn in the Mac's own Chrome,
// headless, at each width, and measured — nothing judged here. The rules
// that read these measures live in equipe's core (design/lint/render.rs).
//
//   lint.mjs <url|file> --widths 360,1280 --out <dir> --json <file>
//     at each width: the page's width against the window's, what reaches
//     past the edge, text cut off by its box, each text's colour and what it
//     sits on (in its dark scheme too, where it has one, at the narrowest
//     and widest widths), whether focus shows as Tab moves through the controls, where
//     the elements marked data-primary are, how much text is centred, the
//     radius and shadow of raised blocks, line lengths and target sizes; and
//     a picture, whole (<width>.png) and the first screenful (<width>.first.png).
//   lint.mjs <url|file> --inventory --json <file>
//     what the page offers: its links, fields, actions and headings.
import { chromium } from 'playwright';
import { resolve, join } from 'node:path';
import { mkdirSync, writeFileSync } from 'node:fs';
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
const widths = opt('--widths', '360,390,768,1024,1280,1440,1920').split(',').map((w) => parseInt(w, 10)).filter((w) => w > 0);
const out = resolve(opt('--out', '.'));
const json = opt('--json', null);
const inventory = flag('--inventory');
const target = args[0];
if (!target || !json) {
  console.error('usage: lint.mjs <url|file> [--widths 360,1280] [--out dir] --json <file> [--inventory]');
  process.exit(2);
}
const url = /^(https?|file|data):/.test(target) ? target : pathToFileURL(resolve(target)).href;
// A window as tall as the screens these widths come on.
const heightFor = (w) => (w <= 480 ? 844 : w <= 800 ? 1024 : w <= 1100 ? 768 : w <= 1300 ? 800 : w <= 1600 ? 900 : 1080);
const TALLEST = 16384;

const open = async (browser, width, colorScheme = 'light') => {
  const context = await browser.newContext({ viewport: { width, height: heightFor(width) }, deviceScaleFactor: 1, locale: 'en-US', colorScheme });
  const page = await context.newPage();
  await page.goto(url, { waitUntil: 'networkidle', timeout: 45000 }).catch(() => page.waitForLoadState('load'));
  await page.evaluate(() => document.fonts && document.fonts.ready);
  await page.waitForTimeout(500);
  return { context, page };
};

// Everything measured in the page, at one width.
const measure = () => {
  const vw = document.documentElement.clientWidth;
  // An element as a person finds it: its tag, id or classes, and a control's words.
  const name = (el) => {
    let s = el.tagName.toLowerCase();
    if (el.id) return `${s}#${el.id}`;
    const cls = [...el.classList].slice(0, 2);
    if (cls.length) s += '.' + cls.join('.');
    const words = (el.innerText || el.value || el.getAttribute('aria-label') || '').replace(/\s+/g, ' ').trim();
    if (words && el.matches('a, button, [role=button], label, h1, h2, h3, h4, h5, h6')) s += ` “${words.slice(0, 24)}”`;
    return s;
  };
  const shown = (el) => {
    const st = getComputedStyle(el);
    if (st.display === 'none' || st.visibility === 'hidden' || parseFloat(st.opacity) === 0) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  };
  const ownText = (el) => [...el.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent).join(' ').replace(/\s+/g, ' ').trim();
  const all = [...document.body.querySelectorAll('*')].filter((el) => !['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'svg', 'path'].includes(el.tagName));
  // Inside something that scrolls or clips on its own: its overflow isn't the page's.
  const contained = (el) => {
    for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
      const o = getComputedStyle(p).overflowX;
      if (o === 'auto' || o === 'scroll' || o === 'hidden' || o === 'clip') return true;
    }
    return false;
  };
  const wide = all
    .filter((el) => shown(el) && el.getBoundingClientRect().right > vw + 1 && !contained(el))
    .map((el) => ({ selector: name(el), right: el.getBoundingClientRect().right }))
    .sort((a, b) => b.right - a.right)
    .slice(0, 5);
  const clipped = [];
  for (const el of all) {
    if (clipped.length >= 20) break;
    const text = (el.innerText || '').trim();
    if (!text || !shown(el)) continue;
    const st = getComputedStyle(el);
    const hides = (o) => o === 'hidden' || o === 'clip';
    const x = hides(st.overflowX) && el.scrollWidth > el.clientWidth + 1;
    const y = hides(st.overflowY) && el.scrollHeight > el.clientHeight + 1;
    if (!x && !y) continue;
    const ellipsis = st.textOverflow === 'ellipsis' || (st.webkitLineClamp && st.webkitLineClamp !== 'none');
    // Only the innermost box that cuts.
    if ([...el.children].some((c) => clipped.some((k) => k.el === c))) continue;
    clipped.push({ el, selector: name(el), text: text.slice(0, 80), axis: x ? 'x' : 'y', ellipsis: !!ellipsis });
  }
  const parse = (c) => {
    const m = c.match(/rgba?\(([^)]+)\)/);
    if (!m) return null;
    const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number);
    return { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 };
  };
  // What a text sits on: the backgrounds under it, composed, until one is opaque.
  const backdrop = (el) => {
    const layers = [];
    for (let p = el; p; p = p.parentElement) {
      const st = getComputedStyle(p);
      if (st.backgroundImage && st.backgroundImage !== 'none') return null;
      const c = parse(st.backgroundColor);
      if (c && c.a > 0) {
        layers.push(c);
        if (c.a >= 1) break;
      }
    }
    let bg = { r: 255, g: 255, b: 255 };
    for (const c of layers.reverse()) bg = { r: c.r * c.a + bg.r * (1 - c.a), g: c.g * c.a + bg.g * (1 - c.a), b: c.b * c.a + bg.b * (1 - c.a) };
    return `rgb(${Math.round(bg.r)}, ${Math.round(bg.g)}, ${Math.round(bg.b)})`;
  };
  // Native :disabled includes fieldset inheritance and its first-legend exception.
  // Do not exempt a whole fieldset: its prose, links and first legend stay active.
  // ARIA/data attributes and CSS alone do not prove behavioral inactivity.
  const inactiveText = (el) => !!el.closest(':disabled:not(fieldset)');
  const texts = [];
  for (const el of all) {
    if (texts.length >= 400) break;
    const text = ownText(el);
    if (!text || !shown(el) || inactiveText(el)) continue;
    const st = getComputedStyle(el);
    texts.push({ selector: name(el), text: text.slice(0, 60), fg: st.color, bg: backdrop(el), size: parseFloat(st.fontSize), weight: parseInt(st.fontWeight, 10) || 400 });
  }
  const primary = [...document.querySelectorAll('[data-primary]')].filter(shown).map((el) => {
    const r = el.getBoundingClientRect();
    return { selector: name(el), top: r.top + window.scrollY, bottom: r.bottom + window.scrollY };
  });
  const prose = all.filter((el) => /^(H[1-6]|P|LI|BLOCKQUOTE|DD|FIGCAPTION)$/.test(el.tagName) && shown(el) && (el.innerText || '').trim());
  const centred = { total: prose.length, centred: prose.filter((el) => getComputedStyle(el).textAlign === 'center').length };
  const blocks = all
    .filter((el) => {
      if (!shown(el)) return false;
      const st = getComputedStyle(el);
      const r = el.getBoundingClientRect();
      return st.boxShadow !== 'none' && st.borderRadius !== '0px' && r.width >= 80 && r.height >= 40;
    })
    .slice(0, 60)
    .map((el) => ({ radius: getComputedStyle(el).borderRadius, shadow: getComputedStyle(el).boxShadow }));
  const lines = prose
    .filter((el) => /^(P|LI|DD|BLOCKQUOTE)$/.test(el.tagName) && (el.innerText || '').length > 160)
    .slice(0, 40)
    .map((el) => ({ selector: name(el), chars: Math.round(el.getBoundingClientRect().width / (parseFloat(getComputedStyle(el).fontSize) * 0.5)) }));
  const controls = all.filter((el) => el.matches('a[href], button, input:not([type=hidden]), select, textarea, [role=button]') && shown(el));
  const inline = (el) => el.tagName === 'A' && getComputedStyle(el).display === 'inline' && el.parentElement && ownText(el.parentElement).length > 0;
  const targets = controls.filter((el) => !inline(el)).slice(0, 60).map((el) => {
    const r = el.getBoundingClientRect();
    return { selector: name(el), w: r.width, h: r.height };
  });
  return {
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: vw,
    wide,
    clipped: clipped.map(({ el, ...rest }) => rest),
    texts,
    primary,
    centred,
    blocks,
    lines,
    targets,
  };
};

// Tab through the controls; each should look different focused than not.
const focusCheck = async (page) => {
  const before = await page.evaluate(() => {
    const look = (el) => {
      const st = getComputedStyle(el);
      return [st.outlineStyle, st.outlineWidth, st.outlineColor, st.boxShadow, st.borderColor, st.backgroundColor, st.textDecorationLine, st.color].join('|');
    };
    const controls = [...document.querySelectorAll('a[href], button, input:not([type=hidden]), select, textarea, [tabindex]:not([tabindex="-1"])')].filter((el) => {
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0 && !el.disabled;
    });
    if (document.activeElement && document.activeElement !== document.body) document.activeElement.blur();
    return controls.slice(0, 30).map((el, i) => {
      el.setAttribute('data-equipe-focus', String(i));
      return look(el);
    });
  });
  const seen = [];
  for (let n = 0; n < Math.min(before.length + 2, 32); n++) {
    await page.keyboard.press('Tab');
    const now = await page.evaluate(() => {
      const el = document.activeElement;
      if (!el || !el.hasAttribute('data-equipe-focus')) return null;
      const st = getComputedStyle(el);
      const words = (el.innerText || el.value || el.getAttribute('aria-label') || '').replace(/\s+/g, ' ').trim().slice(0, 24);
      const name = (el.id ? `${el.tagName.toLowerCase()}#${el.id}` : el.tagName.toLowerCase() + [...el.classList].slice(0, 2).map((c) => '.' + c).join('')) + (words ? ` “${words}”` : '');
      const outline = st.outlineStyle !== 'none' && parseFloat(st.outlineWidth) > 0 && !/rgba\([^)]*,\s*0\)/.test(st.outlineColor);
      return { i: Number(el.getAttribute('data-equipe-focus')), selector: name, outline, look: [st.outlineStyle, st.outlineWidth, st.outlineColor, st.boxShadow, st.borderColor, st.backgroundColor, st.textDecorationLine, st.color].join('|') };
    });
    if (!now || seen.some((s) => s.i === now.i)) continue;
    seen.push({ i: now.i, selector: now.selector, visible: now.outline || now.look !== before[now.i] });
  }
  return seen.map(({ selector, visible }) => ({ selector, visible }));
};

const offered = () => {
  const words = (el) => (el.innerText || el.value || '').replace(/\s+/g, ' ').trim();
  return {
    links: [...document.querySelectorAll('a[href]')].map((a) => a.getAttribute('href')).filter((h) => h && h !== '#'),
    fields: [...document.querySelectorAll('input:not([type=hidden]), select, textarea')].map((f) => f.getAttribute('name') || f.id).filter(Boolean),
    actions: [...document.querySelectorAll('button, input[type=submit]')].map(words).filter(Boolean),
    headings: [...document.querySelectorAll('h1, h2, h3, h4, h5, h6')].map(words).filter(Boolean),
  };
};

const browser = await chromium.launch({ channel: 'chrome', headless: true });
try {
  if (inventory) {
    const { context, page } = await open(browser, 1280);
    writeFileSync(json, JSON.stringify(await page.evaluate(offered)));
    await context.close();
  } else {
    mkdirSync(out, { recursive: true });
    const result = { url, widths: [] };
    for (const [i, width] of widths.entries()) {
      const { context, page } = await open(browser, width);
      const m = await page.evaluate(measure);
      // A page with a dark scheme is read in it too, at the narrowest and
      // widest widths: a colour that reads in light can fail in dark.
      if (i === 0 || i === widths.length - 1) {
        const dark = await open(browser, width, 'dark');
        const d = await dark.page.evaluate(measure);
        const pairs = (ts) => JSON.stringify(ts.map((t) => [t.fg, t.bg]));
        if (pairs(d.texts) !== pairs(m.texts)) m.texts.push(...d.texts.map((t) => ({ ...t, selector: `${t.selector} (dark)` })));
        await dark.context.close();
      }
      m.focus = await focusCheck(page);
      await page.evaluate(() => {
        if (document.activeElement) document.activeElement.blur();
        window.scrollTo(0, 0);
      });
      const picture = join(out, `${width}.png`);
      const tall = (await page.evaluate(() => document.documentElement.scrollHeight)) > TALLEST;
      await page.screenshot({ path: picture, fullPage: !tall });
      await page.screenshot({ path: join(out, `${width}.first.png`) });
      result.widths.push({ width, height: heightFor(width), picture, ...m });
      console.log(picture);
      await context.close();
    }
    writeFileSync(json, JSON.stringify(result));
  }
} finally {
  await browser.close();
}
