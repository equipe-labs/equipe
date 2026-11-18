// `equipe design session show`'s look at a page before it is sent: opened as
// the person's screen opens it (the page server's address), through the Mac's
// own Chrome, headless — what failed to load, whether its styles applied,
// whether it shows anything, pictures drawn far larger than the window — and
// photographed. Ends with one line: `equipe-page-check: {json}`
// (`equipe_core::design::page_check::Seen`).
import { chromium } from 'playwright';

const [url, flag, out] = process.argv.slice(2);
if (!url || flag !== '--out' || !out) {
  console.error('usage: pagecheck <url> --out <picture.png>');
  process.exit(2);
}
const seen = { failed: [], error: null };
const browser = await chromium.launch({ channel: 'chrome', headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  page.on('requestfailed', (r) => seen.failed.push(`${r.url()} (${r.failure()?.errorText ?? 'failed'})`));
  page.on('response', (r) => {
    if (r.status() >= 400) seen.failed.push(`${r.url()} (${r.status()})`);
  });
  try {
    await page.goto(url, { waitUntil: 'load', timeout: 30000 });
    await page.waitForLoadState('networkidle', { timeout: 5000 }).catch(() => {});
    await page.waitForTimeout(300);
    Object.assign(
      seen,
      await page.evaluate(() => {
        const [vw, vh] = [window.innerWidth, window.innerHeight];
        let rules = 0;
        for (const sheet of document.styleSheets) {
          try {
            rules += sheet.cssRules.length;
          } catch {
            rules += 1; // another origin's: it loaded
          }
        }
        const linked = document.querySelectorAll('link[rel~="stylesheet"], style').length;
        const body = document.body || document.documentElement;
        const room = (e) => {
          const r = e.getBoundingClientRect();
          return r.width > 4 && r.height > 4;
        };
        const huge = [...document.querySelectorAll('svg, img')]
          .filter((e) => !(e.tagName.toLowerCase() === 'svg' && e.parentElement && e.parentElement.closest('svg')))
          .filter((e) => {
            const r = e.getBoundingClientRect();
            return r.width > vw * 1.5 || r.height > vh * 1.5;
          })
          .map((e) => (e.tagName.toLowerCase() === 'img' ? e.getAttribute('src') || 'img' : e.id ? `svg#${e.id}` : e.getAttribute('class') ? `svg.${e.getAttribute('class').split(/\s+/)[0]}` : 'svg'))
          .slice(0, 5);
        return {
          linked,
          rules,
          font: getComputedStyle(body).fontFamily,
          text: (body.innerText || '').trim().length,
          visible: [...document.querySelectorAll('img, svg, canvas, video, picture')].filter(room).length,
          huge,
        };
      }),
    );
  } catch (e) {
    seen.error = String(e?.message ?? e).split('\n')[0];
  }
  await page.screenshot({ path: out }).catch(() => {});
} finally {
  await browser.close();
}
console.log('equipe-page-check: ' + JSON.stringify(seen));
