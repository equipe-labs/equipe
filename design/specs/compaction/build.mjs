// Renders the conversation-summary (compaction) mockups — node build.mjs  → *.png (HTML in /tmp). Tokens come only from ../../build/css/tokens.css.
// Plain layout numbers follow the space token values (4·8·12·16·24). The one new thing needed is a loop motion duration (proposed: motion.duration.loop = 1600ms).
import fs from 'node:fs'; import path from 'node:path'; import { execFileSync } from 'node:child_process'; import { fileURLToPath } from 'node:url';
const HERE = path.dirname(fileURLToPath(import.meta.url));
const tokens = fs.readFileSync(path.join(HERE, '../../build/css/tokens.css'), 'utf8').replace(/:root\[data-theme="(dark|light)"\]/g, '[data-theme="$1"]'); // lifts the :root scoping so light and dark can show side by side on one page
const CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const fonts = `<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;700&family=JetBrains+Mono:wght@400&display=swap"><link rel="stylesheet" href="https://cdn.jsdelivr.net/gh/orioncactus/pretendard@v1.3.9/dist/web/variable/pretendardvariable.min.css">`;
const css = `
*{box-sizing:border-box} body{margin:0;background:var(--eq-color-bg);color:var(--eq-color-text);font:var(--eq-type-ui);-webkit-font-smoothing:antialiased;word-break:keep-all;overflow-wrap:anywhere;font-variant-numeric:tabular-nums}
body,button{font-family:var(--eq-font-family-sans)}
.ico{width:var(--eq-size-icon-md);height:var(--eq-size-icon-md);flex:none}
.muted{color:var(--eq-color-muted)}
/* summary band */
.cz{display:flex;gap:var(--eq-space-3);align-items:flex-start;padding:var(--eq-space-3) var(--eq-space-4);background:var(--eq-color-surface);border:1px solid var(--eq-color-line);border-radius:var(--eq-radius-card)}
.cz .main{flex:1;min-width:0}
.cz .t{font:var(--eq-type-small);font-weight:600;color:var(--eq-color-text)}
.cz .s{font:var(--eq-type-meta);color:var(--eq-color-muted);margin-top:2px}
.cz .ico{color:var(--eq-color-work);margin-top:1px}
.cz.ok .ico{color:var(--eq-color-ok)} .cz.err .ico{color:var(--eq-color-err)}
.bar{position:relative;height:4px;border-radius:var(--eq-radius-full);background:var(--eq-color-sunken);overflow:hidden;flex:1}
.bar i{position:absolute;inset:0 auto 0 0;background:var(--eq-color-work);border-radius:inherit}
.bar.ind i{left:20%;width:32%}
.bar.ind.anim i{animation:ind 1600ms linear infinite;left:0}
@keyframes ind{from{transform:translateX(-100%)}to{transform:translateX(320%)}}
@media (prefers-reduced-motion:reduce){.bar.ind.anim i{animation:none;left:20%}}
.row2{display:flex;align-items:center;gap:var(--eq-space-3);margin-top:var(--eq-space-2)}
.pct{font:var(--eq-type-small);color:var(--eq-color-muted);min-width:36px;text-align:right}
.btn{font:var(--eq-type-label);font-size:14px;height:28px;padding:0 var(--eq-space-3);border-radius:var(--eq-radius-control);border:1px solid var(--eq-color-border-strong);background:none;color:var(--eq-color-text)}
.queued{display:inline-flex;gap:var(--eq-space-2);align-items:center;margin-top:var(--eq-space-2);font:var(--eq-type-meta);color:var(--eq-color-text);background:var(--eq-color-sunken);border-radius:var(--eq-radius-full);padding:2px var(--eq-space-3)}
/* conversation */
.chat{display:flex;flex-direction:column;gap:var(--eq-space-3);font:var(--eq-type-small)}
.u{align-self:flex-end;max-width:80%;background:var(--eq-color-sunken);border-radius:var(--eq-radius-lg);padding:var(--eq-space-2) var(--eq-space-3)}
.m{max-width:88%;line-height:1.6}
.who{font:var(--eq-type-meta);font-weight:600;color:var(--eq-color-muted);margin-bottom:2px}
.divider{display:flex;align-items:center;gap:var(--eq-space-3);font:var(--eq-type-meta);color:var(--eq-color-muted)}
.divider:before,.divider:after{content:"";flex:1;height:1px;background:var(--eq-color-line)}
.divider b{font-weight:600;display:inline-flex;gap:6px;align-items:center}
.inl{display:flex;gap:var(--eq-space-2);align-items:center;font:var(--eq-type-small);color:var(--eq-color-muted)}
.inl .ico{color:var(--eq-color-work);width:var(--eq-size-icon-sm);height:var(--eq-size-icon-sm)}
.composer{display:flex;align-items:center;gap:var(--eq-space-2);border:var(--eq-input-border-width) solid var(--eq-input-border);border-radius:var(--eq-radius-card);background:var(--eq-color-surface);padding:var(--eq-space-2) var(--eq-space-3);min-height:var(--eq-size-field);font:var(--eq-type-small);color:var(--eq-color-muted)}
.composer .send{margin-left:auto;width:28px;height:28px;border-radius:var(--eq-radius-full);background:var(--eq-color-action);color:var(--eq-color-on-action);display:grid;place-items:center;font-size:14px}
.spin{width:var(--eq-size-icon-sm);height:var(--eq-size-icon-sm);border-radius:50%;border:2px solid var(--eq-color-sunken);border-top-color:var(--eq-color-work);flex:none}
.spin.anim{animation:sp 1000ms linear infinite} @keyframes sp{to{transform:rotate(360deg)}}
.dot{width:8px;height:8px;border-radius:50%;background:var(--eq-color-ok);flex:none}
`;
const I = {
  compress: `<svg class="ico" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 3.5l4 4 4-4M6 16.5l4-4 4 4M4 10h12"/></svg>`,
  check: `<svg class="ico" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="10" cy="10" r="7.5"/><path d="m6.8 10.2 2.2 2.2 4.2-4.6"/></svg>`,
  x: `<svg class="ico" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><circle cx="10" cy="10" r="7.5"/><path d="m7.5 7.5 5 5m0-5-5 5"/></svg>`,
};
const T = {
  auto: ['Summarizing the conversation', 'Tidying up a long conversation. You can continue when it\'s done.'],
  manual: ['Summarizing to start fresh', 'Carrying a summary into the next conversation.'],
  long: ['Summarizing is taking a while', 'The conversation is long and needs more time. Hang tight.'],
  done: ['Summarized', 'You can keep chatting.'],
  fail: ['Couldn\'t summarize', 'The conversation is unchanged. Try again?'],
};
const el = (s) => s;
// kind: 'est' | 'ind' | 'line'; st: auto|manual|long|done|fail; q: number of messages waiting
function strip(kind, st = 'auto', { sec = 12, q = 0, anim = true, pct = 65 } = {}) {
  const [t, s] = T[st];
  const fin = st === 'done' || st === 'fail';
  const time = sec >= 60 ? `${Math.floor(sec / 60)}m ${sec % 60}s` : `${sec}s`;
  const qd = q ? `<div class="queued">${q} message${q > 1 ? 's' : ''} waiting · I\'ll send ${q > 1 ? 'them' : 'it'} when the summary is done</div>` : '';
  if (kind === 'line') {
    if (st === 'done') return `<div class="divider"><b style="color:var(--eq-color-ok)">${I.check.replace('class="ico"', 'class="ico" style="width:16px;height:16px"')}Summarized · you can keep chatting</b></div>`;
    if (st === 'fail') return `<div class="inl" style="color:var(--eq-color-err)">${I.x.replace('class="ico"', 'class="ico" style="width:16px;height:16px"')}<span>Couldn't summarize. The conversation is unchanged.</span><button class="btn" style="margin-left:auto">Try again</button></div>`;
    return `<div><div class="inl"><span class="spin ${anim ? 'anim' : ''}"></span><span><b style="color:var(--eq-color-text);font-weight:600">${t}</b> · ${time}</span></div>${q ? `<div class="queued" style="margin-left:24px">${q} message${q > 1 ? 's' : ''} waiting · I'll send ${q > 1 ? 'them' : 'it'} when it's done</div>` : ''}</div>`;
  }
  const icon = st === 'done' ? I.check : st === 'fail' ? I.x : I.compress;
  const cls = st === 'done' ? 'ok' : st === 'fail' ? 'err' : '';
  let right = '';
  if (!fin) {
    const bar = kind === 'est' ? `<div class="bar"><i style="width:${st === 'long' ? 99 : pct}%"></i></div><span class="pct">${st === 'long' ? 99 : pct}%</span>`
      : `<div class="bar ind ${anim ? 'anim' : ''}"><i></i></div><span class="pct" style="min-width:48px">${time}</span>`;
    right = `<div class="row2">${bar}</div>`;
  }
  const act = st === 'fail' ? `<button class="btn" style="margin-top:var(--eq-space-2)">Try again</button>` : '';
  return `<div class="cz ${cls}" role="status">${icon}<div class="main"><div class="t">${t}</div><div class="s">${s}</div>${right}${qd}${act}</div></div>`;
}
const convo = `<div class="chat"><div class="u">Line up the spacing on the sign-in screen too</div><div><div class="who">frontend</div><div class="m">Sure — I did the settings screen first. Spacing is now <code style="font:var(--eq-type-code);font-size:12px;background:var(--eq-color-sunken);padding:1px 5px;border-radius:var(--eq-radius-code)">space.4</code> throughout; the sign-in screen is next.</div></div></div>`;
const composer = `<div class="composer"><span>Message a member…</span><span class="send">↑</span></div>`;
const page = (mode, w, h, body, extra = '') => `<!doctype html><html lang="en" data-theme="${mode}"><head><meta charset="utf-8">${fonts}<style>${tokens}\n${css}${extra}</style></head><body style="width:${w}px;${h ? `height:${h}px;overflow:hidden` : ''}">${body}</body></html>`;
function shot(name, html, w, h) {
  const f = `/tmp/cz-${name}.html`; fs.writeFileSync(f, html);
  execFileSync(CHROME, ['--headless=new', '--hide-scrollbars', '--force-device-scale-factor=2', `--window-size=${w},${h}`, '--virtual-time-budget=6000', `--screenshot=${path.join(HERE, name + '.png')}`, 'file://' + f], { stdio: 'ignore' });
  console.log(name);
}
const only = process.argv[2];
const run = (n, fn) => { if (!only || only === n) fn(); };

