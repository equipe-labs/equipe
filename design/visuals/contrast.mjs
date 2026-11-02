// node contrast.mjs — WCAG contrast of every text/graphic pair the cards and the Mermaid theme use, light and dark, from tokens/*.json.
// Writes contrast.md; exits 1 when a pair is under AA (text 4.5, graphic 3).
import fs from 'node:fs';
import { resolve } from '../scripts/lib.mjs';
const lin = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
const lum = (c) => 0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b);
const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
const over = (t, b) => ({ r: t.r * t.a + b.r * (1 - t.a), g: t.g * t.a + b.g * (1 - t.a), b: t.b * t.a + b.b * (1 - t.a), a: 1 });
// [fg, [bg layers bottom→top], need, use]
const P = [
  ['text', ['surface'], 4.5, '카드: 제목·본문'], ['muted', ['surface'], 4.5, '카드: 보조 글자·표 라벨·눈금'], ['muted', ['sunken'], 4.5, '카드: 프로젝트 줄·표 첫 열·중립 배지·Mermaid 레인 제목'], ['text', ['sunken'], 4.5, '카드: 프로젝트 이름'],
  ['ok', ['surface', 'ok-bg'], 4.5, '배지 완료'], ['work', ['surface', 'work-bg'], 4.5, '배지 진행 중'], ['wait', ['surface', 'wait-bg'], 4.5, '배지 사람 차례'], ['err', ['surface', 'err-bg'], 4.5, '배지 막힘'],
  ['accent', ['surface', 'accent-bg'], 4.5, '추천 배지·열'], ['text', ['surface', 'accent-bg'], 4.5, '추천 열 글자'], ['accent', ['surface'], 4.5, '추천 이유 강조'],
  ['ok', ['surface'], 4.5, '증감 ▲ 좋음'], ['err', ['surface'], 4.5, '증감 ▼ 나쁨'],
  ['accent', ['surface'], 3, '스파크라인 선·끝점'], ['text', ['surface', 'work-bg'], 4.5, 'Mermaid: 진행 중 노드 글자'], ['work', ['surface'], 3, 'Mermaid: 진행 중 노드 테두리'],
  ['text', ['surface', 'wait-bg'], 4.5, 'Mermaid: 사람 차례 노드 글자'], ['wait', ['surface'], 3, 'Mermaid: 사람 차례 노드 테두리'], ['text', ['surface', 'ok-bg'], 4.5, 'Mermaid: 완료 노드 글자'], ['ok', ['surface'], 3, 'Mermaid: 완료 테두리'],
  ['text', ['surface', 'err-bg'], 4.5, 'Mermaid: 막힘 노드 글자'], ['err', ['surface'], 3, 'Mermaid: 막힘 테두리'],
  ['border-strong', ['surface'], 3, 'Mermaid: 다음 노드 테두리'], ['muted', ['surface'], 3, 'Mermaid: 선·화살표'], ['accent', ['surface'], 3, 'Mermaid: 핵심 줄기'],
  ['on-action', ['action'], 4.5, 'Mermaid: 강조 노드 글자'], ['action', ['surface'], 3, 'Mermaid: 강조 노드 면'],
  ['text', ['bg'], 4.5, 'Mermaid: 알약 노드 글자 on 바탕색 면'], ['border-strong', ['bg'], 3, 'Mermaid: 알약 테두리'],
];
const SLICES = ['accent', 'ok', 'work', 'wait', 'err', 'border-strong'];
let fails = 0; const L = ['# 대화 속 그림 대비 표 (WCAG 2.x AA)', '', '`node contrast.mjs`로 다시 만든다. 글자 4.5:1, 그래픽(선·테두리·점) 3:1. 반투명 면은 아래층 위에 합성해서 계산했다.', ''];
for (const mode of ['light', 'dark']) {
  const T = Object.fromEntries((await resolve('web', mode)).map((x) => [x.name, x.value]));
  const g = (n) => T['color.' + n];
  L.push(`## ${mode === 'light' ? '라이트' : '다크'}`, '', '| 전경 | 바탕 | 대비 | 기준 | 판정 | 용도 |', '|---|---|---|---|---|---|');
  const sliceText = mode === 'light' ? 'on-accent' : 'bg';
  for (const [fg, bgs, need, use] of [...P, ...SLICES.map((k) => [sliceText, [k], 4.5, `Mermaid 원그래프: 조각(${k}) 위 글자`]), ...SLICES.map((k) => [k, ['surface'], 3, `Mermaid 원그래프: 조각(${k}) 면 vs 카드`])]) {
    let bg = { ...g(bgs[0]), a: 1 }; for (const o of bgs.slice(1)) bg = over(g(o), bg);
    const f = g(fg), r = ratio(f.a < 1 ? over(f, bg) : f, bg), ok = r >= need; if (!ok) fails++;
    L.push(`| ${fg} | ${bgs.join(' + ')} | ${r.toFixed(2)} | ${need} | ${ok ? '통과' : '**미달**'} | ${use} |`);
  }
  L.push('');
}
fs.writeFileSync(new URL('contrast.md', import.meta.url), L.join('\n'));
console.log(fails ? `${fails} pair(s) under AA` : 'all pairs pass'); process.exit(fails ? 1 : 0);
