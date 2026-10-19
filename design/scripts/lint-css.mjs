// npm run lint:css — reference/components.css may only use tokens: no hex colour, no px/ms literal except 0.
import fs from 'node:fs'; import path from 'node:path'; import { ROOT } from './lib.mjs';
const src = fs.readFileSync(path.join(ROOT, 'reference/components.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
const bad = [...src.matchAll(/(#[0-9a-fA-F]{3,8}\b|\b[1-9]\d*(\.\d+)?(px|ms|rem)\b|rgba?\()/g)].map((m) => m[0]);
if (bad.length) { console.error('raw values in components.css:', [...new Set(bad)].join(' ')); process.exit(1); }
console.log('components.css: tokens only');
