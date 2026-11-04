// `equipe design flows record`: a flow walked through as a person would —
// its steps at a human pace, typing visibly, a caption naming the screen —
// through the Mac's own Chrome, headless, its frames caught as Chrome paints
// them (CDP screencast). The plan names the steps (a module `equipe` writes
// from flows.json), the viewport and where the frames go; what it saw is
// written to `raw`: { start, end, frames: [{ file, t }], marks: [{ screen, t }] },
// times in Unix ms. equipe makes the video from that.
import { chromium } from 'playwright';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const at = process.argv.indexOf('--plan');
if (at < 0) {
  console.error('usage: record --plan <plan.json>');
  process.exit(2);
}
const { walk, frames, raw, width, height, scale, scheme, pace = 650, typing = 55 } = JSON.parse(readFileSync(process.argv[at + 1], 'utf8'));
mkdirSync(frames, { recursive: true });
const walkthrough = (await import(pathToFileURL(walk).href)).default;

// The caption: the screen's title, and what the walk says there.
const PAINT = ({ title, say }) => {
  let box = document.getElementById('__equipe_caption');
  if (!box) {
    box = document.createElement('div');
    box.id = '__equipe_caption';
    box.style.cssText = 'position:fixed;left:50%;bottom:16px;transform:translateX(-50%);z-index:2147483647;pointer-events:none;max-width:calc(100% - 32px);box-sizing:border-box;padding:8px 14px;border-radius:12px;background:rgba(18,18,20,.8);border:1px solid rgba(255,255,255,.22);color:#fff;font:500 13px/1.4 -apple-system,BlinkMacSystemFont,"Apple SD Gothic Neo","Hiragino Sans","PingFang SC",sans-serif;text-align:center;box-shadow:0 2px 12px rgba(0,0,0,.25)';
    document.documentElement.appendChild(box);
  }
  box.replaceChildren();
  for (const [text, style] of [[title, 'font-weight:600'], [say, 'opacity:.85;font-weight:400']]) {
    if (!text) continue;
    const line = document.createElement('div');
    line.style.cssText = style;
    line.textContent = text;
    box.appendChild(line);
  }
  box.style.display = title || say ? 'block' : 'none';
};
// Where a click lands, shown for a moment.
const RING = ({ x, y }) => {
  const ring = document.createElement('div');
  ring.style.cssText = `position:fixed;left:${x - 14}px;top:${y - 14}px;width:28px;height:28px;border-radius:50%;border:2px solid rgba(80,140,255,.95);background:rgba(80,140,255,.25);z-index:2147483646;pointer-events:none;transition:transform .4s,opacity .4s`;
  document.documentElement.appendChild(ring);
  requestAnimationFrame(() => requestAnimationFrame(() => {
    ring.style.transform = 'scale(1.7)';
    ring.style.opacity = '0';
  }));
  setTimeout(() => ring.remove(), 600);
};

const shots = [];
const marks = [];
let caption = { title: '', say: '' };
let start = 0;
let end = 0;
const browser = await chromium.launch({ channel: 'chrome', headless: true });
try {
  const context = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: scale, colorScheme: scheme });
  const page = await context.newPage();
  const paint = () => page.evaluate(PAINT, caption).catch(() => {});
  page.on('domcontentloaded', paint);
  page.on('load', paint);
  const cdp = await context.newCDPSession(page);
  cdp.on('Page.screencastFrame', ({ data, sessionId }) => {
    const file = join(frames, `f${String(shots.length + 1).padStart(5, '0')}.jpg`);
    writeFileSync(file, Buffer.from(data, 'base64'));
    shots.push({ file, t: Date.now() });
    cdp.send('Page.screencastFrameAck', { sessionId }).catch(() => {});
  });
  // Filming starts at the first step that shows something, so a video
  // doesn't open on a blank page.
  const roll = async () => {
    if (start) return;
    await cdp.send('Page.startScreencast', { format: 'jpeg', quality: 85, everyNthFrame: 1, maxWidth: width * scale, maxHeight: height * scale });
    start = Date.now();
    await paint();
  };
  const settle = () => page.waitForLoadState('networkidle', { timeout: 8000 }).catch(() => {});
  const rest = (ms = pace) => page.waitForTimeout(ms);
  const target = (selector) => page.locator(selector).first();
  await walkthrough({
    screen: async (id, title) => {
      marks.push({ screen: id, t: Date.now() });
      caption = { title, say: '' };
      await paint();
    },
    go: async (url) => {
      await page.goto(url, { waitUntil: 'load', timeout: 45000 });
      await settle();
      await roll();
      await paint();
      await rest();
    },
    click: async (selector) => {
      await roll();
      const el = target(selector);
      await el.scrollIntoViewIfNeeded();
      const box = await el.boundingBox();
      if (box) {
        await page.evaluate(RING, { x: box.x + box.width / 2, y: box.y + box.height / 2 });
        await rest(250);
      }
      await el.click();
      await settle();
      await paint();
      await rest();
    },
    fill: async (selector, text) => {
      await roll();
      const el = target(selector);
      await el.click();
      await el.fill('');
      await el.pressSequentially(text, { delay: typing });
      await rest();
    },
    press: async (key) => {
      await roll();
      await page.keyboard.press(key);
      await settle();
      await paint();
      await rest();
    },
    wait: async (ms) => {
      await roll();
      await page.waitForTimeout(ms);
    },
    say: async (text) => {
      await roll();
      caption = { ...caption, say: text };
      await paint();
      await rest(Math.max(1600, text.length * 55));
    },
  });
  await roll();
  await rest(1200);
  end = Date.now();
  await cdp.send('Page.stopScreencast').catch(() => {});
  await context.close();
} catch (err) {
  console.error(String((err && err.message) || err).split('\n')[0]);
  process.exitCode = 1;
} finally {
  await browser.close();
}
if (!process.exitCode) writeFileSync(raw, JSON.stringify({ start, end, frames: shots, marks }));
