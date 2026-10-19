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
const STATE_H = ['기본 default', '호버 hover', '눌림 pressed', '포커스 focus', '비활성 disabled'];

const btn = (v, label, st, extra = '') => `<button class="eq-btn eq-btn--${v} eq-anim eq-focusable ${v === 'primary' ? '' : 'eq-overlay-hover'} ${st} ${extra}">${label}</button>`;

const sheets = {
  button: () => ({
    title: '버튼 Button', sub: '높이 `button.height`(웹 44 · 데스크톱 28 · iOS 44 · Android 48). 주 버튼은 `color.action`(글자색 면), 브랜드 파랑 면은 웹 마케팅에만. 호버·눌림은 주 버튼만 전용 색, 나머지는 중립 오버레이(`color.hover/pressed`). 위험 버튼은 되돌릴 수 없는 일에만.',
    body: `<div class="grid" style="${cols(5)}">${heads(STATE_H)}
      ${row('주 primary', STATES.map((s) => btn('primary', '허용하기', s)))}
      ${row('보조 secondary', STATES.map((s) => btn('secondary', '거절', s)))}
      ${row('텍스트 ghost', STATES.map((s) => btn('ghost', '자세히 보기', s)))}
      ${row('위험 danger', STATES.map((s) => btn('danger', '기기 지우기', s)))}
      ${row('브랜드 brand<br>(웹 마케팅)', STATES.map((s) => btn('brand', 'macOS 앱 받기', s)))}
    </div>
    <div class="grid" style="${cols(4)}">${heads(['크기 size', '', '', ''])}
      ${row('md (기본)', [btn('primary', '허용하기', ''), btn('secondary', '거절', ''), '', ''])}
      ${row('sm', [btn('primary', '허용하기', '', 'eq-btn--sm'), btn('secondary', '거절', '', 'eq-btn--sm'), btn('ghost', '더 보기', '', 'eq-btn--sm'), ''])}
    </div>`,
  }),
  input: () => ({
    title: '입력창 Input', sub: '테두리는 `color.border-strong`(3:1 이상) — 헤어라인(`line`)이 아니다. 포커스는 테두리 2px + 링 색 `color.focus`. 오류는 색만이 아니라 아래 문구로도 알린다. 플레이스홀더는 `muted`(4.5:1 이상)이고 라벨을 대신하지 않는다.',
    body: `<div class="grid" style="${cols(5)}">${heads(['기본 default', '호버 hover', '포커스 focus', '오류 error', '비활성 disabled'])}
      ${row('비었을 때', [
        `<div class="eq-field"><label>프로젝트 이름</label><input class="eq-input eq-anim" placeholder="예: equipe-site"><p class="eq-help">나중에 바꿀 수 있어요.</p></div>`,
        `<div class="eq-field"><label>프로젝트 이름</label><input class="eq-input eq-anim is-hover" placeholder="예: equipe-site"><p class="eq-help">나중에 바꿀 수 있어요.</p></div>`,
        `<div class="eq-field"><label>프로젝트 이름</label><input class="eq-input eq-anim is-focus" placeholder="예: equipe-site"><p class="eq-help">나중에 바꿀 수 있어요.</p></div>`,
        `<div class="eq-field"><label>프로젝트 이름</label><input class="eq-input eq-anim is-error" placeholder="예: equipe-site"><p class="eq-help is-error">✕ 이름을 입력해 주세요.</p></div>`,
        `<div class="eq-field"><label>프로젝트 이름</label><input class="eq-input eq-anim" disabled placeholder="예: equipe-site"><p class="eq-help">지금은 바꿀 수 없어요.</p></div>`])}
      ${row('값이 있을 때', ['기본', '호버', '포커스', '오류', '비활성'].map((_, i) => `<div class="eq-field"><label>페어링 코드</label><input class="eq-input eq-anim ${['', 'is-hover', 'is-focus', 'is-error', ''][i]}" value="K7QD-92LM" ${i === 4 ? 'disabled' : ''}>${i === 3 ? '<p class="eq-help is-error">✕ 코드가 맞지 않아요.</p>' : ''}</div>`))}
    </div>`,
  }),
  card: () => ({
    title: '카드 Card', sub: '기본 카드는 `surface` + 헤어라인. 떠 있는 카드(`raised`)는 그림자 `elevation.float` — 팝오버·메뉴처럼 위로 뜨는 것에만. 누를 수 있는 카드는 호버·눌림에 중립 오버레이를 얹는다. 카드 안 카드는 쓰지 않는다.',
    body: `<div class="grid" style="${cols(5)}">${heads(STATE_H)}
      ${row('눌러서 여는 카드', STATES.map((s) => `<div class="eq-card eq-card--link eq-anim eq-focusable eq-overlay-hover ${s}" tabindex="-1"><h3>equipe-site</h3><p>디자이너가 토큰 초안을 올렸어요.</p><span class="meta">2분 전 · 티켓 3개</span></div>`))}
      ${row('떠 있는 카드<br>(raised)', STATES.slice(0, 1).map((s) => `<div class="eq-card eq-card--raised"><h3>복사했어요</h3><p>링크를 클립보드에 넣었어요.</p></div>`).concat(['', '', '', '']))}
    </div>`,
  }),
  chip: () => ({
    title: '칩 Chip', sub: '필터·선택용 알약. 선택은 색만이 아니라 `✓` 표시도 함께. 높이 `size.chip`(웹·모바일 32 · 데스크톱 24), 모바일은 터치 영역을 44/48까지 키운다(보이는 크기는 그대로). 상태를 알리는 건 배지(Badge)이고 칩이 아니다.',
    body: `<div class="grid" style="${cols(5)}">${heads(STATE_H)}
      ${row('기본', STATES.map((s) => `<span class="eq-chip eq-anim eq-focusable eq-overlay-hover ${s}">전체</span>`))}
      ${row('선택됨 selected', STATES.map((s) => `<span class="eq-chip eq-anim eq-focusable eq-overlay-hover is-selected ${s}">✓ equipe-site</span>`))}
    </div>`,
  }),
  badge: () => ({
    title: '배지 Badge', sub: '상태는 색 + 기호 + 글자 셋을 함께 쓴다(색만으로 전달하지 않는다). 의미: 완료 ok · 일하는 중 work(amber) · 내 차례 wait(orange, 사람을 기다림) · 오류 err. 프로젝트 색으로 amber/orange를 쓰지 않는다. 눌러서 동작하지 않는 표시 전용이라 호버·눌림 상태가 없다.',
    body: `<div class="grid" style="${cols(5)}">${heads(['완료 ok', '일하는 중 work', '내 차례 wait', '오류 err', '중립 neutral'])}
      ${['바탕 위 (bg)', '카드 위 (surface)'].map((l, i) => row(l, [
        `<span class="eq-badge eq-badge--ok">✓ 완료</span>`, `<span class="eq-badge eq-badge--work">⋯ 일하는 중</span>`, `<span class="eq-badge eq-badge--wait">! 내 차례</span>`, `<span class="eq-badge eq-badge--err">✕ 오류</span>`, `<span class="eq-badge">초안</span>`].map((c) => i ? `<div style="background:var(--eq-color-surface);padding:14px 16px;border-radius:var(--eq-radius-md);display:inline-block">${c}</div>` : c))).join('')}
    </div>`,
  }),
  row: () => ({
    title: '목록 행 List row', sub: '행 높이 `size.row`(웹 48 · 데스크톱 32 · iOS 44 · Android 56). 구분선은 `line`, 선택은 `accent-bg` + 글자(강조 색 선만으로 구분하지 않는다). 행 전체가 눌림 영역이다.',
    body: `<div class="grid" style="grid-template-columns: 150px 640px">${heads(['행 (web)'])}
      ${[['기본', ''], ['호버 hover', 'is-hover'], ['눌림 pressed', 'is-pressed'], ['선택됨 selected', 'is-selected'], ['포커스 focus', 'is-focus'], ['비활성 disabled', 'is-disabled']].map(([l, s], i) => `<div class="rl">${l}</div><div style="background:var(--eq-color-surface);border-radius:var(--eq-radius-card);padding:0 var(--eq-space-2)"><div class="eq-row eq-anim eq-focusable eq-overlay-hover ${s}"><span style="width:var(--eq-space-3);height:var(--eq-space-3);border-radius:var(--eq-radius-full);background:var(--eq-color-work);flex:none"></span><span class="grow"><span>designer</span><span class="secondary">토큰 초안을 정리하고 있어요</span></span><span class="eq-badge eq-badge--wait">! 내 차례</span><span class="secondary" style="color:var(--eq-color-muted)">›</span></div></div>`).join('')}
    </div>
    <p class="note">앞의 점은 <code>work</code>색이지만 <b>옆 글자(상태 배지·보조 문장)가 의미를 함께 전달</b>한다. 실제 화면에서는 멤버 아바타가 들어간다.</p>`,
  }),
  sheet: () => ({
    title: '시트·모달 Sheet / Modal', sub: '웹·데스크톱은 가운데 대화상자(최대 480), 모바일은 아래에서 올라오는 시트(손잡이 36×4). 뒤에는 `scrim`. 닫는 길(취소·바깥 탭·Esc)이 항상 있고, 위험한 한 가지 행동만 강조한다. 움직임: 대화상자 `motion.duration.slow`(200ms) + 8 이동, \'동작 줄이기\'에서는 이동 없이 100ms 페이드.',
    body: `<div style="display:grid;grid-template-columns: 1fr 1fr;gap:28px">
      <div><div class="note"><b>대화상자 (web · desktop)</b></div><div class="frame"><div class="under">설정 · 기기<br>목록 …</div><div class="eq-scrim"><div class="eq-sheet" role="dialog"><h3>이 기기를 지울까요?</h3><p>폰 'iPhone 15'의 연결이 끊어지고, 다시 쓰려면 새로 페어링해야 해요.</p><div class="eq-actions">${btn('secondary', '취소', '')}${btn('danger', '지우기', '')}</div></div></div></div></div>
      <div><div class="note"><b>바텀 시트 (iOS · Android)</b></div><div class="frame"><div class="under">기기<br>목록 …</div><div class="eq-scrim eq-scrim--bottom"><div class="eq-sheet eq-sheet--bottom"><div class="grabber"></div><h3>이 기기를 지울까요?</h3><p>연결이 끊어지고, 다시 쓰려면 새로 페어링해야 해요.</p><div class="eq-actions" style="flex-direction:column">${btn('danger', '지우기', '')}${btn('secondary', '취소', '')}<div style="height:var(--eq-space-2)"></div></div></div></div></div></div>
    </div>`,
  }),
  decision: () => ({
    title: '결정 카드 Decision card', sub: '사람의 승인이 필요한 순간 — 제품에서 가장 크게 보여야 하는 컴포넌트. 테두리 2px `wait`(내 차례)과 배지 "내 차례"로 알리고, 누가·어디서·무엇을 하려는지를 먼저, 두 행동(거절·허용)은 같은 폭으로 나란히 둔다. 허용은 주 버튼, 거절은 보조. 한 번만 유효하다는 걸 글로 밝힌다.',
    body: `<div style="display:grid;grid-template-columns: 1fr 1fr;gap:28px 28px">
      ${[
        ['기본 default', '', ''],
        ['포커스 focus (허용)', '', 'f'],
        ['처리 중 processing', '', 'p'],
        ['완료 resolved', 'r', ''],
      ].map(([l, k, o]) => `<div><div class="note"><b>${l}</b></div><div class="eq-decision ${k === 'r' ? 'is-resolved' : ''}">
        <div style="display:flex;justify-content:space-between;align-items:center">${k === 'r' ? '<span class="eq-badge eq-badge--ok">✓ 허용했어요</span>' : '<span class="eq-badge eq-badge--wait">! 내 차례</span>'}<span class="eq-help">방금</span></div>
        <p class="who">frontend가 파일을 쓰려고 해요</p><p class="where">equipe-site · src/styles/global.css</p>
        <pre>write src/styles/global.css  (+18 −4)</pre>
        <p class="eq-help">한 번만 허용돼요. 같은 일을 또 하면 다시 물어요.</p>
        ${k === 'r' ? '' : `<div class="eq-actions">${btn('secondary', '거절', o === 'p' ? 'is-disabled' : '')}${btn('primary', o === 'p' ? '보내는 중…' : '한 번 허용', o === 'f' ? 'is-focus' : o === 'p' ? 'is-disabled' : '')}</div>`}
      </div></div>`).join('')}
    </div>`,
  }),
  toast: () => ({
    title: '토스트 Toast', sub: '짧은 확인용 — 사라져도 큰일이 없는 정보만. 페이지 반대 색(`inverse`) 바탕이라 어떤 화면 위에서도 읽힌다. 5초(`motion.duration.toast`) 뒤에 사라지고, 호버·포커스 중에는 멈춘다. 액션이 있으면 사라지지 않게 두거나 다른 길(설정·내역)을 남긴다. 스크린리더에는 `role="status"`로 읽어 준다. 오류는 토스트가 아니라 배너/필드 오류로.',
    body: `<div class="grid" style="${cols(4)}">${heads(['기본', '액션 있음', '액션 호버', '액션 포커스'])}
      ${row('바탕 위 (bg)', [
        `<div class="eq-toast" role="status"><span>✓</span><span>링크를 복사했어요</span></div>`,
        `<div class="eq-toast"><span>✓</span><span>티켓을 보냈어요</span><button class="eq-toast-action">되돌리기</button></div>`,
        `<div class="eq-toast"><span>✓</span><span>티켓을 보냈어요</span><button class="eq-toast-action is-hover">되돌리기</button></div>`,
        `<div class="eq-toast"><span>✓</span><span>티켓을 보냈어요</span><button class="eq-toast-action is-focus">되돌리기</button></div>`])}
      ${row('카드 위 (surface)', ['', '', '', ''].map((_, i) => `<div style="background:var(--eq-color-surface);padding:20px;border-radius:var(--eq-radius-card)"><div class="eq-toast"><span>✓</span><span>${i ? '티켓을 보냈어요' : '링크를 복사했어요'}</span>${i ? `<button class="eq-toast-action ${['', 'is-hover', 'is-focus'][i - 1]}">되돌리기</button>` : ''}</div></div>`))}
    </div>`,
  }),
  foundations: () => {
    const groups = {
      '바탕·면': ['bg', 'surface', 'raised', 'sunken'], '글자': ['text', 'muted'], '강조': ['accent', 'on-accent', 'accent-bg', 'focus', 'dot'],
      '행동': ['action', 'on-action', 'action-hover', 'action-pressed', 'hover', 'pressed', 'border-strong', 'line', 'scrim'], '상태': ['ok', 'work', 'wait', 'err', 'ok-bg', 'work-bg', 'wait-bg', 'err-bg', 'on-err'], '반전': ['inverse', 'on-inverse', 'inverse-accent'],
    };
    const sw = (n) => `<div class="sw"><i style="background:var(--eq-color-${n})"></i><div><b>${n}</b><span>--eq-color-${n}</span></div></div>`;
    const types = ['display', 'headline', 'title', 'body', 'ui', 'label', 'small', 'meta', 'code'];
    return { title: '기초 Foundations', sub: '색 역할·글자 역할·간격·모서리·그림자·움직임. 값은 모두 `tokens/*.json` → `build/css/tokens.css`에서 온다. 이 시트는 웹 값 기준(앱은 `components.md`의 플랫폼 표).',
      body: `${Object.entries(groups).map(([g, ns]) => `<p class="note"><b>${g}</b></p><div style="display:grid;grid-template-columns:repeat(4,1fr);gap:14px 24px;margin-bottom:26px">${ns.map(sw).join('')}</div>`).join('')}
      <p class="note"><b>글자 역할 type roles</b> (웹 &gt;640px)</p><div style="margin-bottom:26px">${types.map((t) => `<div style="display:flex;gap:24px;align-items:baseline;margin-bottom:6px"><code style="width:90px;text-align:center">${t}</code><span style="font:var(--eq-type-${t});letter-spacing:var(--eq-type-${t}-tracking)">${t === 'code' ? 'equipe pair --qr' : '이끄는 사람이 승인해요 · You approve every merge'}</span></div>`).join('')}</div>
      <p class="note"><b>간격 space</b> · <b>모서리 radius</b> · <b>그림자 elevation</b></p>
      <div style="display:flex;gap:40px;align-items:flex-end;margin-bottom:26px;flex-wrap:wrap">
        <div style="display:flex;gap:14px;align-items:flex-end">${[1, 2, 3, 4, 5, 6, 7].map((n) => `<div style="text-align:center"><div style="width:var(--eq-space-${n});height:var(--eq-space-${n});background:var(--eq-color-accent);border-radius:2px"></div><code>${n}</code></div>`).join('')}</div>
        <div style="display:flex;gap:14px;align-items:flex-end">${['sm', 'md', 'lg', 'xl', 'xxl', 'full'].map((n) => `<div style="text-align:center"><div style="width:64px;height:64px;border:2px solid var(--eq-color-text);border-radius:var(--eq-radius-${n})"></div><code>${n}</code></div>`).join('')}</div>
        <div style="display:flex;gap:26px">${['float', 'overlay'].map((n) => `<div style="text-align:center"><div style="width:120px;height:72px;background:var(--eq-color-raised);border-radius:var(--eq-radius-card);box-shadow:var(--eq-elevation-${n})"></div><code>${n}</code></div>`).join('')}</div>
      </div>
      <p class="note"><b>움직임 motion</b> — 곡선은 하나(<code>out-cubic</code>), 시간은 셋. '동작 줄이기'에서는 이동 0, 100ms 페이드만.</p>
      <table style="border-collapse:collapse;font:var(--eq-type-small);margin-bottom:12px">${[['fast 120ms', '호버 채움 · 눌림 · 툴팁'], ['base 160ms', '팝오버 · 메뉴 · 칩'], ['slow 200ms', '대화상자 · 시트 · 카드/메시지 등장'], ['slide 180ms', '사이드바·패널 (out-quint)'], ['toast 5000ms', '토스트 머무는 시간 (애니메이션 아님)'], ['travel 4 · 6 · 8', 'pop · rise · dialog 이동 거리']].map(([a, b]) => `<tr><td style="padding:3px 24px 3px 0"><code>${a}</code></td><td style="color:var(--eq-color-muted)">${b}</td></tr>`).join('')}</table>`,
    };
  },
};

