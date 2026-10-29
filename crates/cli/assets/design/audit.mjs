// `equipe design audit <url>`: what a brand really uses, harvested from its
// site as Chrome renders it — before anything is designed. Visits the page
// and a few of its own (nav links), and takes: colours as the pages paint
// them, weighted by the area they cover (backgrounds, text, borders) and
// where each was seen; the type families and the webfonts loaded; the type
// scale (h1…body, buttons); radii and shadows; the logo (header image or
// SVG, favicon, apple-touch-icon, og:image) saved as files; and its voice
// (title, description, headlines, buttons, navigation). Writes
// brand-audit.json, brand-audit.html — one sheet — and brand-audit.png, plus
// a screenshot of each page, into --out; prints a summary to read.
import { chromium } from 'playwright';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve, extname } from 'node:path';
import { pathToFileURL } from 'node:url';

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const at = args.indexOf(name);
  if (at < 0) return fallback;
  const value = args[at + 1];
  args.splice(at, 2);
  return value;
};
const pagesWanted = Math.max(1, Math.min(8, parseInt(opt('--pages', '3'), 10) || 3));
const out = resolve(opt('--out', 'design/audit'));
let start = args[0];
if (!start) {
  console.error('usage: audit <url> [--pages 3] [--out design/audit]');
  process.exit(2);
}
if (!/^https?:\/\//.test(start)) start = 'https://' + start;
mkdirSync(out, { recursive: true });

// What one page paints, read in the page.
function collect() {
  const parse = (s) => {
    const m = s && s.match(/rgba?\(([^)]+)\)/);
    if (!m) return null;
    const p = m[1].split(/[\s,/]+/).filter(Boolean).map(Number);
    const a = p.length > 3 ? p[3] : 1;
    if (a < 0.08) return null;
    return [p[0], p[1], p[2], a];
  };
  const hex = (c) => '#' + c.slice(0, 3).map((v) => Math.round(v).toString(16).padStart(2, '0')).join('').toUpperCase();
  const colors = {};
  const add = (c, weight, as, where) => {
    if (!c || weight <= 0) return;
    const key = hex(c);
    const e = (colors[key] ||= { hex: key, weight: 0, as: {}, where: {} });
    e.weight += weight;
    e.as[as] = (e.as[as] || 0) + weight;
    e.where[where] = (e.where[where] || 0) + 1;
  };
  const label = (el) => {
    const tag = el.tagName.toLowerCase();
    if (['button', 'a', 'h1', 'h2', 'h3', 'nav', 'header', 'footer', 'input', 'label'].includes(tag)) return tag;
    if (el.closest('header')) return 'header';
    if (el.closest('footer')) return 'footer';
    if (el.closest('nav')) return 'nav';
    if (el.getAttribute('role') === 'button') return 'button';
    return tag === 'body' || tag === 'main' || tag === 'section' ? tag : 'block';
  };
  const visible = (el, cs, r) => cs.display !== 'none' && cs.visibility !== 'hidden' && parseFloat(cs.opacity) > 0.05 && r.width >= 2 && r.height >= 2;
  const docH = Math.max(document.documentElement.scrollHeight, innerHeight);
  // The page's own background, over the whole page.
  for (const el of [document.documentElement, document.body]) {
    const c = parse(getComputedStyle(el).backgroundColor);
    if (c) add(c, innerWidth * docH, 'bg', 'page');
  }
  const els = Array.from(document.body.querySelectorAll('*')).slice(0, 8000);
  const radii = {};
  const shadows = {};
  for (const el of els) {
    const cs = getComputedStyle(el);
    const r = el.getBoundingClientRect();
    if (!visible(el, cs, r)) continue;
    const where = label(el);
    const area = Math.min(r.width, innerWidth) * Math.min(r.height, docH);
    const bg = parse(cs.backgroundColor);
    if (bg) {
      const parent = el.parentElement && parse(getComputedStyle(el.parentElement).backgroundColor);
      if (!parent || hex(parent) !== hex(bg)) add(bg, area, 'bg', where);
    }
    const own = Array.from(el.childNodes).filter((n) => n.nodeType === 3).map((n) => n.textContent.trim()).join(' ');
    if (own.length) {
      const size = parseFloat(cs.fontSize) || 16;
      add(parse(cs.color), own.length * size * size * 0.55, 'text', where);
    }
    const bw = parseFloat(cs.borderTopWidth) + parseFloat(cs.borderBottomWidth) + parseFloat(cs.borderLeftWidth) + parseFloat(cs.borderRightWidth);
    if (bw > 0 && cs.borderStyle !== 'none') add(parse(cs.borderTopColor), (r.width + r.height) * 2 * (bw / 4), 'border', where);
    if (['button', 'input', 'a'].includes(where) || cs.boxShadow !== 'none' || (bg && cs.borderRadius !== '0px')) {
      if (cs.borderRadius && cs.borderRadius !== '0px') {
        const k = `${where}:${cs.borderRadius}`;
        radii[k] = (radii[k] || 0) + 1;
      }
    }
    if (cs.boxShadow && cs.boxShadow !== 'none') shadows[cs.boxShadow] = (shadows[cs.boxShadow] || 0) + 1;
  }
  const first = (sel) => Array.from(document.querySelectorAll(sel)).find((el) => {
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0 && el.textContent.trim().length > 0;
  });
  const typeOf = (el) => {
    if (!el) return null;
    const cs = getComputedStyle(el);
    return { family: cs.fontFamily, size: cs.fontSize, weight: cs.fontWeight, lineHeight: cs.lineHeight, letterSpacing: cs.letterSpacing, transform: cs.textTransform, sample: el.textContent.trim().replace(/\s+/g, ' ').slice(0, 60) };
  };
  const type = {};
  for (const [name, sel] of [['h1', 'h1'], ['h2', 'h2'], ['h3', 'h3'], ['body', 'p, li'], ['button', 'button, [role=button], a.button, a.btn'], ['link', 'a'], ['caption', 'small, figcaption, label']]) {
    const t = typeOf(first(sel));
    if (t) type[name] = t;
  }
  const faces = [];
  if (document.fonts) for (const f of document.fonts) if (f.status === 'loaded') faces.push(`${f.family.replace(/"/g, '')} ${f.weight} ${f.style}`);
  const abs = (u) => { try { return new URL(u, location.href).href; } catch { return null; } };
  const logos = [];
  const looksLogo = (el) => /logo|brand|wordmark/i.test([el.id, el.className && el.className.baseVal !== undefined ? el.className.baseVal : el.className, el.getAttribute('alt'), el.getAttribute('aria-label'), el.getAttribute('src')].join(' '));
  for (const el of document.querySelectorAll('header img, header svg, nav img, nav svg, a[href="/"] img, a[href="/"] svg, [class*=logo] img, [class*=logo] svg, img[alt*=logo i], img[src*=logo i]')) {
    const r = el.getBoundingClientRect();
    if (r.width < 12 || r.height < 8) continue;
    // The site's own mark sits at the top; logos further down are its clients'.
    if (r.top + window.scrollY > 220) continue;
    if (!looksLogo(el) && !el.closest('a[href="/"], [class*=logo], [id*=logo]')) continue;
    if (el.tagName.toLowerCase() === 'svg') logos.push({ kind: 'svg', svg: el.outerHTML.slice(0, 120000), width: r.width, height: r.height });
    else if (!logos.some((l) => l.src === abs(el.currentSrc || el.src))) logos.push({ kind: 'img', src: abs(el.currentSrc || el.src), width: r.width, height: r.height });
    if (logos.length >= 3) break;
  }
  const meta = (sel, attr = 'content') => document.querySelector(sel)?.getAttribute(attr) || null;
  const icons = Array.from(document.querySelectorAll('link[rel~="icon"], link[rel="apple-touch-icon"], link[rel="mask-icon"]')).map((l) => ({ rel: l.getAttribute('rel'), href: abs(l.getAttribute('href')), sizes: l.getAttribute('sizes') }));
  const texts = (sel, n, max = 120) => Array.from(new Set(Array.from(document.querySelectorAll(sel)).map((e) => e.textContent.trim().replace(/\s+/g, ' ')).filter((t) => t.length > 1 && t.length <= max))).slice(0, n);
  const links = Array.from(document.querySelectorAll('header a[href], nav a[href], footer a[href], main a[href]')).map((a) => abs(a.getAttribute('href'))).filter(Boolean);
  return {
    url: location.href,
    lang: document.documentElement.lang || null,
    colors: Object.values(colors),
    radii,
    shadows,
    type,
    faces: Array.from(new Set(faces)),
    logos,
    icons,
    ogImage: abs(meta('meta[property="og:image"]') || '') ,
    themeColor: meta('meta[name="theme-color"]'),
    voice: {
      title: document.title,
      description: meta('meta[name="description"]') || meta('meta[property="og:description"]'),
      headlines: texts('h1, h2', 10),
      buttons: texts('button, [role=button], a.button, a.btn, input[type=submit]', 15, 40),
      nav: texts('nav a, header a', 15, 30),
    },
    links,
  };
}

