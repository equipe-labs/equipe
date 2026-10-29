// 대화 요약(compaction) 시안 렌더 — node build.mjs  → *.png (HTML은 /tmp). 토큰은 ../../build/css/tokens.css만 쓴다.
// 순수 레이아웃 숫자는 space 토큰 값(4·8·12·16·24)을 따른다. 새로 필요한 건 loop 모션 길이 하나(제안: motion.duration.loop = 1600ms).
import fs from 'node:fs'; import path from 'node:path'; import { execFileSync } from 'node:child_process'; import { fileURLToPath } from 'node:url';
const HERE = path.dirname(fileURLToPath(import.meta.url));
const tokens = fs.readFileSync(path.join(HERE, '../../build/css/tokens.css'), 'utf8').replace(/:root\[data-theme="(dark|light)"\]/g, '[data-theme="$1"]'); // 한 화면 안에서 라이트·다크를 나란히 보이려고 :root 한정을 푼다
const CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const fonts = `<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;700&family=JetBrains+Mono:wght@400&display=swap"><link rel="stylesheet" href="https://cdn.jsdelivr.net/gh/orioncactus/pretendard@v1.3.9/dist/web/variable/pretendardvariable.min.css">`;
const css = `
*{box-sizing:border-box} body{margin:0;background:var(--eq-color-bg);color:var(--eq-color-text);font:var(--eq-type-ui);-webkit-font-smoothing:antialiased;word-break:keep-all;overflow-wrap:anywhere;font-variant-numeric:tabular-nums}
body,button{font-family:'Pretendard Variable','Inter Variable',Inter,system-ui,sans-serif}
.ico{width:var(--eq-size-icon-md);height:var(--eq-size-icon-md);flex:none}
.muted{color:var(--eq-color-muted)}
/* 요약 띠 */
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
/* 대화 */
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
  auto: ['대화를 요약하고 있어요', '길어진 대화를 정리하는 중이에요. 끝나면 이어서 할 수 있어요.'],
  manual: ['요약해서 새로 시작하고 있어요', '지금까지의 대화를 요약해 다음 대화로 넘겨요.'],
  long: ['요약이 오래 걸리고 있어요', '대화가 길어서 시간이 더 필요해요. 그대로 기다려 주세요.'],
  done: ['요약했어요', '이어서 대화할 수 있어요.'],
  fail: ['요약하지 못했어요', '대화는 그대로예요. 다시 시도할까요?'],
};
const el = (s) => s;
// kind: 'est' | 'ind' | 'line'; st: auto|manual|long|done|fail; q: 대기 메시지 수
function strip(kind, st = 'auto', { sec = 12, q = 0, anim = true, pct = 65 } = {}) {
  const [t, s] = T[st];
  const fin = st === 'done' || st === 'fail';
  const time = sec >= 60 ? `${Math.floor(sec / 60)}분 ${sec % 60}초` : `${sec}초`;
  const qd = q ? `<div class="queued">메시지 ${q}개 대기 중 · 요약이 끝나면 보낼게요</div>` : '';
  if (kind === 'line') {
    if (st === 'done') return `<div class="divider"><b style="color:var(--eq-color-ok)">${I.check.replace('class="ico"', 'class="ico" style="width:16px;height:16px"')}요약했어요 · 이어서 대화할 수 있어요</b></div>`;
    if (st === 'fail') return `<div class="inl" style="color:var(--eq-color-err)">${I.x.replace('class="ico"', 'class="ico" style="width:16px;height:16px"')}<span>요약하지 못했어요. 대화는 그대로예요.</span><button class="btn" style="margin-left:auto">다시 시도</button></div>`;
    return `<div><div class="inl"><span class="spin ${anim ? 'anim' : ''}"></span><span><b style="color:var(--eq-color-text);font-weight:600">${t}</b> · ${time}</span></div>${q ? `<div class="queued" style="margin-left:24px">메시지 ${q}개 대기 중 · 끝나면 보낼게요</div>` : ''}</div>`;
  }
  const icon = st === 'done' ? I.check : st === 'fail' ? I.x : I.compress;
  const cls = st === 'done' ? 'ok' : st === 'fail' ? 'err' : '';
  let right = '';
  if (!fin) {
    const bar = kind === 'est' ? `<div class="bar"><i style="width:${st === 'long' ? 99 : pct}%"></i></div><span class="pct">${st === 'long' ? 99 : pct}%</span>`
      : `<div class="bar ind ${anim ? 'anim' : ''}"><i></i></div><span class="pct" style="min-width:48px">${time}</span>`;
    right = `<div class="row2">${bar}</div>`;
  }
  const act = st === 'fail' ? `<button class="btn" style="margin-top:var(--eq-space-2)">다시 시도</button>` : '';
  return `<div class="cz ${cls}" role="status">${icon}<div class="main"><div class="t">${t}</div><div class="s">${s}</div>${right}${qd}${act}</div></div>`;
}
const convo = `<div class="chat"><div class="u">로그인 화면 간격도 같이 맞춰 줘</div><div><div class="who">frontend</div><div class="m">네, 설정 화면부터 맞췄어요. 간격은 <code style="font:var(--eq-type-code);font-size:12px;background:var(--eq-color-sunken);padding:1px 5px;border-radius:var(--eq-radius-code)">space.4</code>로 통일했고, 로그인 화면은 이어서 볼게요.</div></div></div>`;
const composer = `<div class="composer"><span>멤버에게 메시지…</span><span class="send">↑</span></div>`;
const page = (mode, w, h, body, extra = '') => `<!doctype html><html lang="ko" data-theme="${mode}"><head><meta charset="utf-8">${fonts}<style>${tokens}\n${css}${extra}</style></head><body style="width:${w}px;${h ? `height:${h}px;overflow:hidden` : ''}">${body}</body></html>`;
function shot(name, html, w, h) {
  const f = `/tmp/cz-${name}.html`; fs.writeFileSync(f, html);
  execFileSync(CHROME, ['--headless=new', '--hide-scrollbars', '--force-device-scale-factor=2', `--window-size=${w},${h}`, '--virtual-time-budget=6000', `--screenshot=${path.join(HERE, name + '.png')}`, 'file://' + f], { stdio: 'ignore' });
  console.log(name);
}
const only = process.argv[2];
const run = (n, fn) => { if (!only || only === n) fn(); };

// ── 1. 안 비교 시트 ───────────────────────────────────────────
run('options', () => {
  const opts = [
    { k: 'est', name: 'A · 추정 막대 + %', desc: 'Claude 앱과 같은 모양. 지난 압축 시간으로 추정해 채우고, 끝나기 전엔 99%에서 멈춰요.', pos: '자리: 입력창 위에 붙는 띠', cons: ['＋ 사람들이 이미 아는 모양이에요', '－ 엔진이 진행률을 주지 않아서 %는 지어낸 값이에요', '－ 추정이 틀리면 99%에서 오래 멈춰 "고장"처럼 보여요'] },
    { k: 'ind', name: 'B · 흐르는 막대 + 경과 시간', desc: '%는 없어요. 막대는 "일하는 중"만 알리고, 오른쪽 숫자는 실제로 지난 시간이에요.', pos: '자리: 입력창 위에 붙는 띠', cons: ['＋ 보여 주는 값이 전부 사실이에요 (정직)', '＋ 오래 걸려도 거짓말이 되지 않아요', '－ "얼마나 남았는지"는 알려 주지 못해요'] },
    { k: 'line', name: 'C · 대화 속 한 줄 + 경과 시간', desc: '막대 없이 대화 맨 아래에 한 줄. 끝나면 그 자리가 "여기까지 요약했어요" 구분선으로 남아요.', pos: '자리: 대화 흐름 안', cons: ['＋ 가장 조용하고, 기록이 대화에 남아요', '－ 위로 스크롤하면 진행이 안 보여요', '－ 사이드바 상태 점과 따로 놀기 쉬워요'] },
  ];
  const mock = (o, mode) => {
    const inner = o.k === 'line' ? `${convo}<div style="margin-top:var(--eq-space-3)">${strip('line')}</div>` : convo;
    return `<div data-theme="${mode}" style="background:var(--eq-color-bg);color:var(--eq-color-text);padding:var(--eq-space-4);border:1px solid var(--eq-color-line);border-radius:var(--eq-radius-lg);display:flex;flex-direction:column;gap:var(--eq-space-3);height:330px;justify-content:flex-end">${inner}${o.k === 'line' ? '' : strip(o.k)}${composer}</div>`;
  };
  const col = (o) => `<section style="flex:1;min-width:0;${o.k === 'ind' ? 'outline:2px solid var(--eq-color-text);outline-offset:12px;border-radius:var(--eq-radius-lg)' : ''}"><h2 style="font:700 20px/1.3 var(--eq-font-family-sans);margin:0 0 6px;letter-spacing:-0.01em">${o.name}${o.k === 'ind' ? ' <span style="font-size:12px;background:var(--eq-color-action);color:var(--eq-color-on-action);font-weight:600;border-radius:var(--eq-radius-full);padding:2px 10px;vertical-align:middle">추천</span>' : ''}</h2><p class="muted" style="font:var(--eq-type-small);margin:0 0 4px;min-height:42px">${o.desc}</p><p class="muted" style="font:var(--eq-type-meta);margin:0 0 12px">${o.pos}</p>
    <div style="display:grid;gap:12px"><div class="th" data-theme="light">${mock(o, 'light')}</div>${mock(o, 'dark')}</div><ul style="list-style:none;padding:0;margin:14px 0 0;font:var(--eq-type-small);display:grid;gap:6px">${o.cons.map((c) => `<li>${c}</li>`).join('')}</ul></section>`;
  const body = `<div style="padding:40px 48px 40px"><h1 style="font:700 30px/1.2 var(--eq-font-family-sans);letter-spacing:-0.02em;margin:0 0 6px">대화 요약 중 표시 — 진행률을 어떻게 보여줄까</h1><p class="muted" style="font:var(--eq-type-small);max-width:900px;margin:0 0 28px">엔진(Claude Code·Codex)은 요약 진행률을 주지 않아요. 같은 상태(자동 요약 12초째)를 세 가지로 그렸어요. 위는 라이트, 아래는 다크 · 데스크톱 대화 화면 아랫부분. 시안은 정지 화면이고, 실제로는 B의 막대 조각이 왼쪽에서 오른쪽으로 계속 흘러요.</p><div style="display:flex;gap:28px">${cols()}</div></div>`;
  function cols() { return opts.map(col).join(''); }
  shot('options', page('light', 1500, 0, body), 1500, 1070);
});

// ── 2. 상태 시트 (추천안 B, 데스크톱 띠 폭 520) ──────────────
run('states', () => {
  const S = [
    ['① 자동 요약 시작', strip('ind', 'auto', { sec: 1 }), '엔진이 스스로 압축을 시작하면 바로 띠가 나타나요 (120ms 페이드).'],
    ['② 진행 (사람이 /compact)', strip('ind', 'manual', { sec: 12 }), '"요약해서 새로 시작" 버튼이나 /compact로 시작한 경우의 문구.'],
    ['③ 오래 걸림 (60초 넘음)', strip('ind', 'long', { sec: 74 }), '막대는 그대로 흐르고 문구만 바뀌어요. 시간 제한으로 실패 처리하지 않아요.'],
    ['④ 요약 중에 메시지를 보냄', strip('ind', 'auto', { sec: 31, q: 1 }), '입력은 받아 두고 끝나면 전달해요. 보낸 글은 대화에 "대기 중"으로 보여요.'],
    ['⑤ 끝', strip('ind', 'done'), '띠는 4초 뒤 사라지고, 대화에 구분선("요약했어요")이 남아요. 스크린리더에는 한 번만 알려요.'],
    ['⑥ 실패', strip('ind', 'fail'), '자동 요약 실패는 대화를 건드리지 않고 알려요. 대기 메시지는 그대로 전달돼요.'],
  ];
  const cell = (s, mode) => `<div data-theme="${mode}" style="background:var(--eq-color-bg);padding:var(--eq-space-4);border-radius:var(--eq-radius-lg)">${s}</div>`;
  const body = `<div style="padding:40px 48px"><h1 style="font:700 28px/1.2 var(--eq-font-family-sans);letter-spacing:-0.02em;margin:0 0 6px">추천안 B — 상태 여섯</h1><p class="muted" style="font:var(--eq-type-small);margin:0 0 24px">자동 요약과 /compact 둘 다. 왼쪽 라이트 · 오른쪽 다크. 띠 폭 520(데스크톱), 모바일은 화면 폭에 맞춰요.</p>
  <div style="display:grid;grid-template-columns:200px 1fr 1fr;gap:20px 20px;align-items:start">${S.map(([n, h, d]) => `<div><div style="font:600 14px var(--eq-font-family-sans)">${n}</div><div class="muted" style="font:var(--eq-type-meta);margin-top:4px">${d}</div></div>${cell(h, 'light')}${cell(h, 'dark')}`).join('')}</div>
  <h2 style="font:700 18px var(--eq-font-family-sans);margin:36px 0 12px">긴 문구·동작 줄이기</h2><div style="display:grid;grid-template-columns:200px 1fr 1fr;gap:20px;align-items:start"><div><div style="font:600 14px var(--eq-font-family-sans)">긴 이름·좁은 폭 (320)</div><div class="muted" style="font:var(--eq-type-meta);margin-top:4px">글은 줄바꿈(keep-all), 막대와 시간은 한 줄 유지.</div></div><div style="width:320px" data-theme="light"><div style="background:var(--eq-color-bg);padding:var(--eq-space-3)">${strip('ind', 'auto', { sec: 125, q: 2 })}</div></div><div style="width:320px" data-theme="dark"><div style="background:var(--eq-color-bg);padding:var(--eq-space-3);border-radius:var(--eq-radius-lg)">${strip('ind', 'auto', { sec: 125, q: 2 })}</div></div>
  <div><div style="font:600 14px var(--eq-font-family-sans)">동작 줄이기</div><div class="muted" style="font:var(--eq-type-meta);margin-top:4px">막대 움직임 없음: 조각이 가운데 고정. 시간 숫자는 계속 올라가요. 나타남은 100ms 페이드.</div></div>${cell(strip('ind', 'auto', { sec: 12, anim: false }), 'light')}${cell(strip('ind', 'auto', { sec: 12, anim: false }), 'dark')}</div></div>`;
  shot('states', page('light', 1300, 0, body), 1300, 1400);
});

// ── 3. 플랫폼 화면 (추천안 B) ───────────────────────────────
const members = [['pm', 'ok', '대기 중'], ['frontend', 'work', '대화 요약 중 · 12초'], ['backend', 'work', '일하는 중'], ['mobile', 'ok', '대기 중'], ['designer', 'ok', '대기 중']];
run('desktop', () => {
  for (const mode of ['light', 'dark']) {
    const side = members.map(([n, s, sub]) => `<div style="display:flex;gap:10px;align-items:center;padding:8px 12px;border-radius:var(--eq-radius-control);${n === 'frontend' ? 'background:var(--eq-color-hover)' : ''}">${s === 'work' ? `<span class="spin anim"></span>` : `<span class="dot"></span>`}<div style="min-width:0"><div style="font:var(--eq-type-small);font-weight:600">${n}</div><div class="muted" style="font:var(--eq-type-meta);white-space:nowrap;overflow:hidden;text-overflow:ellipsis">${sub}</div></div></div>`).join('');
    const body = `<div style="display:flex;height:100%"><aside style="width:240px;background:var(--eq-color-sunken);padding:16px 8px;border-right:1px solid var(--eq-color-line)"><div style="font:600 12px var(--eq-font-family-sans);color:var(--eq-color-muted);padding:0 12px 8px">멤버</div>${side}</aside>
    <main style="flex:1;display:flex;flex-direction:column;background:var(--eq-color-bg)"><header style="padding:14px 24px;border-bottom:1px solid var(--eq-color-line);font:var(--eq-type-label)">frontend <span class="muted" style="font:var(--eq-type-small)"> · equipe-site</span></header>
    <div style="flex:1;padding:24px;display:flex;flex-direction:column;justify-content:flex-end;gap:16px;max-width:760px;width:100%;margin:0 auto">${convo}${strip('ind', 'auto', { sec: 12, q: 1 })}${composer}</div></main></div>`;
    shot(`desktop-${mode}`, page(mode, 1100, 680, body), 1100, 680);
  }
});
run('ios', () => {
  for (const mode of ['light', 'dark']) {
    const body = `<div style="display:flex;flex-direction:column;height:100%;background:var(--eq-color-bg)"><div style="height:54px;display:flex;justify-content:space-between;align-items:flex-end;padding:0 28px 6px;font:600 15px var(--eq-font-family-sans)"><span>9:41</span><span class="muted">●●●</span></div>
    <div style="height:44px;display:flex;align-items:center;justify-content:center;border-bottom:1px solid var(--eq-color-line);font:var(--eq-type-label)">frontend</div>
    <div style="flex:1;padding:16px;display:flex;flex-direction:column;justify-content:flex-end;gap:12px">${convo}${strip('ind', 'auto', { sec: 12, q: 1 })}<div class="composer" style="min-height:44px;border-radius:22px;padding-left:16px"><span>메시지</span><span class="send" style="width:32px;height:32px">↑</span></div></div><div style="height:34px"></div></div>`;
    shot(`ios-${mode}`, page(mode, 390, 844, body), 390, 844);
  }
});
run('android', () => {
  for (const mode of ['light', 'dark']) {
    const body = `<div style="display:flex;flex-direction:column;height:100%;background:var(--eq-color-bg)"><div style="height:32px;display:flex;justify-content:space-between;align-items:center;padding:0 20px;font:var(--eq-type-meta)"><span>9:41</span><span class="muted">▲ ◼</span></div>
    <div style="height:56px;display:flex;align-items:center;padding:0 16px;border-bottom:1px solid var(--eq-color-line);font:var(--eq-type-label);gap:16px"><span>←</span>frontend</div>
    <div style="flex:1;padding:16px;display:flex;flex-direction:column;justify-content:flex-end;gap:12px">${convo}${strip('ind', 'auto', { sec: 12, q: 1 })}<div class="composer" style="min-height:48px;border-radius:24px;padding-left:20px"><span>메시지</span><span class="send" style="width:36px;height:36px">↑</span></div></div><div style="height:24px"></div></div>`;
    shot(`android-${mode}`, page(mode, 412, 892, body), 412, 892);
  }
});
run('tui', () => {
  const L = (c, t) => `<div style="color:${c}">${t}</div>`;
  const m = 'var(--eq-color-muted)', w = 'var(--eq-color-work)', ok = 'var(--eq-color-ok)', er = 'var(--eq-color-err)', tx = 'var(--eq-color-text)';
  const blk = (title, lines) => `<div style="margin-bottom:18px"><div style="color:${m};font-size:12px;margin-bottom:4px"># ${title}</div>${lines.join('')}</div>`;
  const body = `<div style="padding:20px 24px;font:14px/1.6 var(--eq-font-family-mono);background:var(--eq-color-bg);width:100%;height:100%">
  ${blk('요약 중 (경과 시간이 1초마다 올라가요)', [L(tx, 'frontend › 간격은 space.4로 통일했고, 로그인 화면은 이어서 볼게요.'), L(w, '⠹ 대화 요약 중… 12초'), L(m, '  메시지 1개는 요약이 끝나면 보낼게요'), L(tx, '› ▌')])}
  ${blk('요약 중 — 동작 줄이기 (스피너 대신 고정 기호)', [L(w, '… 대화 요약 중 12초'), L(tx, '› ▌')])}
  ${blk('끝', [L(ok, '✓ 요약했어요. 이어서 대화할 수 있어요'), L(tx, '› ▌')])}
  ${blk('실패', [L(er, '✕ 요약하지 못했어요. 대화는 그대로예요 · /compact로 다시 시도'), L(tx, '› ▌')])}</div>`;
  shot('tui', page('dark', 760, 330, body), 760, 470);
});

