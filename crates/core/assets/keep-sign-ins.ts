// equipe: a member's browser hands what it signs in to back to the squad, and
// takes one of the squad's browser places while it is open.
// Playwright MCP runs this on every page (--init-page); STATE, SLOTS and MAX are
// filled in by equipe. Cookies and local storage are merged into the shared
// sign-ins, newest wins, a second after each page loads and when the browser
// closes. At most MAX browsers run at once: a browser opening while all places
// are taken waits for one (a few minutes at most, then opens anyway).
import * as fs from 'node:fs';

const STATE: string = __STATE__;
const SLOTS: string = __SLOTS__;
const MAX: number = __MAX__;

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
  // Start from the squad's sign-ins — the person's, and what teammates signed in to.
  try {
    const kept = JSON.parse(fs.readFileSync(STATE, 'utf8'));
    const now = Date.now() / 1000;
    await context.addCookies((kept.cookies || []).filter((c: any) => c.expires === -1 || c.expires > now));
  } catch {}
  const place = await takePlace();
  process.on('exit', () => leave(place));
  const save = async () => {
    const mine = await context.storageState();
    let kept = { cookies: [] as any[], origins: [] as any[] };
    try { kept = JSON.parse(fs.readFileSync(STATE, 'utf8')); } catch {}
    const key = (c: any) => `${c.name}\u0000${c.domain}\u0000${c.path}`;
    const cookies = new Map(kept.cookies.map((c: any) => [key(c), c]));
    for (const c of mine.cookies) cookies.set(key(c), c);
    const origins = new Map(kept.origins.map((o: any) => [o.origin, o]));
    for (const o of mine.origins) origins.set(o.origin, o);
    const tmp = `${STATE}.${process.pid}`;
    fs.writeFileSync(tmp, JSON.stringify({ cookies: [...cookies.values()], origins: [...origins.values()] }), { mode: 0o600 });
    fs.renameSync(tmp, STATE);
  };
  let timer: any;
  const soon = () => {
    clearTimeout(timer);
    timer = setTimeout(() => save().catch(() => {}), 1000);
  };
  context.on('page', (p: any) => p.on('load', soon));
  page.on('load', soon);
  context.on('close', () => {
    save().catch(() => {}).finally(() => leave(place));
  });
};
