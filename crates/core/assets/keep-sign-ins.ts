// equipe: a member's browser hands what it signs in to back to the squad, and
// takes one of the squad's browser places while it is open.
// Playwright MCP runs this on every page (--init-page); STATE, SLOTS and MAX are
// filled in by equipe. Cookies and local storage are merged into the shared
// sign-ins, newest wins, a second after each page loads, every few seconds, and
// when the browser closes; others' new sign-ins are taken up at each page. At most MAX browsers run at once: a browser opening while all places
// are taken waits for one (a few minutes at most, then opens anyway).
import * as fs from 'node:fs';

const STATE: string = __STATE__;
const SLOTS: string = __SLOTS__;
const MAX: number = __MAX__;

// Google's sign-in is the browser's own: it rotates its cookies and ends a
// session it sees in two browsers at once. So Google stays in the browser it was
// signed in to — a member signs in in its own window — and is never shared.
const own = (domain: string) => /(^|\.)(google\.[a-z]{2,3}(\.[a-z]{2})?|youtube\.com|googleusercontent\.com)$/.test(domain.replace(/^\./, ''));
const shared = (c: any) => !own(c.domain || '');
const sharedOrigin = (o: any) => { try { return !own(new URL(o.origin).hostname); } catch { return true; } };

// A browser on the person's screen shows who is driving it, as Claude in Chrome
// does: a still glow along its edge (one that pulsed redrew the whole window
// every frame, a flicker over screen sharing), a label, and the member's pointer gliding
// to where it clicks. Drawn in a closed shadow root that no click, page style
// or accessibility snapshot reaches; built without HTML strings or inline
// <style>, which strict pages (Trusted Types, CSP) refuse.
const argv = process.argv;
const onScreen = !argv.includes('--headless');
const profileAt = argv.indexOf('--user-data-dir');
const member = profileAt >= 0 ? (argv[profileAt + 1] || '').split('/').filter(Boolean).pop() || '' : '';

function driving({ member }: { member: string }) {
  if (window.top !== window || (window as any).__equipeDriving) return;
  (window as any).__equipeDriving = true;
  const ACCENT = '217, 119, 87';
  const mount = () => {
    if (!document.documentElement) return;
    const host = document.createElement('equipe-driving');
    host.setAttribute('aria-hidden', 'true');
    host.style.cssText = 'all: initial; position: fixed; inset: 0; pointer-events: none; z-index: 2147483647;';
    const root = host.attachShadow({ mode: 'closed' });
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(`
      * { box-sizing: border-box; pointer-events: none; }
      .glow { position: fixed; inset: 0; box-shadow: inset 0 0 0 2px rgba(${ACCENT}, .5), inset 0 0 28px 3px rgba(${ACCENT}, .22); }
      .label { position: fixed; top: 10px; left: 50%; transform: translateX(-50%); display: flex; align-items: center; gap: 7px; padding: 5px 12px 5px 10px; border-radius: 999px; background: rgba(24, 24, 27, .84); color: #fafafa; font: 500 12px/1.3 -apple-system, BlinkMacSystemFont, system-ui, sans-serif; letter-spacing: .01em; box-shadow: 0 2px 12px rgba(0, 0, 0, .25); white-space: nowrap; }
      .dot { width: 7px; height: 7px; border-radius: 50%; background: rgb(${ACCENT}); box-shadow: 0 0 8px rgb(${ACCENT}); animation: breathe 1.6s ease-in-out infinite; }
      .pointer { position: fixed; left: 0; top: 0; width: 24px; height: 24px; opacity: 0; transition: transform .5s cubic-bezier(.2, .8, .2, 1), opacity .25s ease; filter: drop-shadow(0 1px 2px rgba(0, 0, 0, .35)) drop-shadow(0 0 6px rgba(${ACCENT}, .6)); }
      .ripple { position: fixed; left: 0; top: 0; width: 30px; height: 30px; margin: -15px 0 0 -15px; border-radius: 50%; border: 2px solid rgb(${ACCENT}); opacity: 0; }
      .ripple.on { animation: ripple .55s ease-out; }
      @keyframes breathe { 0%, 100% { opacity: .6; } 50% { opacity: 1; } }
      @keyframes ripple { from { transform: scale(.35); opacity: .9; } to { transform: scale(1.7); opacity: 0; } }
      @media (prefers-reduced-motion: reduce) { .dot { animation: none; } .pointer { transition: opacity .25s ease; } }
    `);
    (root as any).adoptedStyleSheets = [sheet];
    const el = (tag: string, cls: string) => {
      const e = document.createElement(tag);
      e.className = cls;
      return root.appendChild(e);
    };
    el('div', 'glow');
    const label = el('div', 'label');
    const dot = document.createElement('span');
    dot.className = 'dot';
    const who = document.createElement('span');
    who.textContent = member ? `${member} is working in this browser` : 'equipe is working in this browser';
    label.append(dot, who);
    const svg = 'http://www.w3.org/2000/svg';
    const pointer = document.createElementNS(svg, 'svg');
    pointer.setAttribute('class', 'pointer');
    pointer.setAttribute('viewBox', '0 0 24 24');
    const arrow = document.createElementNS(svg, 'path');
    arrow.setAttribute('d', 'M5 3l14 8.2-6.1 1.5-3.3 6.1z');
    arrow.setAttribute('fill', `rgb(${ACCENT})`);
    arrow.setAttribute('stroke', '#fff');
    arrow.setAttribute('stroke-width', '1.6');
    arrow.setAttribute('stroke-linejoin', 'round');
    pointer.appendChild(arrow);
    root.appendChild(pointer);
    const ripple = el('div', 'ripple');
    document.documentElement.appendChild(host);
    // A page that rewrites its document, or a framework that clears <html>, takes
    // it away: it comes back.
    setInterval(() => { if (!host.isConnected && document.documentElement) document.documentElement.appendChild(host); }, 1500);

    // The member's pointer jumps where it acts — one move, then the click — while
    // a hand on a mouse sends a stream of small moves: those are the person's own,
    // with their own pointer, and the member's is put away.
    let last = { x: -1, y: -1, t: 0 };
    let shown = false;
    let hide: any;
    const put = (x: number, y: number) => { pointer.style.transform = `translate(${x - 5}px, ${y - 3}px)`; };
    addEventListener('mousemove', (e) => {
      const now = performance.now();
      const jump = Math.hypot(e.clientX - last.x, e.clientY - last.y);
      const member = now - last.t > 160 && jump > 24;
      last = { x: e.clientX, y: e.clientY, t: now };
      clearTimeout(hide);
      if (member) {
        shown = true;
        pointer.style.opacity = '1';
        put(e.clientX, e.clientY);
        hide = setTimeout(() => { shown = false; pointer.style.opacity = '0'; }, 6000);
      } else if (shown) {
        shown = false;
        pointer.style.opacity = '0';
      }
    }, true);
    addEventListener('mousedown', (e) => {
      if (!shown) return;
      ripple.style.transform = '';
      ripple.style.left = `${e.clientX}px`;
      ripple.style.top = `${e.clientY}px`;
      ripple.classList.remove('on');
      void ripple.offsetWidth;
      ripple.classList.add('on');
    }, true);
  };
  if (document.body) mount();
  else addEventListener('DOMContentLoaded', mount, { once: true });
}

