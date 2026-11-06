#!/usr/bin/env node
// Fill a card with data, get one self-contained HTML file (no script, no font or network request).
//   node card.mjs <board|compare|figure> <data.json> [--mode auto|light|dark] > card.html
// Every string in the data is HTML-escaped here; the page carries a CSP that blocks everything but its own inline style.
import fs from 'node:fs'; import path from 'node:path'; import { fileURLToPath } from 'node:url';
const HERE = path.dirname(fileURLToPath(import.meta.url));
const read = (f) => fs.readFileSync(path.join(HERE, f), 'utf8');

const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
// status → [css class, symbol, default word]. Symbol + word always travel together.
const ST = {
  work: ['work', '●', 'In progress'], next: ['', '○', 'Next'], wait: ['wait', '◆', 'Your turn'], ok: ['ok', '✓', 'Done'], err: ['err', '✕', 'Blocked'],
};
const badge = (kind, text) => { const [c, sym, word] = ST[kind]; return `<span class="badge ${c}"><span aria-hidden="true">${sym}</span>${esc(text ?? word)}</span>`; };
const pickBadge = '<span class="badge accent"><span aria-hidden="true">★</span>Recommended</span>';
const foot = (d) => (d.asOf ? `<p class="vc-foot">As of ${esc(d.asOf)}${d.source ? ` · ${esc(d.source)}` : ''}</p>` : '');
const head = (d) => `<div class="vc-head"><h2 class="vc-title">${esc(d.title)}</h2>${d.subtitle ? `<p class="vc-meta">${esc(d.subtitle)}</p>` : ''}</div>`;

// ---- board: { title, subtitle?, asOf, source?, projects:[{name, work:[item], next:[item], wait:[item]}] }  item: { id?, title, who?, note? }
function board(d) {
  const proj = (p) => {
    const row = (kind, items) => `<div class="bd-row"><div>${badge(kind)}</div>${items?.length
      ? `<ul class="bd-items">${items.map((i) => `<li class="bd-item">${i.id ? `<span class="mono">${esc(i.id)}</span>` : ''}<span>${esc(i.title)}</span>${i.who ? `<span class="who">· ${esc(i.who)}</span>` : ''}${i.note ? `<span class="bd-note">${esc(i.note)}</span>` : ''}</li>`).join('')}</ul>`
      : '<p class="bd-none">None</p>'}</div>`;
    const n = (k) => (p[k] || []).length;
    return `<section class="bd-proj"><div class="bd-ph"><h3 class="bd-pn">${esc(p.name)}</h3><span class="bd-counts">${n('work')} in progress · ${n('next')} next · ${n('wait')} your turn</span></div>${row('work', p.work)}${row('next', p.next)}${row('wait', p.wait)}</section>`;
  };
  return `${head(d)}${(d.projects || []).map(proj).join('')}${foot(d)}`;
}

// ---- compare: { title, subtitle?, options:[{name, pick?:true}], rows:[{label, values:[string]}], why?, asOf? }
function compare(d) {
  const o = d.options || [];
  const cell = (v, i) => `<div class="cp-c${o[i]?.pick ? ' pick' : ''}"><span class="lab">${esc(o[i]?.name)}${o[i]?.pick ? ' ★ Recommended' : ''}</span>${esc(v)}</div>`;
  const heads = `<div class="cp-r cp-h" style="--n:${o.length}"><div class="cp-c k"></div>${o.map((x) => `<div class="cp-c${x.pick ? ' pick' : ''}">${esc(x.name)}${x.pick ? pickBadge : ''}</div>`).join('')}</div>`;
  const rows = (d.rows || []).map((r) => `<div class="cp-r" style="--n:${o.length}"><div class="cp-c k">${esc(r.label)}</div>${r.values.map(cell).join('')}</div>`).join('');
  const pick = o.find((x) => x.pick);
  return `${head(d)}<div class="cp" role="table" aria-label="${esc(d.title)}">${heads}${rows}</div>${d.why ? `<p class="cp-why"><b>${esc(pick ? `${pick.name} recommended` : 'Recommended')}</b> · ${esc(d.why)}</p>` : ''}${foot(d)}`;
}

// ---- figure: { title, subtitle?, asOf?, source?, stats:[{label, value, unit?, delta?, deltaTone?:'good'|'bad'|'flat', points?:[number], from?, to?}] }
function spark(pts, label) {
  const W = 200, H = 48, p = 4, min = Math.min(...pts), max = Math.max(...pts), span = max - min || 1;
  const xy = pts.map((v, i) => [p + (i * (W - 2 * p)) / Math.max(pts.length - 1, 1), H - p - ((v - min) / span) * (H - 2 * p)]);
  const line = xy.map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`).join(' ');
  const last = xy[xy.length - 1];
  return `<svg class="spark" viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" role="img" aria-label="${esc(label)}"><line class="base" x1="0" x2="${W}" y1="${H - 1}" y2="${H - 1}"/><polygon class="ar" points="${p},${H - 1} ${line} ${last[0].toFixed(1)},${H - 1}"/><polyline class="ln" points="${line}" vector-effect="non-scaling-stroke"/></svg>`;
}
function figure(d) {
  const s = d.stats || [];
  const tile = (x) => {
    const tone = ['good', 'bad', 'flat'].includes(x.deltaTone) ? x.deltaTone : 'flat';
    const sym = { good: '▲', bad: '▼', flat: '–' }[tone];
    return `<div class="fg-tile"><p class="fg-label">${esc(x.label)}</p><div class="fg-val"><span class="fg-num">${esc(x.value)}</span>${x.unit ? `<span class="fg-unit">${esc(x.unit)}</span>` : ''}${x.delta !== undefined ? `<span class="fg-delta ${tone}"><span aria-hidden="true">${sym}</span> ${esc(x.delta)}</span>` : ''}</div>${x.points?.length > 1 ? spark(x.points, `${x.label} trend: ${x.points.join(', ')}`) + `<div class="fg-range"><span>${esc(x.from ?? '')}</span><span>${esc(x.to ?? '')}</span></div>` : ''}</div>`;
  };
  return `${head(d)}<div class="fg n${Math.min(s.length, 3)}">${s.map(tile).join('')}</div>${foot(d)}`;
}

export const TYPES = { board, compare, figure };
export function render(type, data, mode = 'auto') {
  if (!TYPES[type]) throw new Error(`type must be one of ${Object.keys(TYPES).join(', ')}`);
  const theme = mode === 'light' || mode === 'dark' ? ` data-theme="${mode}"` : '';
  const css = read('../../build/css/tokens.css') + '\n' + read('card.css');
  return `<!doctype html><html lang="${esc(data.lang ?? 'en')}"${theme}><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data:"><title>${esc(data.title)}</title><style>${css}</style></head><body><main class="vc"><article class="vc-card">${TYPES[type](data)}</article></main></body></html>`;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [type, file, ...rest] = process.argv.slice(2);
  const i = rest.indexOf('--mode');
  if (!type || !file) { console.error('usage: node card.mjs <board|compare|figure> <data.json> [--mode auto|light|dark]'); process.exit(2); }
  process.stdout.write(render(type, JSON.parse(fs.readFileSync(file, 'utf8')), i >= 0 ? rest[i + 1] : 'auto'));
}
