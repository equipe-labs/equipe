// The quality gate's own page measurements, for graders of design tasks.
//
// Deliberately independent of equipe's design lint (crates/cli/assets/design
// and crates/core/src/design/lint): its own browser driver (the Chrome
// DevTools protocol over Node's built-in WebSocket, no packages) and its own
// measuring code, so the eval never grades a design gate with the gate itself.
// It measures; the Rust grader (tasks.rs) decides what passes.
//
//   node page_check.mjs --chrome <path> --page <file.html> --out <dir>
//     prints one JSON object: at 360, 768 and 1440 px wide, whether the page
//     scrolls sideways and what is cut at the edge; text contrast at 360 and
//     1440; whether focus shows on each control as Tab moves through them;
//     the page's structure (lang, viewport, title, landmarks, headings, image
//     alt text, control names, duplicate ids); its visible text and controls;
//     and pictures of it for a reviewer (<out>/<width>-<n>.png).
//   node page_check.mjs --chrome <path> --shot <file.svg|.html> --out <dir>
//     only pictures of a file (an SVG mockup, say), as <out>/shot-<n>.png.
import { spawn } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const argv = process.argv.slice(2);
const opt = (name) => {
  const at = argv.indexOf(name);
  return at < 0 ? null : argv[at + 1];
};
const chrome = opt('--chrome');
const pagePath = opt('--page');
const shotPath = opt('--shot');
const out = resolve(opt('--out') || '.');
if (!chrome || !(pagePath || shotPath)) {
  console.error('usage: page_check.mjs --chrome <path> (--page <file> | --shot <file>) --out <dir>');
  process.exit(2);
}
mkdirSync(out, { recursive: true });
const url = pathToFileURL(resolve(pagePath || shotPath)).href;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const heightFor = (w) => (w < 600 ? 780 : w < 1000 ? 1024 : 900);

// ── Chrome, headless, in a profile of its own ──
const profile = mkdtempSync(join(tmpdir(), 'eqeval-chrome-'));
const proc = spawn(chrome, ['--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`, '--no-first-run', '--no-default-browser-check', '--disable-gpu', '--disable-extensions', '--hide-scrollbars', '--force-color-profile=srgb', '--mute-audio', 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] });
// Chrome stopped, and its profile removed once it has let go of it.
const done = async (code) => {
  const gone = proc.exitCode !== null ? Promise.resolve() : new Promise((r) => proc.once('exit', r));
  try { proc.kill('SIGKILL'); } catch {}
  await Promise.race([gone, sleep(3000)]);
  for (let i = 0; i < 5; i++) {
    try { rmSync(profile, { recursive: true, force: true }); break; } catch { await sleep(200); }
  }
  process.exit(code);
};
const wsUrl = await new Promise((res, rej) => {
  let buf = '';
  const t = setTimeout(() => rej(new Error('Chrome did not start')), 30000);
  proc.stderr.on('data', (d) => {
    buf += d;
    const m = buf.match(/DevTools listening on (ws:\/\/\S+)/);
    if (m) { clearTimeout(t); res(m[1]); }
  });
  proc.on('exit', (c) => rej(new Error(`Chrome exited (${c}): ${buf.slice(-400)}`)));
}).catch((e) => { console.error(String(e)); return done(3); });

const ws = new WebSocket(wsUrl);
await new Promise((r, j) => { ws.onopen = r; ws.onerror = () => j(new Error('no DevTools connection')); });
let nextId = 1;
const pending = new Map();
const listeners = new Set();
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) {
    const { res, rej } = pending.get(msg.id);
    pending.delete(msg.id);
    if (msg.error) rej(new Error(msg.error.message)); else res(msg.result);
  } else if (msg.method) {
    for (const l of listeners) l(msg);
  }
};
const send = (method, params = {}, sessionId) => new Promise((res, rej) => {
  const id = nextId++;
  pending.set(id, { res, rej });
  ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
});
const once = (method, sessionId, ms) => new Promise((res) => {
  const l = (m) => { if (m.method === method && m.sessionId === sessionId) { listeners.delete(l); res(m.params); } };
  listeners.add(l);
  setTimeout(() => { listeners.delete(l); res(null); }, ms);
});

