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
// does: a glow along its edge and a label, in the member's own colour (the one
// it has in the console), and an outline on what it clicks or types into. A
// member doesn't steer a pointer — it picks an element and Playwright acts on it —
// so what it acts on is what's shown. The glow stands still: one that pulsed
// redrew the whole window every frame, a flicker over screen sharing. Drawn in
// a closed shadow root that no click, page style or accessibility snapshot
// reaches; built without HTML strings or inline <style>, which strict pages
// (Trusted Types, CSP) refuse.
const argv = process.argv;
const onScreen = !argv.includes('--headless');
const profileAt = argv.indexOf('--user-data-dir');
const member = profileAt >= 0 ? (argv[profileAt + 1] || '').split('/').filter(Boolean).pop() || '' : '';

// The console's colours by role (`frontend-2` is a frontend); anyone else's by name.
const ROLE_HUES: Record<string, number> = { pm: 0, designer: 1, frontend: 2, backend: 3, qa: 4, data: 5, mobile: 6 };
const PALETTE = ['232, 190, 120', '236, 150, 190', '120, 190, 240', '140, 208, 150', '182, 162, 242', '110, 212, 200', '240, 160, 110', '205, 212, 120'];
const roleOf = (name: string) => name.replace(/-\d+$/, '');
const hueOf = (name: string) => {
  const role = roleOf(name);
  if (role in ROLE_HUES) return ROLE_HUES[role];
  let h = 7;
  for (const b of Buffer.from(role)) h = (h * 31 + b) >>> 0;
  return 6 + (h % 2);
};
const color = PALETTE[hueOf(member) % PALETTE.length];

function driving({ member, color }: { member: string; color: string }) {
  if (window.top !== window || (window as any).__equipeDriving) return;
  (window as any).__equipeDriving = true;
  const mount = () => {
    if (!document.documentElement) return;
    const host = document.createElement('equipe-driving');
    host.setAttribute('aria-hidden', 'true');
    host.style.cssText = 'all: initial; position: fixed; inset: 0; pointer-events: none; z-index: 2147483647;';
    const root = host.attachShadow({ mode: 'closed' });
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(`
      * { box-sizing: border-box; pointer-events: none; }
      .glow { position: fixed; inset: 0; box-shadow: inset 0 0 0 4px rgba(${color}, .9), inset 0 0 0 5px rgba(0, 0, 0, .12), inset 0 0 60px 10px rgba(${color}, .38); }
      .label { position: fixed; top: 12px; left: 50%; transform: translateX(-50%); display: flex; align-items: center; gap: 7px; padding: 5px 12px 5px 10px; border-radius: 999px; background: rgba(24, 24, 27, .86); color: #fafafa; font: 500 12px/1.3 -apple-system, BlinkMacSystemFont, system-ui, sans-serif; letter-spacing: .01em; box-shadow: 0 0 0 1px rgba(${color}, .7), 0 2px 12px rgba(0, 0, 0, .25); white-space: nowrap; }
      .dot { width: 8px; height: 8px; border-radius: 50%; background: rgb(${color}); box-shadow: 0 0 8px rgb(${color}); animation: breathe 1.6s ease-in-out infinite; }
      .target { position: fixed; border-radius: 6px; border: 2px solid rgb(${color}); background: rgba(${color}, .12); box-shadow: 0 0 0 4px rgba(${color}, .25); opacity: 0; transition: opacity .4s ease; }
      .target.on { opacity: 1; transition: opacity .08s ease; }
      @keyframes breathe { 0%, 100% { opacity: .55; } 50% { opacity: 1; } }
      @media (prefers-reduced-motion: reduce) { .dot { animation: none; } }
    `);
    (root as any).adoptedStyleSheets = [sheet];
    const el = (cls: string) => {
      const e = document.createElement('div');
      e.className = cls;
      return root.appendChild(e);
    };
    el('glow');
    const label = el('label');
    const dot = document.createElement('span');
    dot.className = 'dot';
    const who = document.createElement('span');
    who.textContent = member ? `${member} is working in this browser` : 'equipe is working in this browser';
    label.append(dot, who);
    const target = el('target');
    document.documentElement.appendChild(host);
    // A page that rewrites its document, or a framework that clears <html>, takes
    // it away: it comes back.
    setInterval(() => { if (!host.isConnected && document.documentElement) document.documentElement.appendChild(host); }, 1500);

    // What the member acts on, outlined a moment. A member's click comes as one
    // jump of the pointer and the press; its typing focuses a field with no hand
    // on the mouse. The person's own hand sends a stream of small moves first —
    // what they do isn't outlined.
    // Nothing has moved yet: not the hand, not the pointer.
    let last = { x: -1, y: -1, t: -1e9 };
    let hand = -1e9;
    let fade: any;
    const outline = (node: EventTarget | null) => {
      const e = node instanceof Element ? (node.closest('a, button, input, textarea, select, label, [role], [contenteditable]') || node) : null;
      if (!e || e === document.body || e === document.documentElement) return;
      const r = e.getBoundingClientRect();
      if (r.width === 0 || r.height === 0 || r.width * r.height > innerWidth * innerHeight * 0.5) return;
      Object.assign(target.style, { left: `${r.left - 4}px`, top: `${r.top - 4}px`, width: `${r.width + 8}px`, height: `${r.height + 8}px` });
      target.classList.add('on');
      clearTimeout(fade);
      fade = setTimeout(() => target.classList.remove('on'), 1200);
    };
    addEventListener('mousemove', (e) => {
      const now = performance.now();
      const jump = now - last.t > 160 && Math.hypot(e.clientX - last.x, e.clientY - last.y) > 24;
      if (!jump) hand = now;
      last = { x: e.clientX, y: e.clientY, t: now };
    }, true);
    addEventListener('mousedown', (e) => {
      if (performance.now() - hand > 400) outline(e.target);
    }, true);
    addEventListener('focusin', (e) => {
      if (performance.now() - hand > 1000 && performance.now() - last.t > 300) outline(e.target);
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
    await context.addInitScript(driving, { member, color }).catch(() => {});
    // The page open already gets it now; every later page and navigation, from the script.
    for (const p of context.pages()) await p.evaluate(driving, { member, color }).catch(() => {});
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