// ── 1. options comparison sheet ───────────────────────────────────────────
run('options', () => {
  const opts = [
    { k: 'est', name: 'A · estimated bar + %', desc: 'The same look as the Claude app. Filled by an estimate from past compaction times; it stops at 99% until it ends.', pos: 'Place: a band attached above the composer', cons: ['＋ A look people already know', '－ The engine gives no progress, so the % is made up', '－ When the estimate is wrong it sits at 99% for long and looks "broken"'] },
    { k: 'ind', name: 'B · flowing bar + elapsed time', desc: 'No %. The bar only says "working", and the number on the right is the time that has really passed.', pos: 'Place: a band attached above the composer', cons: ['＋ Every value shown is true (honest)', '＋ Taking long never makes it a lie', '－ It can\'t say "how much is left"'] },
    { k: 'line', name: 'C · a line in the conversation + elapsed time', desc: 'No bar: one line at the bottom of the conversation. When done, it stays as a "Summarized up to here" divider.', pos: 'Place: inside the conversation flow', cons: ['＋ The quietest, and the record stays in the conversation', '－ Scrolled up, the progress can\'t be seen', '－ Easily drifts apart from the sidebar status dot'] },
  ];
  const mock = (o, mode) => {
    const inner = o.k === 'line' ? `${convo}<div style="margin-top:var(--eq-space-3)">${strip('line')}</div>` : convo;
    return `<div data-theme="${mode}" style="background:var(--eq-color-bg);color:var(--eq-color-text);padding:var(--eq-space-4);border:1px solid var(--eq-color-line);border-radius:var(--eq-radius-lg);display:flex;flex-direction:column;gap:var(--eq-space-3);height:330px;justify-content:flex-end">${inner}${o.k === 'line' ? '' : strip(o.k)}${composer}</div>`;
  };
  const col = (o) => `<section style="flex:1;min-width:0;${o.k === 'ind' ? 'outline:2px solid var(--eq-color-text);outline-offset:12px;border-radius:var(--eq-radius-lg)' : ''}"><h2 style="font:700 20px/1.3 var(--eq-font-family-sans);margin:0 0 6px;letter-spacing:-0.01em">${o.name}${o.k === 'ind' ? ' <span style="font-size:12px;background:var(--eq-color-action);color:var(--eq-color-on-action);font-weight:600;border-radius:var(--eq-radius-full);padding:2px 10px;vertical-align:middle">Recommended</span>' : ''}</h2><p class="muted" style="font:var(--eq-type-small);margin:0 0 4px;min-height:42px">${o.desc}</p><p class="muted" style="font:var(--eq-type-meta);margin:0 0 12px">${o.pos}</p>
    <div style="display:grid;gap:12px"><div class="th" data-theme="light">${mock(o, 'light')}</div>${mock(o, 'dark')}</div><ul style="list-style:none;padding:0;margin:14px 0 0;font:var(--eq-type-small);display:grid;gap:6px">${o.cons.map((c) => `<li>${c}</li>`).join('')}</ul></section>`;
  const body = `<div style="padding:40px 48px 40px"><h1 style="font:700 30px/1.2 var(--eq-font-family-sans);letter-spacing:-0.02em;margin:0 0 6px">Showing a conversation being summarized — how to show progress</h1><p class="muted" style="font:var(--eq-type-small);max-width:900px;margin:0 0 28px">The engines (Claude Code·Codex) give no summary progress. The same state (auto summary, 12 seconds in) drawn three ways. Light above, dark below · the bottom of the desktop conversation screen. The mockups are still frames; in reality B's bar segment keeps flowing from left to right.</p><div style="display:flex;gap:28px">${cols()}</div></div>`;
  function cols() { return opts.map(col).join(''); }
  shot('options', page('light', 1500, 0, body), 1500, 1110);
});

