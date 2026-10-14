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
