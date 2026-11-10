// The rename card in all eight languages — the longest copy (save failed, too long), at 100% and 150%, 760 wide.
//   node design/specs/rename-conversation/langs.mjs  → langs/<lang>.<state>.<zoom>.png and langs-sheet.png
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';
const require = createRequire(path.join(os.homedir(), '.equipe/design-kit/package.json'));
const { chromium } = require('playwright');
const here = path.dirname(fileURLToPath(import.meta.url));
const out = path.join(here, 'langs');
fs.mkdirSync(out, { recursive: true });
const LANGS = ['ko', 'en', 'ja', 'zh-Hans', 'zh-Hant', 'de', 'fr', 'es'];
const browser = await chromium.launch();
for (const lang of LANGS) for (const [state, zoom] of [['failed', 1], ['too-long', 1], ['empty', 1], ['large-failed', 1.5]]) {
  const vw = Math.round(760 / zoom), vh = Math.round(760 / zoom);
  const page = await browser.newPage({ viewport: { width: vw, height: vh }, deviceScaleFactor: 760 / vw, locale: lang });
  await page.goto(pathToFileURL(path.join(here, 'prototype.html')).href + `?state=${state}&lang=${lang}&mode=light&w=760&zoom=${zoom}`);
  await page.waitForFunction(() => document.body.dataset.ready === '1' && document.getElementById('base').complete);
  const card = await page.evaluate(() => { const r = document.querySelector('.card').getBoundingClientRect(); return { y: r.bottom }; });
  // From the bar's left (x 260 at 100%) to the window's right, the bar and the card.
  const x0 = zoom === 1 ? 260 : 0;
  await page.screenshot({ path: path.join(out, `${lang}.${state}.png`), clip: { x: x0, y: 0, width: vw - x0, height: Math.min(vh, card.y + 12) } });
  await page.close();
}
await browser.close();
execFileSync('python3', ['-c', `
from PIL import Image, ImageDraw, ImageFont
import os
L=${JSON.stringify(LANGS)}; S=['failed','too-long','empty','large-failed']
d='${out}'
cols=[[Image.open(os.path.join(d,f'{l}.{s}.png')) for s in S] for l in L]
cw=[max(c[i].width for c in cols) for i in range(4)]
rh=[max(im.height for im in c) for c in cols]
pad=16; lab=90
W=lab+sum(cw)+pad*5; H=sum(rh)+pad*(len(L)+1)
sheet=Image.new('RGB',(W,H),(250,249,246)); dr=ImageDraw.Draw(sheet)
f=ImageFont.truetype('/System/Library/Fonts/SFNS.ttf',18)
y=pad
for l,c,h in zip(L,cols,rh):
    dr.text((pad,y+8),l,fill=(98,96,90),font=f)
    x=lab+pad
    for i,im in enumerate(c):
        sheet.paste(im,(x,y)); x+=cw[i]+pad
    y+=h+pad
sheet.save(os.path.join('${here}','langs-sheet.png'),optimize=True)
print(sheet.size)
`], { stdio: 'inherit' });
