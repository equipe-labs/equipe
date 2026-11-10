// npm run check:copies -- [<repo>=<path> …] — every copy listed in copies.json is byte-for-byte its source.
// A repository not given is looked for next to this one (../<repo>); one not checked out fails the check.
import { readFileSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const design = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const { copies = [] } = JSON.parse(readFileSync(join(design, 'copies.json'), 'utf8'));
const given = Object.fromEntries(process.argv.slice(2).map((a) => a.split('=')).filter((p) => p.length === 2));
const where = (repo) => resolve(given[repo] ?? join(design, '..', '..', repo));

let bad = 0;
for (const { from, repo, to } of copies) {
  const source = join(design, from);
  const copy = join(where(repo), to);
  const state = !existsSync(source) ? `source missing: ${from}`
    : !existsSync(where(repo)) ? `${repo} not checked out at ${where(repo)} (give ${repo}=<path>)`
    : !existsSync(copy) ? 'copy missing'
    : readFileSync(source).equals(readFileSync(copy)) ? null : 'differs from its source';
  console.log(`${state ? '✗' : '✓'} ${from} → ${repo}:${to}${state ? ` — ${state}` : ''}`);
  if (state) bad++;
}
if (!copies.length) console.log('no copies listed in copies.json');
if (bad) {
  console.error(`${bad} cop${bad === 1 ? 'y' : 'ies'} out of step: run \`npm run build\` here and copy the generated file again — never edit a copy by hand.`);
  process.exit(1);
}
