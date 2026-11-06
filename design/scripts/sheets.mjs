// npm run sheets — renders one PNG per component and mode (components/*.png) with headless Chrome,
// from build/css/tokens.css + reference/components.css. If a token changes, the sheets change.
import fs from 'node:fs'; import path from 'node:path'; import { execFileSync } from 'node:child_process'; import { ROOT } from './lib.mjs';

const CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const read = (f) => fs.readFileSync(path.join(ROOT, f), 'utf8');
const tokens = read('build/css/tokens.css'), comps = read('reference/components.css');
const SCALE = 1; // 1400 px wide sheets

const shellCss = `
body { margin: 0; background: var(--eq-color-bg); color: var(--eq-color-text); font: var(--eq-type-ui); -webkit-font-smoothing: antialiased; }
.page { width: 1400px; box-sizing: border-box; padding: 48px 56px 24px; }
h1 { font: 700 34px/1.2 var(--eq-font-family-sans); letter-spacing: -0.02em; margin: 0 0 6px; }
.sub { color: var(--eq-color-muted); font: var(--eq-type-small); margin: 0 0 32px; max-width: 980px; }
.grid { display: grid; gap: 20px 24px; align-items: center; margin-bottom: 36px; }
.grid > .h { font: 600 12px/1 var(--eq-font-family-sans); letter-spacing: 0.08em; text-transform: uppercase; color: var(--eq-color-muted); padding-bottom: 4px; border-bottom: 1px solid var(--eq-color-line); }
.grid > .rl { font: var(--eq-type-small); color: var(--eq-color-muted); }
.note { font: var(--eq-type-small); color: var(--eq-color-muted); max-width: 1100px; margin: 0 0 12px; }
.note b { color: var(--eq-color-text); }
.grid > .h:empty { border-bottom: 0; }
code { font: var(--eq-type-code); font-size: 12px; background: var(--eq-color-sunken); padding: 1px 6px; border-radius: var(--eq-radius-code); }
.sw { display: flex; align-items: center; gap: 12px; font: var(--eq-type-small); }
.sw i { width: 44px; height: 44px; border-radius: var(--eq-radius-md); border: 1px solid var(--eq-color-line); flex: none; }
.sw b { display: block; font-weight: 600; } .sw span { color: var(--eq-color-muted); font: 12px var(--eq-font-family-mono); }
.frame { position: relative; height: 380px; border-radius: var(--eq-radius-lg); overflow: hidden; border: 1px solid var(--eq-color-line); }
.frame .under { position: absolute; inset: 0; padding: 20px; color: var(--eq-color-muted); font: var(--eq-type-small); background: var(--eq-color-bg); }
.frame .eq-scrim { position: absolute; inset: 0; padding: 24px; }
.frame .eq-scrim--bottom { padding: 24px 24px 0; }
`;
const fonts = `<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;700&family=JetBrains+Mono:wght@400&display=swap"><link rel="stylesheet" href="https://cdn.jsdelivr.net/gh/orioncactus/pretendard@v1.3.9/dist/web/variable/pretendardvariable.min.css">`;

const cols = (n, first = 150) => `grid-template-columns: ${first}px repeat(${n}, 1fr)`;
const row = (label, cells) => `<div class="rl">${label}</div>${cells.map((c) => `<div>${c}</div>`).join('')}`;
const heads = (hs) => `<div class="h"></div>${hs.map((h) => `<div class="h">${h}</div>`).join('')}`;
const STATES = ['', 'is-hover', 'is-pressed', 'is-focus', 'is-disabled'];
const STATE_H = ['default', 'hover', 'pressed', 'focus', 'disabled'];

const btn = (v, label, st, extra = '') => `<button class="eq-btn eq-btn--${v} eq-anim eq-focusable ${v === 'primary' || v === 'danger-soft' ? '' : 'eq-overlay-hover'} ${st} ${extra}">${label}</button>`;

