// node preview.mjs <dir with <name>-light.svg/-dark.svg> <outdir> name… — each diagram in the block frame, light and dark → <outdir>/<name>.html
import fs from 'node:fs'; import path from 'node:path';
const [dir, out, ...names] = process.argv.slice(2); fs.mkdirSync(out, { recursive: true });
const tokens = fs.readFileSync(new URL('../../build/css/tokens.css', import.meta.url), 'utf8').replace(/@media \(prefers-color-scheme: dark\) \{[\s\S]*?\n\}\n/, '').replace(':root[data-theme="dark"]', '[data-theme="dark"]');
for (const n of names) {
  const panel = (m) => `<section data-theme="${m}" class="p"><figure class="blk"><div class="tools" aria-hidden="true"><span>원문</span><span>복사</span></div>${fs.readFileSync(path.join(dir, `${n}-${m}.svg`), 'utf8').replace(/<\?xml[^>]*>/, '')}</figure></section>`;
  fs.writeFileSync(path.join(out, `${n}.html`), `<!doctype html><html lang="ko"><head><meta charset="utf-8"><style>${tokens}
body{margin:0;font:var(--eq-type-ui);background:#0A0A0C}
.p{background:var(--eq-color-bg);color:var(--eq-color-text);padding:var(--eq-space-3)}
.blk{position:relative;margin:0;background:var(--eq-color-surface);border:1px solid var(--eq-color-line);border-radius:var(--eq-radius-card);padding:var(--eq-space-7) var(--eq-space-4) var(--eq-space-4)}
.blk svg{display:block;width:auto;height:auto;max-width:100%;margin:0 auto}
.tools{position:absolute;top:var(--eq-space-2);right:var(--eq-space-2);display:flex;gap:2px;font:var(--eq-type-meta);color:var(--eq-color-muted)}
.tools span{padding:2px 8px;border-radius:var(--eq-radius-sm);background:var(--eq-color-sunken)}
</style></head><body>${panel('light')}${panel('dark')}</body></html>`);
}
