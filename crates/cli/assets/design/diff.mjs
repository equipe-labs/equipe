// `equipe design diff`: a spec or mockup and what was built, side by side,
// and where they differ — the two scaled to one width, every pixel compared,
// the differing ones lit up over a faded copy of the built screen. One sheet,
// and the share of the picture that differs. Drawn in the Mac's own Chrome.
import { chromium } from 'playwright';
import { readFileSync } from 'node:fs';
import { resolve, extname } from 'node:path';

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const at = args.indexOf(name);
  if (at < 0) return fallback;
  const value = args[at + 1];
  args.splice(at, 2);
  return value;
};
const out = resolve(opt('--out', 'diff.png'));
const [spec, built] = args;
if (!spec || !built) {
  console.error('usage: diff <spec.png> <built.png> [--out diff.png]');
  process.exit(2);
}
const data = (file) => {
  const type = { '.png': 'image/png', '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.webp': 'image/webp' }[extname(file).toLowerCase()] || 'image/png';
  return `data:${type};base64,${readFileSync(resolve(file)).toString('base64')}`;
};
const browser = await chromium.launch({ channel: 'chrome', headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1600, height: 900 }, deviceScaleFactor: 1 });
  const share = await page.evaluate(async ([a, b]) => {
    const load = (src) => new Promise((ok, no) => { const i = new Image(); i.onload = () => ok(i); i.onerror = no; i.src = src; });
    const [ia, ib] = await Promise.all([load(a), load(b)]);
    // One width — the narrower — the heights scaled with it.
    const w = Math.min(ia.naturalWidth, ib.naturalWidth, 1400);
    const ha = Math.round(ia.naturalHeight * w / ia.naturalWidth);
    const hb = Math.round(ib.naturalHeight * w / ib.naturalWidth);
    const h = Math.max(ha, hb);
    const pixels = (img, ih) => {
      const c = new OffscreenCanvas(w, h);
      const g = c.getContext('2d');
      g.fillStyle = '#fff';
      g.fillRect(0, 0, w, h);
      g.drawImage(img, 0, 0, w, ih);
      return g.getImageData(0, 0, w, h);
    };
    const pa = pixels(ia, ha), pb = pixels(ib, hb);
    const heat = new ImageData(w, h);
    let differ = 0;
    for (let i = 0; i < pa.data.length; i += 4) {
      const d = Math.abs(pa.data[i] - pb.data[i]) + Math.abs(pa.data[i + 1] - pb.data[i + 1]) + Math.abs(pa.data[i + 2] - pb.data[i + 2]);
      const grey = (pb.data[i] + pb.data[i + 1] + pb.data[i + 2]) / 3;
      if (d > 48) {
        differ++;
        heat.data.set([235, 40, 90, 255], i);
      } else {
        const faded = 255 - (255 - grey) * 0.25;
        heat.data.set([faded, faded, faded, 255], i);
      }
    }
    const pad = 24, label = 28;
    const sheet = document.createElement('canvas');
    sheet.width = w * 3 + pad * 4;
    sheet.height = h + pad * 2 + label;
    const g = sheet.getContext('2d');
    g.fillStyle = '#f4f4f2';
    g.fillRect(0, 0, sheet.width, sheet.height);
    g.font = '600 16px -apple-system, sans-serif';
    g.fillStyle = '#222';
    const titles = ['Spec', 'Built', 'Where they differ'];
    [ia, ib].forEach((img, k) => {
      g.fillText(titles[k], pad + k * (w + pad), pad + 16);
      g.drawImage(img, pad + k * (w + pad), pad + label, w, k ? hb : ha);
    });
    g.fillText(titles[2], pad + 2 * (w + pad), pad + 16);
    const c = new OffscreenCanvas(w, h);
    c.getContext('2d').putImageData(heat, 0, 0);
    g.drawImage(c, pad + 2 * (w + pad), pad + label);
    document.body.style.margin = '0';
    document.body.appendChild(sheet);
    return differ / (w * h);
  }, [data(spec), data(built)]);
  const sheet = await page.$('canvas');
  await sheet.screenshot({ path: out });
  console.log(`${out} — ${(share * 100).toFixed(1)}% of the picture differs`);
} finally {
  await browser.close();
}
