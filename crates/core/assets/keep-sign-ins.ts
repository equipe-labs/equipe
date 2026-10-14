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

// The overlay itself, shared with the browser host: see driving.js.
__DRIVING__

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
  // window reaches the others at their next page. Each browser writes back only
  // what it changed, and takes up only what it hasn't changed itself: two
  // browsers writing all they hold, stale copies included, overwrote each
  // other's fresh sessions in turn — a site saw its sign-in flip and sent the
  // page off to sign in again, over and over.
  const key = (c: any) => `${c.name}\u0000${c.domain}\u0000${c.path}`;
  const stamp = (c: any) => `${c.value}\u0000${c.expires}`;
  // What this browser and the file last agreed on, cookie by cookie and origin by origin.
  const agreed = new Map<string, string>();
  const agreedOrigins = new Map<string, string>();
  // What the file held when this browser last read it: a change there since is news.
  const read = new Map<string, string>();
  // What the profile already holds is its own, kept from its earlier runs.
  for (const c of await context.cookies().catch(() => [])) agreed.set(key(c), stamp(c));
  let seen = 0;
  const take = async () => {
    try {
      const changed = fs.statSync(STATE).mtimeMs;
      if (changed === seen) return;
      seen = changed;
      const kept = JSON.parse(fs.readFileSync(STATE, 'utf8'));
      const now = Date.now() / 1000;
      const mine = new Map<string, any>((await context.cookies()).map((c: any) => [key(c), c]));
      const fresh = (kept.cookies || []).filter((c: any) => {
        if (!shared(c) || (c.expires !== -1 && c.expires <= now)) return false;
        const before = read.get(key(c));
        read.set(key(c), stamp(c));
        const have = mine.get(key(c));
        // New to this browser; or changed in the file since it last looked, while
        // its own copy is as it was — someone signed in again elsewhere.
        return !have || (before !== undefined && before !== stamp(c) && stamp(have) !== stamp(c) && agreed.get(key(c)) === stamp(have));
      });
      if (fresh.length) await context.addCookies(fresh);
      for (const c of fresh) agreed.set(key(c), stamp(c));
    } catch {}
  };
  await take();
  const place = await takePlace();
  process.on('exit', () => leave(place));
  const save = async () => {
    const mine = await context.storageState();
    const changed = mine.cookies.filter((c: any) => shared(c) && agreed.get(key(c)) !== stamp(c));
    const changedOrigins = mine.origins.filter((o: any) => sharedOrigin(o) && agreedOrigins.get(o.origin) !== JSON.stringify(o.localStorage));
    // Nothing of its own to hand back: the file stays as it is.
    if (!changed.length && !changedOrigins.length) return;
    let kept = { cookies: [] as any[], origins: [] as any[] };
    try { kept = JSON.parse(fs.readFileSync(STATE, 'utf8')); } catch {}
    const cookies = new Map(kept.cookies.filter(shared).map((c: any) => [key(c), c]));
    for (const c of changed) cookies.set(key(c), c);
    const origins = new Map(kept.origins.filter(sharedOrigin).map((o: any) => [o.origin, o]));
    for (const o of changedOrigins) origins.set(o.origin, o);
    const tmp = `${STATE}.${process.pid}`;
    fs.writeFileSync(tmp, JSON.stringify({ cookies: [...cookies.values()], origins: [...origins.values()] }), { mode: 0o600 });
    fs.renameSync(tmp, STATE);
    for (const c of changed) {
      agreed.set(key(c), stamp(c));
      read.set(key(c), stamp(c));
    }
    for (const o of changedOrigins) agreedOrigins.set(o.origin, JSON.stringify(o.localStorage));
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