// ── 2. states sheet (recommended B, desktop band width 520) ──────────────
run('states', () => {
  const S = [
    ['① Auto summary starts', strip('ind', 'auto', { sec: 1 }), 'When the engine starts compacting by itself the band appears at once (120ms fade).'],
    ['② In progress (the person ran /compact)', strip('ind', 'manual', { sec: 12 }), 'The wording when started with the "Summarize and continue fresh" button or /compact.'],
    ['③ Taking long (over 60 seconds)', strip('ind', 'long', { sec: 74 }), 'The bar keeps flowing; only the wording changes. No time limit turns it into a failure.'],
    ['④ A message sent while summarizing', strip('ind', 'auto', { sec: 31, q: 1 }), 'Input is kept and delivered when it ends. What was sent shows in the conversation as "waiting".'],
    ['⑤ Done', strip('ind', 'done'), 'The band goes after 4 seconds and a divider ("Summarized") stays in the conversation. Screen readers are told once.'],
    ['⑥ Failed', strip('ind', 'fail'), 'A failed auto summary is reported without touching the conversation. Waiting messages are still delivered.'],
  ];
  const cell = (s, mode) => `<div data-theme="${mode}" style="background:var(--eq-color-bg);padding:var(--eq-space-4);border-radius:var(--eq-radius-lg)">${s}</div>`;
  const body = `<div style="padding:40px 48px"><h1 style="font:700 28px/1.2 var(--eq-font-family-sans);letter-spacing:-0.02em;margin:0 0 6px">Recommended B — six states</h1><p class="muted" style="font:var(--eq-type-small);margin:0 0 24px">Both auto summary and /compact. Light left · dark right. Band width 520 (desktop); on mobile it fits the screen width.</p>
  <div style="display:grid;grid-template-columns:200px 1fr 1fr;gap:20px 20px;align-items:start">${S.map(([n, h, d]) => `<div><div style="font:600 14px var(--eq-font-family-sans)">${n}</div><div class="muted" style="font:var(--eq-type-meta);margin-top:4px">${d}</div></div>${cell(h, 'light')}${cell(h, 'dark')}`).join('')}</div>
  <h2 style="font:700 18px var(--eq-font-family-sans);margin:36px 0 12px">Long wording·reduce motion</h2><div style="display:grid;grid-template-columns:200px 1fr 1fr;gap:20px;align-items:start"><div><div style="font:600 14px var(--eq-font-family-sans)">Long names·narrow width (320)</div><div class="muted" style="font:var(--eq-type-meta);margin-top:4px">Text wraps (keep-all); the bar and time stay on one line.</div></div><div style="width:320px" data-theme="light"><div style="background:var(--eq-color-bg);padding:var(--eq-space-3)">${strip('ind', 'auto', { sec: 125, q: 2 })}</div></div><div style="width:320px" data-theme="dark"><div style="background:var(--eq-color-bg);padding:var(--eq-space-3);border-radius:var(--eq-radius-lg)">${strip('ind', 'auto', { sec: 125, q: 2 })}</div></div>
  <div><div style="font:600 14px var(--eq-font-family-sans)">Reduce motion</div><div class="muted" style="font:var(--eq-type-meta);margin-top:4px">No bar motion: the segment holds in the middle. The time keeps counting up. It appears with a 100ms fade.</div></div>${cell(strip('ind', 'auto', { sec: 12, anim: false }), 'light')}${cell(strip('ind', 'auto', { sec: 12, anim: false }), 'dark')}</div></div>`;
  shot('states', page('light', 1300, 0, body), 1300, 1440);
});

