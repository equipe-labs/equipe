// Draws the rename states of prototype.html into the UI map (design/flows/conversation/rename/).
//   node design/specs/rename-conversation/render.mjs [--only <state>] [--out <dir>]
// Playwright comes from equipe's design kit (~/.equipe/design-kit).
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';

const require = createRequire(path.join(os.homedir(), '.equipe/design-kit/package.json'));
const { chromium } = require('playwright');
const here = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const only = args.includes('--only') ? args[args.indexOf('--only') + 1] : '';
const out = args.includes('--out') ? args[args.indexOf('--out') + 1] : path.resolve(here, '../../flows/conversation/rename');

// state → [lang, zoom]
const STATES = {
  'entry-title': ['ko', 1], 'entry-menu': ['ko', 1], 'entry-sidebar': ['ko', 1],
  'editing': ['ko', 1], 'composing': ['ko', 1], 'empty': ['ko', 1], 'too-long': ['ko', 1],
  'saved': ['ko', 1], 'failed': ['ko', 1], 'long': ['de', 1], 'large': ['ko', 1.5], 'large-failed': ['en', 1.5],
};

import { execFileSync } from 'node:child_process';
const cut = (file, w, h) => execFileSync('python3', ['-c', `from PIL import Image\nim=Image.open('${file}')\nif im.size!=(${w},${h}): im.crop((0,0,${w},${h})).save('${file}',optimize=True)`]);
fs.mkdirSync(out, { recursive: true });
const browser = await chromium.launch();
for (const [state, [lang, zoom]] of Object.entries(STATES)) {
  if (only && state !== only) continue;
  for (const w of [1200, 760]) {
    for (const mode of ['light', 'dark']) {
      const vw = Math.round(w / zoom), vh = Math.round(760 / zoom);
      const page = await browser.newPage({ viewport: { width: vw, height: vh }, deviceScaleFactor: w / vw, locale: lang });
      const url = pathToFileURL(path.join(here, 'prototype.html')).href + `?state=${state}&lang=${lang}&mode=${mode}&w=${w}&zoom=${zoom}`;
      await page.goto(url);
      await page.waitForFunction(() => document.body.dataset.ready === '1' && document.getElementById('base').complete);
      await page.evaluate(() => document.fonts.ready);
      const file = path.join(out, `${state}.${mode}.${w}.png`);
      await page.screenshot({ path: file });
      await page.close();
      // 760 at 150% is 506.67 points: the picture is cut to the window's own size.
      cut(file, w, 760);
      console.log(file);
    }
  }
}
await browser.close();
