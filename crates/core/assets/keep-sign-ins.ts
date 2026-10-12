// equipe: a member's browser hands what it signs in to back to the squad.
// Playwright MCP runs this on every page (--init-page); STATE is filled in by
// equipe. Cookies and local storage are merged into the shared sign-ins, newest
// wins, a second after each page loads and when the browser closes.
import * as fs from 'node:fs';

const STATE: string = __STATE__;

export default async ({ page }: { page: any }) => {
  const context = page.context();
  if (context.__equipe) return;
  context.__equipe = true;
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
  context.on('close', () => save().catch(() => {}));
};
