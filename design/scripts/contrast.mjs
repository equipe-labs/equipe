// npm run contrast — WCAG 2.x contrast for every text/background (4.5:1) and graphic/background (3:1)
// semantic pair, light and dark, from the same tokens the platforms are built from.
// Writes docs/contrast.md; exits 1 if any pair fails, so a token change that breaks AA breaks the build.
import { resolve, write } from './lib.mjs';

const TEXT = 4.5, GFX = 3.0;
const c = (n) => `color.${n}`;
// [foreground, background layers bottom→top, threshold, what it is]
const P = [
  ['text', ['bg'], TEXT, '본문 글자 on 바탕'], ['text', ['surface'], TEXT, '카드·필드 위 글자'], ['text', ['raised'], TEXT, '팝오버·시트 위 글자'], ['text', ['sunken'], TEXT, '코드·눌린 면 위 글자'],
  ['muted', ['bg'], TEXT, '보조 글자 on 바탕'], ['muted', ['surface'], TEXT, '보조 글자·플레이스홀더 on 카드/필드'], ['muted', ['raised'], TEXT, '보조 글자 on 시트'], ['muted', ['sunken'], TEXT, '보조 글자 on 눌린 면(중립 배지)'],
  ['accent', ['bg'], TEXT, '링크·텍스트 버튼 on 바탕'], ['accent', ['surface'], TEXT, '링크 on 카드'], ['accent', ['raised'], TEXT, '링크 on 시트'],
  ['accent', ['surface', 'accent-bg'], TEXT, '선택된 칩·행 글자(accent on accent-bg)'], ['text', ['surface', 'accent-bg'], TEXT, '선택된 행 본문 글자'], ['muted', ['surface', 'accent-bg'], TEXT, '선택된 행 보조 글자'],
  ['on-accent', ['accent'], TEXT, '채운 브랜드 버튼 글자 (웹 마케팅)'],
  ['on-action', ['action'], TEXT, '주 버튼 글자'], ['on-action', ['action-hover'], TEXT, '주 버튼 호버'], ['on-action', ['action-pressed'], TEXT, '주 버튼 눌림'],
  ['on-err', ['err'], TEXT, '위험 버튼 글자'],
  ['text', ['bg', 'hover'], TEXT, '행 호버 위 글자'], ['text', ['surface', 'hover'], TEXT, '카드 안 행 호버 위 글자'], ['muted', ['surface', 'hover'], TEXT, '행 호버 위 보조 글자'], ['text', ['bg', 'pressed'], TEXT, '눌린 행 위 글자'], ['accent', ['bg', 'pressed'], TEXT, '눌린 텍스트 버튼'],
  ['ok', ['bg'], TEXT, '완료 글자 on 바탕'], ['work', ['bg'], TEXT, '진행 중 글자 on 바탕'], ['wait', ['bg'], TEXT, '내 차례 글자 on 바탕'], ['err', ['bg'], TEXT, '오류 글자 on 바탕'],
  ['ok', ['surface'], TEXT, '완료 글자 on 카드'], ['work', ['surface'], TEXT, '진행 중 글자 on 카드'], ['wait', ['surface'], TEXT, '내 차례 글자 on 카드'], ['err', ['surface'], TEXT, '오류 글자 on 카드'],
  ['ok', ['raised'], TEXT, '완료 on 시트'], ['wait', ['raised'], TEXT, '내 차례 on 시트'], ['err', ['raised'], TEXT, '오류 on 시트(필드 오류 글자)'],
  ['ok', ['surface', 'ok-bg'], TEXT, '배지 ok (카드 위)'], ['work', ['surface', 'work-bg'], TEXT, '배지 work (카드 위)'], ['wait', ['surface', 'wait-bg'], TEXT, '배지 wait (카드 위)'], ['err', ['surface', 'err-bg'], TEXT, '배지 err (카드 위)'],
  ['ok', ['bg', 'ok-bg'], TEXT, '배지 ok (바탕 위)'], ['work', ['bg', 'work-bg'], TEXT, '배지 work (바탕 위)'], ['wait', ['bg', 'wait-bg'], TEXT, '배지 wait (바탕 위)'], ['err', ['bg', 'err-bg'], TEXT, '배지·배너 err (바탕 위)'],
  ['on-inverse', ['inverse'], TEXT, '토스트 글자'], ['inverse-accent', ['inverse'], TEXT, '토스트 액션'],
  ['focus', ['bg'], GFX, '포커스 링 on 바탕'], ['focus', ['surface'], GFX, '포커스 링 on 카드'], ['focus', ['raised'], GFX, '포커스 링 on 시트'], ['focus', ['sunken'], GFX, '포커스 링 on 눌린 면'],
  ['border-strong', ['bg'], GFX, '필드·보조 버튼 테두리 on 바탕'], ['border-strong', ['surface'], GFX, '필드 테두리 on 카드'], ['border-strong', ['raised'], GFX, '시트 손잡이·필드 on 시트'],
  ['accent', ['surface', 'accent-bg'], GFX, '선택된 칩 테두리'], ['wait', ['surface'], GFX, '결정 카드 테두리(wait)'], ['err', ['surface'], GFX, '오류 필드 테두리'],
  ['dot', ['bg'], GFX, '브랜드 점 on 바탕 (그래픽·로고 전용)'], ['dot', ['surface'], GFX, '브랜드 점 on 카드'], ['dot', ['raw.neutral.950'], GFX, '브랜드 점 on 아이콘 타일(void)'],
];