const browser = await chromium.launch({ channel: 'chrome', headless: true });
const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1, locale: 'ko-KR' });
const page = await context.newPage();
const visited = [];
const queue = [start];
const origin = new URL(start).origin;
const pages = [];
try {
  while (queue.length && pages.length < pagesWanted) {
    const url = queue.shift();
    const key = url.replace(/[#?].*$/, '').replace(/\/$/, '');
    if (visited.includes(key)) continue;
    visited.push(key);
    try {
      await page.goto(url, { waitUntil: 'networkidle', timeout: 45000 }).catch(() => page.waitForLoadState('load', { timeout: 15000 }));
      await page.evaluate(async () => {
        for (let y = 0; y < document.documentElement.scrollHeight; y += innerHeight) { window.scrollTo(0, y); await new Promise((r) => setTimeout(r, 120)); }
        window.scrollTo(0, 0);
        await (document.fonts && document.fonts.ready);
      });
      const shot = join(out, `page-${pages.length + 1}.png`);
      await page.screenshot({ path: shot });
      const data = await page.evaluate(collect);
      data.shot = shot;
      pages.push(data);
      for (const link of data.links) {
        try {
          const u = new URL(link);
          if (u.origin === origin && !/\.(pdf|png|jpe?g|zip)$/i.test(u.pathname)) queue.push(u.href);
        } catch {}
      }
      queue.sort((a, b) => a.length - b.length);
    } catch (e) {
      console.error(`couldn't read ${url}: ${e.message}`);
    }
  }
  if (!pages.length) throw new Error('no page could be read');

  // Colours across pages, near ones merged into the bigger.
  const merged = [];
  const rgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  const near = (a, b) => { const x = rgb(a), y = rgb(b); return Math.hypot(x[0] - y[0], x[1] - y[1], x[2] - y[2]) < 12; };
  const all = pages.flatMap((p, i) => p.colors.map((c) => ({ ...c, page: i })));
  all.sort((a, b) => b.weight - a.weight);
  for (const c of all) {
    const into = merged.find((m) => near(m.hex, c.hex));
    if (into) {
      into.weight += c.weight;
      for (const [k, v] of Object.entries(c.as)) into.as[k] = (into.as[k] || 0) + v;
      for (const [k, v] of Object.entries(c.where)) into.where[k] = (into.where[k] || 0) + v;
      into.pages.add(c.page);
    } else merged.push({ hex: c.hex, weight: c.weight, as: { ...c.as }, where: { ...c.where }, pages: new Set([c.page]) });
  }
  // Logos and icons, saved as files.
  const saved = [];
  const save = async (url, name) => {
    if (!url) return null;
    try {
      const res = await page.request.get(url, { timeout: 20000 });
      if (!res.ok()) return null;
      const type = res.headers()['content-type'] || '';
      const ext = type.includes('svg') ? '.svg' : type.includes('png') ? '.png' : type.includes('jpeg') ? '.jpg' : type.includes('webp') ? '.webp' : type.includes('icon') ? '.ico' : extname(new URL(url).pathname) || '.bin';
      const path = join(out, name + ext);
      writeFileSync(path, await res.body());
      saved.push(path);
      return path;
    } catch { return null; }
  };
  const home = pages[0];
  const logoFiles = [];
  for (const [i, l] of home.logos.entries()) {
    if (l.kind === 'svg') {
      const path = join(out, `logo-${i + 1}.svg`);
      const svg = l.svg.includes('xmlns') ? l.svg : l.svg.replace('<svg', '<svg xmlns="http://www.w3.org/2000/svg"');
      writeFileSync(path, svg);
      saved.push(path);
      logoFiles.push(path);
    } else {
      const path = await save(l.src, `logo-${i + 1}`);
      if (path) logoFiles.push(path);
    }
  }
  const iconFiles = [];
  for (const [i, ic] of home.icons.slice(0, 4).entries()) {
    const path = await save(ic.href, `icon-${i + 1}`);
    if (path) iconFiles.push({ path, rel: ic.rel, sizes: ic.sizes });
  }
  const ogFile = await save(home.ogImage, 'og-image');

  // The logo's own colours, from its pixels: each saved file drawn on a
  // blank page from a data URL (nothing cross-site to block it).
  const logoColors = {};
  const sampler = await context.newPage();
  for (const file of logoFiles) {
    try {
      const bytes = (await import('node:fs')).readFileSync(file);
      const type = file.endsWith('.svg') ? 'image/svg+xml' : file.endsWith('.png') ? 'image/png' : file.endsWith('.webp') ? 'image/webp' : 'image/jpeg';
      const counts = await sampler.evaluate(async (url) => {
        const img = new Image();
        img.src = url;
        await img.decode();
        const w = 160, h = Math.max(1, Math.round((160 * (img.naturalHeight || 80)) / (img.naturalWidth || 160)));
        const c = document.createElement('canvas');
        c.width = w; c.height = h;
        const g = c.getContext('2d');
        g.drawImage(img, 0, 0, w, h);
        const px = g.getImageData(0, 0, w, h).data;
        const out = {};
        for (let i = 0; i < px.length; i += 4) {
          if (px[i + 3] < 160) continue;
          const q = [px[i], px[i + 1], px[i + 2]].map((v) => Math.min(255, Math.round(v / 6) * 6));
          const k = '#' + q.map((v) => v.toString(16).padStart(2, '0')).join('').toUpperCase();
          out[k] = (out[k] || 0) + 1;
        }
        return out;
      }, `data:${type};base64,${bytes.toString('base64')}`);
      // An SVG says its colours exactly: what the pixels show snaps to them.
      const declared = file.endsWith('.svg') ? Array.from(new Set((bytes.toString('utf8').match(/#[0-9a-fA-F]{6}\b/g) || []).map((h) => h.toUpperCase()))) : [];
      for (const [k, v] of Object.entries(counts)) {
        const exact = declared.find((d) => near(d, k));
        logoColors[exact || k] = (logoColors[exact || k] || 0) + v;
      }
    } catch {}
  }
  await sampler.close();
  // The logo's colours: as much as the brand's own, and said to be the logo's.
  const logoTotal = Object.values(logoColors).reduce((s, v) => s + v, 0) || 1;
  const logoHexes = Object.entries(logoColors).sort((a, b) => b[1] - a[1]).filter(([h]) => { const [r, g, b] = rgb(h); return Math.max(r, g, b) - Math.min(r, g, b) > 24; }).slice(0, 3);
  const pageTotal = merged.reduce((s, c) => s + c.weight, 0) || 1;
  for (const [h, w] of logoHexes) {
    const weight = (pageTotal * 0.02 * w) / logoTotal;
    const into = merged.find((m) => near(m.hex, h));
    if (into) { into.weight += weight; into.where.logo = (into.where.logo || 0) + 1; into.as.logo = (into.as.logo || 0) + weight; }
    else merged.push({ hex: h, weight, as: { logo: weight }, where: { logo: 1 }, pages: new Set([0]) });
  }
  const total = merged.reduce((s, c) => s + c.weight, 0) || 1;
  const colors = merged.map((c) => {
    const share = (100 * c.weight) / total;
    const role = Object.entries(c.as).sort((a, b) => b[1] - a[1])[0][0];
    const where = Object.entries(c.where).sort((a, b) => b[1] - a[1]).slice(0, 4).map(([k]) => k);
    const [r, g, b] = rgb(c.hex);
    const chroma = Math.max(r, g, b) - Math.min(r, g, b);
    const logo = Boolean(c.where.logo);
    return { hex: c.hex, share: Math.round(share * 100) / 100, role: logo ? 'logo' : role, where, pages: c.pages.size, chromatic: chroma > 24, logo, oneOff: !logo && share < 0.4 && c.pages.size === 1 };
  }).filter((c) => c.share >= 0.05 || c.logo).sort((a, b) => b.share - a.share).slice(0, 40);

  // Type: families by how much they set, and the scale on the home page.
  const familyOf = (f) => (f || '').split(',')[0].replace(/["']/g, '').trim();
  const families = {};
  for (const p of pages) for (const t of Object.values(p.type)) families[familyOf(t.family)] = (families[familyOf(t.family)] || 0) + 1;
  const audit = {
    site: start,
    at: new Date().toISOString(),
    pages: pages.map((p) => ({ url: p.url, shot: p.shot, lang: p.lang })),
    colors,
    themeColor: home.themeColor,
    type: home.type,
    families: Object.entries(families).sort((a, b) => b[1] - a[1]).map(([f]) => f).filter(Boolean),
    webfonts: Array.from(new Set(pages.flatMap((p) => p.faces))).slice(0, 30),
    radii: Object.entries(pages.reduce((acc, p) => { for (const [k, v] of Object.entries(p.radii)) acc[k] = (acc[k] || 0) + v; return acc; }, {})).sort((a, b) => b[1] - a[1]).slice(0, 10),
    shadows: Object.entries(pages.reduce((acc, p) => { for (const [k, v] of Object.entries(p.shadows)) acc[k] = (acc[k] || 0) + v; return acc; }, {})).sort((a, b) => b[1] - a[1]).slice(0, 6),
    logos: logoFiles,
    icons: iconFiles,
    ogImage: ogFile,
    voice: { ...home.voice, headlines: Array.from(new Set(pages.flatMap((p) => p.voice.headlines))).slice(0, 14), buttons: Array.from(new Set(pages.flatMap((p) => p.voice.buttons))).slice(0, 16) },
  };
  writeFileSync(join(out, 'brand-audit.json'), JSON.stringify(audit, null, 2));

  // The sheet.
  const esc = (s) => String(s ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  const rel = (p) => (p ? './' + p.split('/').pop() : '');
  const swatch = (c) => `<div class="sw${c.oneOff ? ' off' : ''}"><div class="chip" style="background:${c.hex}"></div><b>${c.hex}</b><span>${c.share}% · ${c.role}</span><span>${esc(c.where.join(', '))}${c.pages > 1 ? ` · ${c.pages} pages` : ''}</span>${c.oneOff ? '<em>one-off?</em>' : ''}</div>`;
  const brand = colors.filter((c) => c.chromatic);
  const neutral = colors.filter((c) => !c.chromatic);
  const typeRows = Object.entries(audit.type).map(([k, t]) => `<tr><td>${k}</td><td style="font-family:${esc(t.family)};font-size:${esc(t.size)};font-weight:${esc(t.weight)};line-height:${esc(t.lineHeight)};letter-spacing:${esc(t.letterSpacing)}">${esc(t.sample)}</td><td>${esc(familyOf(t.family))}<br>${esc(t.size)} / ${esc(t.weight)} / ${esc(t.lineHeight)}</td></tr>`).join('');
  const html = `<!doctype html><html lang="ko"><head><meta charset="utf-8"><title>Brand audit — ${esc(start)}</title><style>
body{margin:0;padding:40px;font-family:Pretendard,"Apple SD Gothic Neo",system-ui,sans-serif;background:#F4F4F5;color:#18181B}
h1{margin:0 0 4px;font-size:26px} .sub{color:#71717A;margin:0 0 20px} h2{font-size:13px;letter-spacing:.08em;text-transform:uppercase;color:#71717A;margin:34px 0 12px}
.shots{display:flex;gap:12px;overflow:hidden} .shots img{width:420px;border-radius:10px;border:1px solid #E4E4E7;background:#fff}
.grid{display:flex;flex-wrap:wrap;gap:10px} .sw{width:150px;background:#fff;border:1px solid #E4E4E7;border-radius:10px;overflow:hidden;font-size:12px;display:flex;flex-direction:column} .sw .chip{height:56px} .sw b,.sw span,.sw em{padding:0 10px} .sw b{padding-top:8px} .sw span{color:#71717A} .sw em{color:#B45309;font-style:normal;padding-bottom:8px} .sw span:last-of-type{padding-bottom:8px} .off{opacity:.6}
table{border-collapse:collapse;width:100%;background:#fff;border-radius:10px;overflow:hidden} td{padding:12px 14px;border-bottom:1px solid #E4E4E7;vertical-align:middle;font-size:13px} td:first-child{color:#71717A;width:80px} td:last-child{color:#71717A;width:220px;font-size:12px}
.logos{display:flex;gap:14px;flex-wrap:wrap} .logos div{background:#fff;border:1px solid #E4E4E7;border-radius:10px;padding:16px;display:flex;align-items:center;justify-content:center;min-width:140px;height:90px} .logos .dark{background:#111} .logos img{max-height:58px;max-width:220px}
ul{margin:0;padding-left:18px;font-size:14px;line-height:1.7} .cols{display:grid;grid-template-columns:repeat(3,1fr);gap:20px} .muted{color:#71717A;font-size:13px}
</style></head><body>
<h1>Brand audit</h1><p class="sub">${esc(start)} · ${pages.length} page(s) · ${esc(audit.at.slice(0, 10))}${audit.themeColor ? ` · theme-color ${esc(audit.themeColor)}` : ''}</p>
<div class="shots">${pages.map((p) => `<img src="${rel(p.shot)}">`).join('')}</div>
<h2>Logo</h2><div class="logos">${[...logoFiles, ...iconFiles.map((i) => i.path), ogFile].filter(Boolean).map((f) => `<div><img src="${rel(f)}"></div>`).join('') || '<span class="muted">No logo found in the header — ask for the files.</span>'}${logoFiles[0] ? `<div class="dark"><img src="${rel(logoFiles[0])}"></div>` : ''}</div>
<h2>Brand colours (by area)</h2><div class="grid">${brand.map(swatch).join('') || '<span class="muted">No chromatic colour: a neutral brand, or one carried by pictures.</span>'}</div>
<h2>Neutrals</h2><div class="grid">${neutral.map(swatch).join('')}</div>
<h2>Type</h2><p class="muted">Families: ${esc(audit.families.join(', '))} · Webfonts loaded: ${esc(audit.webfonts.slice(0, 8).join(', ') || 'none')}</p><table>${typeRows}</table>
<h2>Shape</h2><p class="muted">Radii: ${esc(audit.radii.map(([k, v]) => `${k} ×${v}`).join(' · ') || 'square')}</p><p class="muted">Shadows: ${esc(audit.shadows.map(([k, v]) => `${k} ×${v}`).join(' · ') || 'none')}</p>
<h2>Voice</h2><div class="cols"><div><b>Headlines</b><ul>${audit.voice.headlines.map((t) => `<li>${esc(t)}</li>`).join('')}</ul></div><div><b>Buttons</b><ul>${audit.voice.buttons.map((t) => `<li>${esc(t)}</li>`).join('')}</ul></div><div><b>Title &amp; description</b><ul><li>${esc(audit.voice.title)}</li>${audit.voice.description ? `<li>${esc(audit.voice.description)}</li>` : ''}</ul><b>Navigation</b><ul>${audit.voice.nav.map((t) => `<li>${esc(t)}</li>`).join('')}</ul></div></div>
</body></html>`;
  const sheet = join(out, 'brand-audit.html');
  writeFileSync(sheet, html);
  const sheetPage = await context.newPage();
  await sheetPage.goto(pathToFileURL(sheet).href, { waitUntil: 'networkidle' }).catch(() => {});
  await sheetPage.screenshot({ path: join(out, 'brand-audit.png'), fullPage: true });

  // What to read.
  console.log(`Brand audit of ${start} — ${pages.length} page(s)`);
  console.log(`Colours (share by area · role · where):`);
  for (const c of colors.slice(0, 14)) console.log(`  ${c.hex}  ${String(c.share).padStart(6)}%  ${c.role.padEnd(6)} ${c.where.join(', ')}${c.oneOff ? '  (one-off?)' : ''}`);
  console.log(`Families: ${audit.families.join(', ') || '—'}`);
  console.log(`Webfonts: ${audit.webfonts.slice(0, 8).join(', ') || '—'}`);
  for (const [k, t] of Object.entries(audit.type)) console.log(`  ${k.padEnd(8)} ${familyOf(t.family)} ${t.size} / ${t.weight} / ${t.lineHeight}`);
  console.log(`Logo files: ${logoFiles.join(', ') || 'none found — ask the person for them'}`);
  console.log(`Voice: ${audit.voice.headlines.slice(0, 5).join(' | ')}`);
  console.log(`Written: ${join(out, 'brand-audit.png')} (look at it), brand-audit.html, brand-audit.json, page-*.png${saved.length ? ', ' + saved.length + ' asset file(s)' : ''}`);
} finally {
  await browser.close();
}