const sheets = {
  button: () => ({
    title: 'Button', sub: 'Height `button.height` (web 44 · desktop 28 · iOS 44 · Android 48). The primary button is `color.action` (a text-coloured fill); a brand-blue fill is for web marketing only. Only the primary and danger-soft buttons have their own hover·pressed colours (danger-soft is opaque `err-fill`, safe on translucent bars); the rest use the neutral overlay (`color.hover/pressed`). The danger button is only for what can\'t be undone.',
    body: `<div class="grid" style="${cols(5)}">${heads(STATE_H)}
      ${row('primary', STATES.map((s) => btn('primary', 'Allow', s)))}
      ${row('secondary', STATES.map((s) => btn('secondary', 'Decline', s)))}
      ${row('ghost', STATES.map((s) => btn('ghost', 'See details', s)))}
      ${row('danger', STATES.map((s) => btn('danger', 'Remove device', s)))}
      ${row('danger-soft', STATES.map((s) => btn('danger-soft', 'Deny', s)))}
      ${row('brand<br>(web marketing)', STATES.map((s) => btn('brand', 'Get the macOS app', s)))}
    </div>
    <div class="grid" style="${cols(4)}">${heads(['size', '', '', ''])}
      ${row('md (default)', [btn('primary', 'Allow', ''), btn('secondary', 'Decline', ''), '', ''])}
      ${row('sm', [btn('primary', 'Allow', '', 'eq-btn--sm'), btn('secondary', 'Decline', '', 'eq-btn--sm'), btn('ghost', 'More', '', 'eq-btn--sm'), ''])}
    </div>`,
  }),
  input: () => ({
    title: 'Input', sub: 'The border is `color.border-strong` (3:1 or more) — not the hairline (`line`). Focus is a 2px border + the ring colour `color.focus`. An error is told by the line below too, not by colour alone. The placeholder is `muted` (4.5:1 or more) and never stands in for the label.',
    body: `<div class="grid" style="${cols(5)}">${heads(['default', 'hover', 'focus', 'error', 'disabled'])}
      ${row('empty', [
        `<div class="eq-field"><label>Project name</label><input class="eq-input eq-anim" placeholder="e.g. equipe-site"><p class="eq-help">You can change it later.</p></div>`,
        `<div class="eq-field"><label>Project name</label><input class="eq-input eq-anim is-hover" placeholder="e.g. equipe-site"><p class="eq-help">You can change it later.</p></div>`,
        `<div class="eq-field"><label>Project name</label><input class="eq-input eq-anim is-focus" placeholder="e.g. equipe-site"><p class="eq-help">You can change it later.</p></div>`,
        `<div class="eq-field"><label>Project name</label><input class="eq-input eq-anim is-error" placeholder="e.g. equipe-site"><p class="eq-help is-error">✕ Enter a name.</p></div>`,
        `<div class="eq-field"><label>Project name</label><input class="eq-input eq-anim" disabled placeholder="e.g. equipe-site"><p class="eq-help">It can't be changed right now.</p></div>`])}
      ${row('filled', ['default', 'hover', 'focus', 'error', 'disabled'].map((_, i) => `<div class="eq-field"><label>Pairing code</label><input class="eq-input eq-anim ${['', 'is-hover', 'is-focus', 'is-error', ''][i]}" value="K7QD-92LM" ${i === 4 ? 'disabled' : ''}>${i === 3 ? '<p class="eq-help is-error">✕ That code doesn\'t match.</p>' : ''}</div>`))}
    </div>`,
  }),
  card: () => ({
    title: 'Card', sub: 'The default card is `surface` + a hairline. The floating card (`raised`) has the shadow `elevation.float` — only for what floats above, like popovers and menus. A card that can be pressed lays the neutral overlay over it on hover·press. No cards inside cards.',
    body: `<div class="grid" style="${cols(5)}">${heads(STATE_H)}
      ${row('card that opens', STATES.map((s) => `<div class="eq-card eq-card--link eq-anim eq-focusable eq-overlay-hover ${s}" tabindex="-1"><h3>equipe-site</h3><p>The designer posted a token draft.</p><span class="meta">2 min ago · 3 tickets</span></div>`))}
      ${row('floating card<br>(raised)', STATES.slice(0, 1).map((s) => `<div class="eq-card eq-card--raised"><h3>Copied</h3><p>The link is on the clipboard.</p></div>`).concat(['', '', '', '']))}
    </div>`,
  }),
  chip: () => ({
    title: 'Chip', sub: 'A pill for filtering·choosing. Selected shows a `✓` too, not colour alone. Height `size.chip` (web·mobile 32 · desktop 24); on mobile the touch area grows to 44/48 (the visible size stays). A status is a Badge, not a chip.',
    body: `<div class="grid" style="${cols(5)}">${heads(STATE_H)}
      ${row('default', STATES.map((s) => `<span class="eq-chip eq-anim eq-focusable eq-overlay-hover ${s}">All</span>`))}
      ${row('selected', STATES.map((s) => `<span class="eq-chip eq-anim eq-focusable eq-overlay-hover is-selected ${s}">✓ equipe-site</span>`))}
    </div>`,
  }),
  badge: () => ({
    title: 'Badge', sub: 'A status uses colour + symbol + words together (never colour alone). Meanings: done ok · working work (amber) · your turn wait (orange, waiting on the person) · error err. Amber/orange are never project colours. It only shows, nothing happens on a press, so it has no hover·pressed states.',
    body: `<div class="grid" style="${cols(5)}">${heads(['done ok', 'working work', 'your turn wait', 'error err', 'neutral'])}
      ${['on page (bg)', 'on card (surface)'].map((l, i) => row(l, [
        `<span class="eq-badge eq-badge--ok">✓ Done</span>`, `<span class="eq-badge eq-badge--work">⋯ Working</span>`, `<span class="eq-badge eq-badge--wait">! Your turn</span>`, `<span class="eq-badge eq-badge--err">✕ Error</span>`, `<span class="eq-badge">Draft</span>`].map((c) => i ? `<div style="background:var(--eq-color-surface);padding:14px 16px;border-radius:var(--eq-radius-md);display:inline-block">${c}</div>` : c))).join('')}
    </div>`,
  }),
  row: () => ({
    title: 'List row', sub: 'Row height `size.row` (web 48 · desktop 32 · iOS 44 · Android 56). Dividers are `line`; selected is `accent-bg` + text (never an accent-coloured line alone). The whole row is the press area.',
    body: `<div class="grid" style="grid-template-columns: 150px 640px">${heads(['row (web)'])}
      ${[['default', ''], ['hover', 'is-hover'], ['pressed', 'is-pressed'], ['selected', 'is-selected'], ['focus', 'is-focus'], ['disabled', 'is-disabled']].map(([l, s], i) => `<div class="rl">${l}</div><div style="background:var(--eq-color-surface);border-radius:var(--eq-radius-card);padding:0 var(--eq-space-2)"><div class="eq-row eq-anim eq-focusable eq-overlay-hover ${s}"><span style="width:var(--eq-space-3);height:var(--eq-space-3);border-radius:var(--eq-radius-full);background:var(--eq-color-work);flex:none"></span><span class="grow"><span>designer</span><span class="secondary">Tidying the token draft</span></span><span class="eq-badge eq-badge--wait">! Your turn</span><span class="secondary" style="color:var(--eq-color-muted)">›</span></div></div>`).join('')}
    </div>
    <p class="note">The leading dot is the <code>work</code> colour, but <b>the words beside it (status badge·secondary line) carry the meaning too</b>. On a real screen a member's avatar goes there.</p>`,
  }),
  sheet: () => ({
    title: 'Sheet / Modal', sub: 'Web·desktop: a centred dialog (max 480); mobile: a sheet rising from the bottom (grabber 36×4). Behind it, `scrim`. There is always a way to close it (cancel·tap outside·Esc), and only the one risky action is emphasised. Motion: dialog `motion.duration.slow` (200ms) + 8 of travel; with \'Reduce motion\', a 100ms fade with no travel.',
    body: `<div style="display:grid;grid-template-columns: 1fr 1fr;gap:28px">
      <div><div class="note"><b>Dialog (web · desktop)</b></div><div class="frame"><div class="under">Settings · Devices<br>list …</div><div class="eq-scrim"><div class="eq-sheet" role="dialog"><h3>Remove this device?</h3><p>The phone 'iPhone 15' is disconnected; to use it again you'll pair it anew.</p><div class="eq-actions">${btn('secondary', 'Cancel', '')}${btn('danger', 'Remove', '')}</div></div></div></div></div>
      <div><div class="note"><b>Bottom sheet (iOS · Android)</b></div><div class="frame"><div class="under">Devices<br>list …</div><div class="eq-scrim eq-scrim--bottom"><div class="eq-sheet eq-sheet--bottom"><div class="grabber"></div><h3>Remove this device?</h3><p>It is disconnected; to use it again you'll pair it anew.</p><div class="eq-actions" style="flex-direction:column">${btn('danger', 'Remove', '')}${btn('secondary', 'Cancel', '')}<div style="height:var(--eq-space-2)"></div></div></div></div></div></div>
    </div>`,
  }),
  decision: () => ({
    title: 'Decision card', sub: 'The moment the person\'s approval is needed — the component that must show most in the product. A 2px `wait` (your turn) border and a "Your turn" badge announce it; who·where·what they want to do comes first, and the two actions (decline·allow) sit side by side at the same width. Allow is primary, decline secondary. It says in words that it holds only once.',
    body: `<div style="display:grid;grid-template-columns: 1fr 1fr;gap:28px 28px">
      ${[
        ['default', '', ''],
        ['focus (allow)', '', 'f'],
        ['processing', '', 'p'],
        ['resolved', 'r', ''],
      ].map(([l, k, o]) => `<div><div class="note"><b>${l}</b></div><div class="eq-decision ${k === 'r' ? 'is-resolved' : ''}">
        <div style="display:flex;justify-content:space-between;align-items:center">${k === 'r' ? '<span class="eq-badge eq-badge--ok">✓ Allowed</span>' : '<span class="eq-badge eq-badge--wait">! Your turn</span>'}<span class="eq-help">just now</span></div>
        <p class="who">frontend wants to write a file</p><p class="where">equipe-site · src/styles/global.css</p>
        <pre>write src/styles/global.css  (+18 −4)</pre>
        <p class="eq-help">Allowed this once. Doing the same again asks again.</p>
        ${k === 'r' ? '' : `<div class="eq-actions">${btn('secondary', 'Decline', o === 'p' ? 'is-disabled' : '')}${btn('primary', o === 'p' ? 'Sending…' : 'Allow once', o === 'f' ? 'is-focus' : o === 'p' ? 'is-disabled' : '')}</div>`}
      </div></div>`).join('')}
    </div>`,
  }),
  toast: () => ({
    title: 'Toast', sub: 'For a short confirmation — only what is no loss if it goes. On the page\'s opposite colour (`inverse`), so it reads over any screen. It goes after 5 seconds (`motion.duration.toast`) and holds while hovered·focused. With an action, keep it from going or leave another way (settings·history). Screen readers read it as `role="status"`. Errors go in a banner/field error, not a toast.',
    body: `<div class="grid" style="${cols(4)}">${heads(['default', 'with action', 'action hover', 'action focus'])}
      ${row('on page (bg)', [
        `<div class="eq-toast" role="status"><span>✓</span><span>Link copied</span></div>`,
        `<div class="eq-toast"><span>✓</span><span>Ticket sent</span><button class="eq-toast-action">Undo</button></div>`,
        `<div class="eq-toast"><span>✓</span><span>Ticket sent</span><button class="eq-toast-action is-hover">Undo</button></div>`,
        `<div class="eq-toast"><span>✓</span><span>Ticket sent</span><button class="eq-toast-action is-focus">Undo</button></div>`])}
      ${row('on card (surface)', ['', '', '', ''].map((_, i) => `<div style="background:var(--eq-color-surface);padding:20px;border-radius:var(--eq-radius-card)"><div class="eq-toast"><span>✓</span><span>${i ? 'Ticket sent' : 'Link copied'}</span>${i ? `<button class="eq-toast-action ${['', 'is-hover', 'is-focus'][i - 1]}">Undo</button>` : ''}</div></div>`))}
    </div>`,
  }),
  foundations: () => {
    const groups = {
      'Backgrounds·surfaces': ['bg', 'surface', 'raised', 'sunken'], 'Text': ['text', 'muted'], 'Accent': ['accent', 'on-accent', 'accent-bg', 'focus', 'dot'],
      'Action': ['action', 'on-action', 'action-hover', 'action-pressed', 'hover', 'pressed', 'border-strong', 'line', 'scrim'], 'Status': ['ok', 'work', 'wait', 'err', 'ok-bg', 'work-bg', 'wait-bg', 'err-bg', 'on-err'], 'Inverse': ['inverse', 'on-inverse', 'inverse-accent'],
    };
    const sw = (n) => `<div class="sw"><i style="background:var(--eq-color-${n})"></i><div><b>${n}</b><span>--eq-color-${n}</span></div></div>`;
    const types = ['display', 'headline', 'title', 'body', 'ui', 'label', 'small', 'meta', 'code'];
    return { title: 'Foundations', sub: 'Colour roles·type roles·spacing·radius·shadow·motion. Every value comes from `tokens/*.json` → `build/css/tokens.css`. This sheet shows web values (apps: the platform table in `components.md`).',
      body: `${Object.entries(groups).map(([g, ns]) => `<p class="note"><b>${g}</b></p><div style="display:grid;grid-template-columns:repeat(4,1fr);gap:14px 24px;margin-bottom:26px">${ns.map(sw).join('')}</div>`).join('')}
      <p class="note"><b>type roles</b> (web &gt;640px)</p><div style="margin-bottom:26px">${types.map((t) => `<div style="display:flex;gap:24px;align-items:baseline;margin-bottom:6px"><code style="width:90px;text-align:center">${t}</code><span style="font:var(--eq-type-${t});letter-spacing:var(--eq-type-${t}-tracking)">${t === 'code' ? 'equipe pair --qr' : 'The person who leads approves · You approve every merge'}</span></div>`).join('')}</div>
      <p class="note"><b>space</b> · <b>radius</b> · <b>elevation</b></p>
      <div style="display:flex;gap:40px;align-items:flex-end;margin-bottom:26px;flex-wrap:wrap">
        <div style="display:flex;gap:14px;align-items:flex-end">${[1, 2, 3, 4, 5, 6, 7].map((n) => `<div style="text-align:center"><div style="width:var(--eq-space-${n});height:var(--eq-space-${n});background:var(--eq-color-accent);border-radius:2px"></div><code>${n}</code></div>`).join('')}</div>
        <div style="display:flex;gap:14px;align-items:flex-end">${['sm', 'md', 'lg', 'xl', 'xxl', 'full'].map((n) => `<div style="text-align:center"><div style="width:64px;height:64px;border:2px solid var(--eq-color-text);border-radius:var(--eq-radius-${n})"></div><code>${n}</code></div>`).join('')}</div>
        <div style="display:flex;gap:26px">${['float', 'overlay'].map((n) => `<div style="text-align:center"><div style="width:120px;height:72px;background:var(--eq-color-raised);border-radius:var(--eq-radius-card);box-shadow:var(--eq-elevation-${n})"></div><code>${n}</code></div>`).join('')}</div>
      </div>
      <p class="note"><b>motion</b> — one curve (<code>out-cubic</code>), three durations. With 'Reduce motion', no travel, only a 100ms fade.</p>
      <table style="border-collapse:collapse;font:var(--eq-type-small);margin-bottom:12px">${[['fast 120ms', 'hover fill · press · tooltip'], ['base 160ms', 'popover · menu · chip'], ['slow 200ms', 'dialog · sheet · card/message entrance'], ['slide 180ms', 'sidebar·panel (out-quint)'], ['toast 5000ms', 'how long a toast stays (not an animation)'], ['travel 4 · 6 · 8', 'pop · rise · dialog travel distance']].map(([a, b]) => `<tr><td style="padding:3px 24px 3px 0"><code>${a}</code></td><td style="color:var(--eq-color-muted)">${b}</td></tr>`).join('')}</table>`,
    };
  },
};