fs.mkdirSync(path.join(ROOT, 'build/sheets'), { recursive: true });
const only = process.argv[2];
for (const [name, fn] of Object.entries(sheets)) {
  if (only && only !== name) continue;
  const s = fn();
  for (const mode of ['light', 'dark']) {
    const html = `<!doctype html><html lang="ko" data-theme="${mode}"><head><meta charset="utf-8">${fonts}<style>${tokens}\n${comps}\n${shellCss}</style></head><body><div class="page"><h1>${s.title} <span style="font-weight:400;color:var(--eq-color-muted);font-size:20px">· ${mode === 'light' ? '라이트' : '다크'}</span></h1><p class="sub">${s.sub.replace(/`([^`]+)`/g, '<code>$1</code>')}</p>${s.body}</div></body></html>`;
    const hf = path.join(ROOT, `build/sheets/${name}-${mode}.html`); fs.writeFileSync(hf, html);
    const raw = `/tmp/eq-${name}-${mode}.png`, out = path.join(ROOT, `components/${name}-${mode}.png`);
    execFileSync(CHROME, ['--headless=new', '--hide-scrollbars', '--force-device-scale-factor=2', '--window-size=1400,2400', '--virtual-time-budget=9000', `--screenshot=${raw}`, 'file://' + hf], { stdio: 'ignore' });
    execFileSync('python3', [path.join(ROOT, 'scripts/crop.py'), raw, out]);
    console.log('sheet', name, mode);
  }
}