// ── 4. 플랫폼 비교 시트 ─────────────────────────────────────
run('platforms', () => {
  const img = (n, w) => `<img src="${HERE}/${n}.png" style="width:${w}px;border-radius:12px;border:1px solid var(--eq-color-line);display:block">`;
  const body = `<div style="padding:36px 40px"><h1 style="font:700 28px/1.2 var(--eq-font-family-sans);letter-spacing:-0.02em;margin:0 0 6px">추천안 B — 플랫폼별 화면 (실제 크기의 절반 가까이로 줄임)</h1><p class="muted" style="font:var(--eq-type-small);margin:0 0 24px">데스크톱 1100×680 · iOS 390×844 · Android 412×892 · 터미널 760 · 라이트/다크. 사이드바 멤버 줄은 스피너 + "대화 요약 중 · 12초", 끝나면 점으로 돌아와요.</p>
  <div style="display:flex;gap:20px;margin-bottom:20px">${img('desktop-light', 600)}${img('desktop-dark', 600)}</div>
  <div style="display:flex;gap:16px;align-items:flex-start;margin-bottom:20px">${img('ios-light', 280)}${img('ios-dark', 280)}${img('android-light', 296)}${img('android-dark', 296)}</div>
  ${img('tui', 760)}</div>`;
  shot('platforms', page('light', 1320, 0, body), 1320, 1650);
});
