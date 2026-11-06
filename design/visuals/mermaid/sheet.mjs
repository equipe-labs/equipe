// Puts each drawn diagram into the proposed block frame (no code-block header; Source/Copy appear on hover or focus) → one HTML per option, light and dark panels.
// node sheet.mjs <dir with a-light.svg…> <outdir>
import fs from 'node:fs'; import path from 'node:path';
const [dir, out] = process.argv.slice(2); fs.mkdirSync(out, { recursive: true });
const tokens = fs.readFileSync(new URL('../../build/css/tokens.css', import.meta.url), 'utf8').replace(/@media \(prefers-color-scheme: dark\) \{[\s\S]*?\n\}\n/, '').replace(':root[data-theme="dark"]', '[data-theme="dark"]');
const NAMES = { a: ['A', 'Cards', 'Rounded cards in status colours, the role on the second line'], b: ['B', 'Lanes', 'Grouped by role, one lane each'], c: ['C', 'Pills', 'Only the path that matters now in bold'] };
for (const k of ['a', 'b', 'c']) {
  const panel = (m) => `<section data-theme="${m}" class="p"><p class="cap">${m === 'light' ? 'Light' : 'Dark'}</p><figure class="blk"><div class="tools" aria-hidden="true"><span>Source</span><span>Copy</span></div>${fs.readFileSync(path.join(dir, `${k}-${m}.svg`), 'utf8').replace(/<\?xml[^>]*>/, '')}</figure><p class="alt">T-336 – T-340 flow</p></section>`;
  fs.writeFileSync(path.join(out, `option-${k}.html`), `<!doctype html><html lang="en"><head><meta charset="utf-8"><style>${tokens}
body{margin:0;font:var(--eq-type-ui);-webkit-font-smoothing:antialiased;background:#888}
.p{background:var(--eq-color-bg);color:var(--eq-color-text);padding:var(--eq-space-4) var(--eq-space-5) var(--eq-space-5)}
.cap{font:var(--eq-type-meta);color:var(--eq-color-muted);margin:0 0 var(--eq-space-2);letter-spacing:.06em}
h1{font:var(--eq-type-title);margin:0;padding:var(--eq-space-4) var(--eq-space-5) 0;background:#fff;color:#15161A}
h1 span{font:var(--eq-type-small);color:#4F5159;margin-left:8px;font-weight:400}
.blk{position:relative;margin:0;background:var(--eq-color-surface);border:1px solid var(--eq-color-line);border-radius:var(--eq-radius-card);padding:var(--eq-space-7) var(--eq-space-4) var(--eq-space-4)}
.blk svg{display:block;width:auto;height:auto;margin:0 auto;max-width:100%}
.tools{position:absolute;top:var(--eq-space-2);right:var(--eq-space-2);display:flex;gap:2px;font:var(--eq-type-meta);color:var(--eq-color-muted)}
.tools span{padding:2px 8px;border-radius:var(--eq-radius-sm);background:var(--eq-color-sunken)}
.alt{font:var(--eq-type-meta);color:var(--eq-color-muted);margin:var(--eq-space-2) 0 0}
</style></head><body><h1>${NAMES[k][0]} · ${NAMES[k][1]}<span>${NAMES[k][2]}</span></h1>${panel('light')}${panel('dark')}</body></html>`);
}