const lin = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
const lum = ({ r, g, b }) => 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
const over = (top, bot) => ({ r: top.r * top.a + bot.r * (1 - top.a), g: top.g * top.a + bot.g * (1 - top.a), b: top.b * top.a + bot.b * (1 - top.a), a: 1 });
const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
const hex = ({ r, g, b }) => '#' + [r, g, b].map((x) => Math.round(x).toString(16).padStart(2, '0')).join('').toUpperCase();

let fails = 0, total = 0;
const lines = ['# 대비 표 — 의미 토큰 조합 (WCAG 2.x AA)', '',
  '`npm run contrast`로 다시 만든다 (`scripts/contrast.mjs`, 토큰 `tokens/*.json`에서 직접 읽음). 글자 4.5:1, 그래픽(포커스 링·테두리·점·아이콘) 3:1. 반투명 면은 아래층 위에 합성해서 계산한다 ("바탕" 열의 `a + b`는 a 위에 b를 얹은 것).',
  '비활성 상태(`opacity.disabled` 0.4)는 WCAG 1.4.3 예외라 계산에서 뺐다.', ''];
for (const mode of ['light', 'dark']) {
  const t = Object.fromEntries((await resolve('web', mode)).map((x) => [x.name, x.value]));
  const get = (n) => { const v = t[c(n)] ?? t['color.' + n]; if (!v) throw new Error('no token ' + n); return v; };
  lines.push(`## ${mode === 'light' ? '라이트' : '다크'}`, '', '| 전경 | 바탕 | 대비 | 기준 | 판정 | 용도 |', '|---|---|---|---|---|---|');
  for (const [fg, bgs, th, use] of P) {
    let bg = { ...get(bgs[0]), a: 1 };
    // A base layer that is itself translucent never occurs; overlays sit on top of the first layer.
    for (const o of bgs.slice(1)) bg = over(get(o), bg);
    const f = get(fg);
    const fgc = f.a < 1 ? over(f, bg) : f;
    const r = ratio(fgc, bg);
    const ok = r >= th; total++; if (!ok) fails++;
    lines.push(`| \`${fg}\` ${f.hex} | ${bgs.map((b) => '`' + b + '`').join(' + ')} ${hex(bg)} | ${r.toFixed(2)}:1 | ${th}:1 | ${ok ? '✅' : '❌'} | ${use} |`);
  }
  lines.push('');
}
lines.push(`**합계: ${total}개 조합 중 미달 ${fails}개.**`, '');
write('docs/contrast.md', lines.join('\n'));
console.log(`contrast: ${total} pairs, ${fails} below threshold -> docs/contrast.md`);
if (fails) { console.error(lines.filter((l) => l.includes('❌')).join('\n')); process.exit(1); }