fs.mkdirSync(path.join(ROOT, 'build/sheets'), { recursive: true });
const only = process.argv[2];
for (const [name, fn] of Object.entries(sheets)) {
  if (only && only !== name) continue;
  const s = fn();
  for (const mode of ['light', 'dark']) {
    const html = `<!doctype html><html lang="en" data-theme="${mode}"><head><meta charset="utf-8">${fonts}<style>${tokens}\n${comps}\n${shellCss}</style></head><body><div class="page"><h1>${s.title} <span style="font-weight:400;color:var(--eq-color-muted);font-size:20px">· ${mode === 'light' ? 'Light' : 'Dark'}</span></h1><p class="sub">${s.sub.replace(/`([^`]+)`/g, '<code>$1</code>')}</p>${s.body}</div></body></html>`;
    const hf = path.join(ROOT, `build/sheets/${name}-${mode}.html`); fs.writeFileSync(hf, html);
    const raw = `/tmp/eq-${name}-${mode}.png`, out = path.join(ROOT, `components/${name}-${mode}.png`);
    execFileSync(CHROME, ['--headless=new', '--hide-scrollbars', '--force-device-scale-factor=2', '--window-size=1400,2400', '--virtual-time-budget=9000', `--screenshot=${raw}`, 'file://' + hf], { stdio: 'ignore' });
    execFileSync('python3', [path.join(ROOT, 'scripts/crop.py'), raw, out]);
    console.log('sheet', name, mode);
  }
}
