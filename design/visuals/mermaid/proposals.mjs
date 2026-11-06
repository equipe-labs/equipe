// Draws the three Mermaid looks (A cards, B lanes, C pills) for one ticket-flow diagram, in light and dark, with the desktop's own renderer.
// needs: $CARGO_TARGET_DIR/release/mmr  (tiny wrapper around mermaid-rs-renderer 0.3 — see README) ; node proposals.mjs <outdir>
import fs from 'node:fs'; import path from 'node:path'; import { execFileSync } from 'node:child_process';
import { resolve } from '../../scripts/lib.mjs';
const out = process.argv[2] || 'proposals'; fs.mkdirSync(out, { recursive: true });
const MMR = path.join(process.env.CARGO_TARGET_DIR || 'target', 'release/mmr');
const over = (top, bot) => { const f = (k) => Math.round(top[k] * top.a + bot[k] * (1 - top.a)); return '#' + [f('r'), f('g'), f('b')].map((x) => x.toString(16).padStart(2, '0')).join('').toUpperCase(); };

for (const mode of ['light', 'dark']) {
  const T = Object.fromEntries((await resolve('web', mode)).map((x) => [x.name, x.value]));
  const c = (n) => T['color.' + n].hex.slice(0, 7);
  const tint = (n, base = 'surface') => over(T['color.' + n], T['color.' + base]);
  const base = { base: mode, font_family: '"Helvetica Neue", "Apple SD Gothic Neo", sans-serif', font_size: 14, background: 'transparent',
    primary_color: c('surface'), primary_text_color: c('text'), text_color: c('muted'), primary_border_color: c('border-strong'), line_color: c('muted'),
    secondary_color: c('sunken'), tertiary_color: c('raised'), edge_label_background: c('surface'), cluster_background: c('sunken'), cluster_border: over(T['color.line'], T['color.sunken']) };
  const st = (k) => `fill:${tint(k + '-bg')},stroke:${c(k)},stroke-width:1.5px,color:${c('text')}`;

  const A = `flowchart LR
  T336("● T-336 Chart theme·HTML cards<br/>designer"):::work
  T335("○ T-335 Contract·prompts·phone format<br/>backend"):::next
  T337("○ T-337 HTML card render<br/>backend"):::next
  T338("○ T-338 Desktop display<br/>frontend"):::next
  T339("○ T-339 iOS·Android display<br/>mobile"):::next
  T340("○ T-340 Security review<br/>security"):::next
  T335 --> T337
  T335 --> T338
  T335 --> T339
  T336 --> T337
  T337 --> T340
  classDef next fill:${c('surface')},stroke:${c('border-strong')},stroke-width:1.5px,color:${c('text')}
  classDef work ${st('work').replace(/^/, '')}
  linkStyle default stroke:${c('muted')},stroke-width:1.5px`;

  const B = `flowchart LR
  subgraph L1[designer]
    T336("● T-336 Chart theme·HTML cards"):::work
  end
  subgraph L2[backend]
    T335("○ T-335 Contract·prompts"):::next
    T337("○ T-337 HTML card render"):::next
  end
  subgraph L3[frontend · mobile]
    T338("○ T-338 Desktop display"):::next
    T339("○ T-339 iOS·Android display"):::next
  end
  subgraph L4[security]
    T340("○ T-340 Security review"):::next
  end
  T335 --> T337
  T335 --> T338
  T335 --> T339
  T336 --> T337
  T337 --> T340
  classDef next fill:${c('surface')},stroke:${c('border-strong')},stroke-width:1px,color:${c('text')}
  classDef work ${st('work')}
  linkStyle default stroke:${c('muted')},stroke-width:1.5px`;

  const C = `flowchart LR
  T336(["● T-336 Chart theme·cards"]):::focus
  T335(["T-335 Contract"]):::next
  T337(["T-337 Card render"]):::next
  T338(["T-338 Desktop"]):::next
  T339(["T-339 iOS·Android"]):::next
  T340(["T-340 Security review"]):::next
  T335 --> T337
  T335 --> T338
  T335 --> T339
  T336 ==> T337
  T337 ==> T340
  classDef next fill:${c('bg')},stroke:${c('border-strong')},stroke-width:1px,color:${c('text')}
  classDef focus fill:${c('action')},stroke:${c('action')},color:${c('on-action')}
  linkStyle 0,1,2 stroke:${c('border-strong')},stroke-width:1px
  linkStyle 3,4 stroke:${c('accent')},stroke-width:3px`;

  for (const [k, src, layout] of [['a', A, { nodeSpacing: 28, rankSpacing: 56, nodePaddingX: 22, nodePaddingY: 14, maxLabelWidthChars: 30 }], ['b', B, { nodeSpacing: 24, rankSpacing: 48, nodePaddingX: 18, nodePaddingY: 12, maxLabelWidthChars: 30 }], ['c', C, { nodeSpacing: 22, rankSpacing: 44, nodePaddingX: 20, nodePaddingY: 10 }]]) {
    const f = path.join(out, `${k}-${mode}`);
    fs.writeFileSync(f + '.mmd', src); fs.writeFileSync(f + '.theme.json', JSON.stringify({ base: mode, fields: base, layout }));
    execFileSync(MMR, [f + '.mmd', f + '.theme.json', f + '.svg']);
  }
}
console.log('ok');