// ── 3. platform screens (recommended B) ───────────────────────────────
const members = [['pm', 'ok', 'Waiting'], ['frontend', 'work', 'Summarizing · 12s'], ['backend', 'work', 'Working'], ['mobile', 'ok', 'Waiting'], ['designer', 'ok', 'Waiting']];
run('desktop', () => {
  for (const mode of ['light', 'dark']) {
    const side = members.map(([n, s, sub]) => `<div style="display:flex;gap:10px;align-items:center;padding:8px 12px;border-radius:var(--eq-radius-control);${n === 'frontend' ? 'background:var(--eq-color-hover)' : ''}">${s === 'work' ? `<span class="spin anim"></span>` : `<span class="dot"></span>`}<div style="min-width:0"><div style="font:var(--eq-type-small);font-weight:600">${n}</div><div class="muted" style="font:var(--eq-type-meta);white-space:nowrap;overflow:hidden;text-overflow:ellipsis">${sub}</div></div></div>`).join('');
    const body = `<div style="display:flex;height:100%"><aside style="width:240px;background:var(--eq-color-sunken);padding:16px 8px;border-right:1px solid var(--eq-color-line)"><div style="font:600 12px var(--eq-font-family-sans);color:var(--eq-color-muted);padding:0 12px 8px">Members</div>${side}</aside>
    <main style="flex:1;display:flex;flex-direction:column;background:var(--eq-color-bg)"><header style="padding:14px 24px;border-bottom:1px solid var(--eq-color-line);font:var(--eq-type-label)">frontend <span class="muted" style="font:var(--eq-type-small)"> · equipe-site</span></header>
    <div style="flex:1;padding:24px;display:flex;flex-direction:column;justify-content:flex-end;gap:16px;max-width:760px;width:100%;margin:0 auto">${convo}${strip('ind', 'auto', { sec: 12, q: 1 })}${composer}</div></main></div>`;
    shot(`desktop-${mode}`, page(mode, 1100, 680, body), 1100, 680);
  }
});
run('ios', () => {
  for (const mode of ['light', 'dark']) {
    const body = `<div style="display:flex;flex-direction:column;height:100%;background:var(--eq-color-bg)"><div style="height:54px;display:flex;justify-content:space-between;align-items:flex-end;padding:0 28px 6px;font:600 15px var(--eq-font-family-sans)"><span>9:41</span><span class="muted">●●●</span></div>
    <div style="height:44px;display:flex;align-items:center;justify-content:center;border-bottom:1px solid var(--eq-color-line);font:var(--eq-type-label)">frontend</div>
    <div style="flex:1;padding:16px;display:flex;flex-direction:column;justify-content:flex-end;gap:12px">${convo}${strip('ind', 'auto', { sec: 12, q: 1 })}<div class="composer" style="min-height:44px;border-radius:22px;padding-left:16px"><span>Message</span><span class="send" style="width:32px;height:32px">↑</span></div></div><div style="height:34px"></div></div>`;
    shot(`ios-${mode}`, page(mode, 390, 844, body), 390, 844);
  }
});
run('android', () => {
  for (const mode of ['light', 'dark']) {
    const body = `<div style="display:flex;flex-direction:column;height:100%;background:var(--eq-color-bg)"><div style="height:32px;display:flex;justify-content:space-between;align-items:center;padding:0 20px;font:var(--eq-type-meta)"><span>9:41</span><span class="muted">▲ ◼</span></div>
    <div style="height:56px;display:flex;align-items:center;padding:0 16px;border-bottom:1px solid var(--eq-color-line);font:var(--eq-type-label);gap:16px"><span>←</span>frontend</div>
    <div style="flex:1;padding:16px;display:flex;flex-direction:column;justify-content:flex-end;gap:12px">${convo}${strip('ind', 'auto', { sec: 12, q: 1 })}<div class="composer" style="min-height:48px;border-radius:24px;padding-left:20px"><span>Message</span><span class="send" style="width:36px;height:36px">↑</span></div></div><div style="height:24px"></div></div>`;
    shot(`android-${mode}`, page(mode, 412, 892, body), 412, 892);
  }
});
run('tui', () => {
  const L = (c, t) => `<div style="color:${c}">${t}</div>`;
  const m = 'var(--eq-color-muted)', w = 'var(--eq-color-work)', ok = 'var(--eq-color-ok)', er = 'var(--eq-color-err)', tx = 'var(--eq-color-text)';
  const blk = (title, lines) => `<div style="margin-bottom:18px"><div style="color:${m};font-size:12px;margin-bottom:4px"># ${title}</div>${lines.join('')}</div>`;
  const body = `<div style="padding:20px 24px;font:14px/1.6 var(--eq-font-family-mono);background:var(--eq-color-bg);width:100%;height:100%">
  ${blk('Summarizing (the elapsed time counts up every second)', [L(tx, 'frontend › Spacing is now space.4 throughout; the sign-in screen is next.'), L(w, '⠹ Summarizing… 12s'), L(m, '  1 message will be sent when the summary is done'), L(tx, '› ▌')])}
  ${blk('Summarizing — reduce motion (a fixed symbol instead of the spinner)', [L(w, '… Summarizing 12s'), L(tx, '› ▌')])}
  ${blk('Done', [L(ok, '✓ Summarized. You can keep chatting'), L(tx, '› ▌')])}
  ${blk('Failed', [L(er, '✕ Couldn\'t summarize. The conversation is unchanged · /compact to try again'), L(tx, '› ▌')])}</div>`;
  shot('tui', page('dark', 760, 330, body), 760, 470);
});

// ── 4. platform comparison sheet ─────────────────────────────────────
run('platforms', () => {
  const img = (n, w) => `<img src="${HERE}/${n}.png" style="width:${w}px;border-radius:12px;border:1px solid var(--eq-color-line);display:block">`;
  const body = `<div style="padding:36px 40px"><h1 style="font:700 28px/1.2 var(--eq-font-family-sans);letter-spacing:-0.02em;margin:0 0 6px">Recommended B — screens by platform (shrunk to about half actual size)</h1><p class="muted" style="font:var(--eq-type-small);margin:0 0 24px">Desktop 1100×680 · iOS 390×844 · Android 412×892 · terminal 760 · light/dark. The sidebar's member row shows a spinner + "Summarizing · 12s", back to a dot when done.</p>
  <div style="display:flex;gap:20px;margin-bottom:20px">${img('desktop-light', 600)}${img('desktop-dark', 600)}</div>
  <div style="display:flex;gap:16px;align-items:flex-start;margin-bottom:20px">${img('ios-light', 280)}${img('ios-dark', 280)}${img('android-light', 296)}${img('android-dark', 296)}</div>
  ${img('tui', 760)}</div>`;
  shot('platforms', page('light', 1320, 0, body), 1320, 1650);
});