// A tab at `width`, the page loaded, its fonts in, motion off (a measure, not a show).
const open = async (width) => {
  const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
  const s = (method, params) => send(method, params, sessionId);
  await s('Page.enable');
  await s('Runtime.enable');
  await s('Emulation.setDeviceMetricsOverride', { width, height: heightFor(width), deviceScaleFactor: 1, mobile: width < 600 });
  const loaded = once('Page.loadEventFired', sessionId, 20000);
  await s('Page.navigate', { url });
  await loaded;
  const evaluate = async (expression) => {
    const r = await s('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
    return r.result.value;
  };
  await evaluate('document.fonts ? document.fonts.ready.then(() => true) : true');
  await evaluate(`(() => { const st = document.createElement('style'); st.textContent = '*,*::before,*::after{transition:none!important;animation:none!important;scroll-behavior:auto!important}'; (document.head || document.documentElement).appendChild(st); return true; })()`);
  // Scroll through once, so content shown on scrolling is in.
  await evaluate(`(async () => { const h = document.documentElement.scrollHeight; for (let y = 0; y < h; y += Math.max(200, innerHeight / 2)) { scrollTo(0, y); await new Promise(r => setTimeout(r, 60)); } scrollTo(0, 0); await new Promise(r => setTimeout(r, 200)); return true; })()`);
  return { s, evaluate, close: () => send('Target.closeTarget', { targetId }) };
};

// Pictures of the page at its width: tiles of at most `tile` px, at most `most` of them.
const shots = async (tab, width, prefix, tile, most) => {
  const metrics = await tab.s('Page.getLayoutMetrics');
  const height = Math.ceil((metrics.cssContentSize || metrics.contentSize).height);
  const files = [];
  for (let y = 0, n = 1; y < height && n <= most; y += tile, n++) {
    const h = Math.min(tile, height - y);
    const { data } = await tab.s('Page.captureScreenshot', { format: 'png', captureBeyondViewport: true, clip: { x: 0, y, width, height: h, scale: 1 } });
    const file = join(out, `${prefix}-${n}.png`);
    writeFileSync(file, Buffer.from(data, 'base64'));
    files.push(file);
  }
  return { files, height };
};

// ── Measured in the page ──
const helpers = String.raw`
  const describe = (el) => {
    let s = el.tagName.toLowerCase();
    if (el.id) s += '#' + el.id;
    else if (typeof el.className === 'string' && el.className.trim()) s += '.' + el.className.trim().split(/\s+/).slice(0, 2).join('.');
    const t = (el.innerText || el.getAttribute('aria-label') || '').trim().replace(/\s+/g, ' ').slice(0, 40);
    return t ? s + ' "' + t + '"' : s;
  };
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 1;
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  const memo = new Map();
  // Any CSS colour as [r, g, b, a] in sRGB, a in 0..1: drawn and read back.
  const rgba = (c) => {
    if (memo.has(c)) return memo.get(c);
    ctx.clearRect(0, 0, 1, 1);
    ctx.fillStyle = 'rgba(0,0,0,0)';
    ctx.fillStyle = c;
    ctx.fillRect(0, 0, 1, 1);
    const d = ctx.getImageData(0, 0, 1, 1).data;
    const v = [d[0], d[1], d[2], d[3] / 255];
    memo.set(c, v);
    return v;
  };
  const over = (top, under) => {
    const a = top[3] + under[3] * (1 - top[3]);
    if (a === 0) return [0, 0, 0, 0];
    return [0, 1, 2].map((i) => (top[i] * top[3] + under[i] * under[3] * (1 - top[3])) / a).concat([a]);
  };
  const lum = (c) => {
    const f = (v) => { v /= 255; return v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); };
    return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]);
  };
  const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
  const shown = (el) => {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden' || cs.visibility === 'collapse') return false;
    const r = el.getBoundingClientRect();
    if (r.width * r.height <= 4) return false;
    if (cs.clip && cs.clip.startsWith('rect(') && /rect\(0(px)?,? 0(px)?,? 0(px)?,? 0(px)?\)/.test(cs.clip)) return false;
    return true;
  };
  const ownText = (el) => Array.from(el.childNodes).some((n) => n.nodeType === 3 && n.textContent.trim());
`;

const measureOverflow = `(() => { ${helpers}
  const vw = document.documentElement.clientWidth;
  const scroll = Math.max(document.documentElement.scrollWidth, document.body ? document.body.scrollWidth : 0);
  const cut = [];
  for (const el of document.body ? document.body.querySelectorAll('*') : []) {
    const visual = ownText(el) || ['IMG', 'VIDEO', 'BUTTON', 'A', 'INPUT', 'SELECT', 'TEXTAREA', 'CANVAS'].includes(el.tagName);
    if (!visual || !shown(el)) continue;
    const r = el.getBoundingClientRect();
    // Out of sight entirely (an off-canvas menu) isn't cut; partly past an edge is.
    if (r.right <= 0 || r.left >= vw) continue;
    if (r.right <= vw + 1 && r.left >= -1) continue;
    // Inside a box of its own that scrolls or clips within the page (a carousel): fine.
    let a = el.parentElement, held = false;
    while (a && a !== document.body && a !== document.documentElement) {
      if (getComputedStyle(a).overflowX !== 'visible') {
        const ar = a.getBoundingClientRect();
        if (ar.right <= vw + 1 && ar.left >= -1) { held = true; break; }
      }
      a = a.parentElement;
    }
    if (!held) cut.push(describe(el) + ' ' + Math.round(r.left) + '..' + Math.round(r.right));
  }
  return { viewport: vw, scrollWidth: scroll, scrolls: scroll > vw + 1, cut: cut.slice(0, 12), cutCount: cut.length };
})()`;

const measureContrast = String.raw`(() => { ${helpers}
  const failures = [];
  let checked = 0, unknown = 0;
  const unknownWhy = [];
  for (const el of document.body ? document.body.querySelectorAll('*') : []) {
    if (!ownText(el) || !shown(el) || ['SCRIPT', 'STYLE', 'NOSCRIPT', 'TITLE', 'OPTION'].includes(el.tagName)) continue;
    const cs = getComputedStyle(el);
    let opacity = 1;
    for (let a = el; a; a = a.parentElement) opacity *= parseFloat(getComputedStyle(a).opacity);
    if (opacity < 0.05) continue;
    const fill = cs.webkitTextFillColor && cs.webkitTextFillColor !== cs.color ? rgba(cs.webkitTextFillColor) : null;
    if (fill && fill[3] === 0) { unknown++; unknownWhy.push(describe(el) + ': gradient or clipped text'); continue; }
    const fg0 = rgba(cs.color);
    // What the text sits on, from its own box outwards to the first opaque
    // one: plain colours, or a gradient's colour stops (the worst one counts).
    // A picture under it can't be measured from styles: counted as unknown.
    const layers = [];
    let image = null;
    for (let a = el; a; a = a.parentElement) {
      const as = getComputedStyle(a);
      if (['IMG', 'VIDEO', 'CANVAS'].includes(a.tagName)) { image = describe(a); break; }
      const c = rgba(as.backgroundColor);
      const bi = as.backgroundImage || 'none';
      if (bi !== 'none') {
        if (/url\(/.test(bi)) { if (c[3] < 1) image = describe(a); else layers.push([c]); break; }
        const stops = (bi.match(/(rgba?|hsla?|oklch|oklab|lab|lch|color|hwb)\([^()]*\)/g) || []).map(rgba);
        if (stops.length) layers.push(stops);
      }
      if (c[3] > 0) layers.push([c]);
      if (c[3] >= 1) break;
    }
    if (image) { unknown++; unknownWhy.push(describe(el) + ': on a picture (' + image + ')'); continue; }
    let under = [[255, 255, 255, 1]];
    for (let i = layers.length - 1; i >= 0; i--) under = layers[i].flatMap((top) => under.map((u) => over(top, u)));
    const size = parseFloat(cs.fontSize);
    const bold = parseInt(cs.fontWeight, 10) >= 700;
    const large = size >= 24 || (size >= 18.66 && bold);
    const need = large ? 3 : 4.5;
    let r = Infinity, bg = under[0];
    for (const u of under.slice(0, 64)) {
      const x = ratio(over([fg0[0], fg0[1], fg0[2], fg0[3] * opacity], u), u);
      if (x < r) { r = x; bg = u; }
    }
    checked++;
    if (r + 1e-6 < need) failures.push({ el: describe(el), ratio: Math.round(r * 100) / 100, need, color: cs.color, background: 'rgb(' + bg.slice(0, 3).map(Math.round).join(',') + ')' });
  }
  return { checked, unknown, unknownWhy: unknownWhy.slice(0, 8), failures: failures.slice(0, 15), failureCount: failures.length };
})()`;

const focusPrepare = `(() => { ${helpers}
  const sel = 'a[href], button, input:not([type=hidden]), select, textarea, summary, [tabindex], [contenteditable=""], [contenteditable=true]';
  const sig = (el) => {
    const pick = (cs) => {
      const outline = cs.outlineStyle === 'none' || parseFloat(cs.outlineWidth) === 0 || rgba(cs.outlineColor)[3] === 0 ? 'none' : [cs.outlineStyle, cs.outlineWidth, cs.outlineColor, cs.outlineOffset].join(' ');
      return [outline, cs.boxShadow, cs.borderTopColor, cs.borderBottomColor, cs.borderTopWidth, cs.borderBottomWidth, cs.backgroundColor, cs.backgroundImage, cs.color, cs.textDecorationLine, cs.textDecorationColor, cs.transform, cs.opacity].join('|');
    };
    const pseudo = (p) => { const cs = getComputedStyle(el, p); return cs.content === 'none' ? '' : pick(cs) + cs.width + cs.height; };
    const parent = el.parentElement ? (() => { const cs = getComputedStyle(el.parentElement); return [cs.outlineStyle === 'none' ? 'none' : cs.outlineStyle + cs.outlineWidth, cs.boxShadow, cs.borderTopColor, cs.backgroundColor].join('|'); })() : '';
    return pick(getComputedStyle(el)) + '#' + pseudo('::before') + '#' + pseudo('::after') + '#' + parent;
  };
  window.__eqevalSig = sig;
  window.__eqevalDescribe = describe;
  if (document.activeElement) document.activeElement.blur();
  const list = Array.from(document.querySelectorAll(sel)).filter((el) => {
    if (el.disabled || el.getAttribute('tabindex') === '-1' || el.closest('[inert]')) return false;
    return shown(el);
  });
  window.__eqevalFocusables = list;
  window.__eqevalBefore = list.map(sig);
  return list.length;
})()`;

const focusRead = `(() => {
  const el = document.activeElement;
  const i = window.__eqevalFocusables.indexOf(el);
  if (!el || el === document.body || i < 0) return { index: -1, name: el ? el.tagName.toLowerCase() : '' };
  return { index: i, name: window.__eqevalDescribe(el), visible: window.__eqevalSig(el) !== window.__eqevalBefore[i] };
})()`;

const measureStructure = `(() => { ${helpers}
  const html = document.documentElement;
  const meta = document.querySelector('meta[name=viewport]');
  const heads = Array.from(document.querySelectorAll('h1, h2, h3, h4, h5, h6')).filter(shown).map((h) => parseInt(h.tagName[1], 10));
  const skips = [];
  for (let i = 1; i < heads.length; i++) if (heads[i] > heads[i - 1] + 1) skips.push('h' + heads[i - 1] + ' → h' + heads[i]);
  if (heads.length && heads[0] !== 1) skips.unshift('starts at h' + heads[0]);
  const byId = (id) => id && document.getElementById(id);
  const named = (el) => {
    if ((el.getAttribute('aria-label') || '').trim()) return true;
    const by = el.getAttribute('aria-labelledby');
    if (by && by.split(/\s+/).some((id) => byId(id) && byId(id).textContent.trim())) return true;
    if (el.id && Array.from(document.querySelectorAll('label')).some((l) => l.htmlFor === el.id && l.textContent.trim())) return true;
    const wrap = el.closest('label');
    if (wrap && wrap.textContent.trim()) return true;
    if ((el.getAttribute('title') || '').trim()) return true;
    return false;
  };
  const controls = Array.from(document.querySelectorAll('input:not([type=hidden]):not([type=submit]):not([type=button]):not([type=reset]):not([type=image]), select, textarea'));
  const unnamedControls = controls.filter((c) => !named(c)).map(describe);
  const actions = Array.from(document.querySelectorAll('a[href], button, input[type=submit], input[type=button]')).filter(shown);
  const actionName = (el) => (el.innerText || el.value || el.getAttribute('aria-label') || el.getAttribute('title') || Array.from(el.querySelectorAll('img[alt]')).map((i) => i.alt).join(' ') || '').trim();
  const unnamedActions = actions.filter((a) => !actionName(a) && !named(a)).map(describe);
  const imgsNoAlt = Array.from(document.querySelectorAll('img')).filter((i) => !i.hasAttribute('alt')).map(describe);
  const ids = Array.from(document.querySelectorAll('[id]')).map((e) => e.id);
  const dupIds = Array.from(new Set(ids.filter((id, i) => ids.indexOf(id) !== i)));
  const count = (s) => document.querySelectorAll(s).length;
  const emailFields = Array.from(document.querySelectorAll('input[type=email]'));
  return {
    lang: (html.getAttribute('lang') || '').trim(),
    viewport: !!(meta && /width\\s*=\\s*device-width/.test(meta.getAttribute('content') || '')),
    title: (document.title || '').trim(),
    main: count('main, [role=main]'),
    header: count('header, [role=banner]'),
    footer: count('footer, [role=contentinfo]'),
    h1: Array.from(document.querySelectorAll('h1')).filter(shown).length,
    headings: heads,
    skips,
    imgsNoAlt,
    unnamedControls,
    unnamedActions,
    dupIds,
    emailField: emailFields.length > 0,
    emailFieldNamed: emailFields.some(named),
    emailInForm: emailFields.some((e) => !!e.closest('form')),
    actions: actions.map((a) => ({ tag: a.tagName.toLowerCase(), text: actionName(a).replace(/\\s+/g, ' ').slice(0, 80) })),
    text: document.body ? document.body.innerText : '',
  };
})()`;

try {
  if (shotPath) {
    const tab = await open(1200);
    const { files } = await shots(tab, 1200, 'shot', 1600, 2);
    await tab.close();
    console.log(JSON.stringify({ shots: files }));
    await done(0);
  }
  const result = { url, widths: {}, contrast: {}, focus: null, structure: null, shots: [] };
  for (const width of [360, 768, 1440]) {
    const tab = await open(width);
    result.widths[width] = await tab.evaluate(measureOverflow);
    if (width !== 768) result.contrast[width] = await tab.evaluate(measureContrast);
    if (width === 1440) {
      result.structure = await tab.evaluate(measureStructure);
      const total = await tab.evaluate(focusPrepare);
      const reached = new Map();
      for (let i = 0; i < total + 2; i++) {
        await tab.s('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: 'Tab', code: 'Tab', windowsVirtualKeyCode: 9, nativeVirtualKeyCode: 9 });
        await tab.s('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Tab', code: 'Tab', windowsVirtualKeyCode: 9, nativeVirtualKeyCode: 9 });
        await sleep(40);
        const at = await tab.evaluate(focusRead);
        if (at.index >= 0 && !reached.has(at.index)) reached.set(at.index, at);
      }
      const unseen = Array.from(reached.values()).filter((r) => !r.visible).map((r) => r.name);
      result.focus = { focusable: total, reached: reached.size, noIndicator: unseen.slice(0, 12), noIndicatorCount: unseen.length };
    }
    if (width === 360 || width === 1440) {
      const s = await shots(tab, width, String(width), width === 360 ? 1400 : 1800, 4);
      result.shots.push(...s.files);
    }
    await tab.close();
  }
  console.log(JSON.stringify(result));
  await done(0);
} catch (e) {
  console.error(String(e && e.stack || e));
  await done(1);
}