const alive = (pid: number) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};

// A place is a file holding the pid of the server that has it; a dead pid frees it.
async function takePlace(): Promise<string | undefined> {
  fs.mkdirSync(SLOTS, { recursive: true });
  for (let waited = 0; waited < 5 * 60_000; waited += 2000) {
    for (let i = 0; i < MAX; i++) {
      const file = `${SLOTS}/${i}`;
      let holder = 0;
      try { holder = Number(fs.readFileSync(file, 'utf8')); } catch {}
      if (holder === process.pid) return file;
      if (holder && alive(holder)) continue;
      fs.writeFileSync(file, String(process.pid));
      await new Promise(r => setTimeout(r, 50));
      try { if (Number(fs.readFileSync(file, 'utf8')) === process.pid) return file; } catch {}
    }
    await new Promise(r => setTimeout(r, 2000));
  }
  return undefined;
}

function leave(file: string | undefined) {
  if (!file) return;
  try { if (Number(fs.readFileSync(file, 'utf8')) === process.pid) fs.unlinkSync(file); } catch {}
}

export default async ({ page }: { page: any }) => {
  const context = page.context();
  if (context.__equipe) return;
  context.__equipe = true;
  if (onScreen) {
    await context.addInitScript(driving, { member }).catch(() => {});
    // The page open already gets it now; every later page and navigation, from the script.
    for (const p of context.pages()) await p.evaluate(driving, { member }).catch(() => {});
  }
  // Start from the squad's sign-ins — the person's, and what teammates signed in
  // to — and take up new ones as they come: a sign-in the person makes in one
  // member's window reaches the others at their next page.
  let seen = 0;
  const take = async () => {
    try {
      const changed = fs.statSync(STATE).mtimeMs;
      if (changed === seen) return;
      seen = changed;
      const kept = JSON.parse(fs.readFileSync(STATE, 'utf8'));
      const now = Date.now() / 1000;
      await context.addCookies((kept.cookies || []).filter((c: any) => shared(c) && (c.expires === -1 || c.expires > now)));
    } catch {}
  };
  await take();
  const place = await takePlace();
  process.on('exit', () => leave(place));
  const save = async () => {
    const mine = await context.storageState();
    let kept = { cookies: [] as any[], origins: [] as any[] };
    try { kept = JSON.parse(fs.readFileSync(STATE, 'utf8')); } catch {}
    const key = (c: any) => `${c.name}\u0000${c.domain}\u0000${c.path}`;
    const cookies = new Map(kept.cookies.filter(shared).map((c: any) => [key(c), c]));
    for (const c of mine.cookies.filter(shared)) cookies.set(key(c), c);
    const origins = new Map(kept.origins.filter(sharedOrigin).map((o: any) => [o.origin, o]));
    for (const o of mine.origins.filter(sharedOrigin)) origins.set(o.origin, o);
    const tmp = `${STATE}.${process.pid}`;
    fs.writeFileSync(tmp, JSON.stringify({ cookies: [...cookies.values()], origins: [...origins.values()] }), { mode: 0o600 });
    fs.renameSync(tmp, STATE);
    // Its own write isn't news to take up again.
    try { seen = fs.statSync(STATE).mtimeMs; } catch {}
  };
  let timer: any;
  const soon = () => {
    clearTimeout(timer);
    timer = setTimeout(() => save().catch(() => {}), 1000);
  };
  const loaded = () => {
    take().catch(() => {});
    soon();
  };
  context.on('page', (p: any) => p.on('load', loaded));
  page.on('load', loaded);
  // A sign-in finishes on a page that may never load again — the person types
  // a code and waits: kept every few seconds too, so the window needn't close.
  const every = setInterval(() => save().catch(() => {}), 5000);
  context.on('close', () => clearInterval(every));
  context.on('close', () => {
    save().catch(() => {}).finally(() => leave(place));
  });
};
