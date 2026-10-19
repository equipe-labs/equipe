// Loads the DTCG token files, applies one platform's overrides, lets Style Dictionary
// resolve every {reference}, and returns a flat, typed list. Every emitter and the
// contrast check read from here, so all four platforms come from the same source.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import StyleDictionary from 'style-dictionary';

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const PLATFORMS = ['web', 'desktop', 'ios', 'android'];
export const MODES = ['light', 'dark'];
const EXT = 'im.equipe';

const readJson = (f) => JSON.parse(fs.readFileSync(path.join(ROOT, 'tokens', f), 'utf8'));

function merge(a, b) {
  for (const [k, v] of Object.entries(b)) {
    if (v && typeof v === 'object' && !Array.isArray(v) && !('$value' in v) && a[k] && typeof a[k] === 'object') merge(a[k], v);
    else a[k] = v;
  }
  return a;
}

// platform: 'web' | 'web-narrow' | 'desktop' | 'ios' | 'android'. 'web-narrow' is web with
// the (<=640px) overrides applied; everything without an override keeps the base value.
function applyPlatform(node, platform) {
  for (const [k, v] of Object.entries(node)) {
    if (k.startsWith('$') || !v || typeof v !== 'object') continue;
    if ('$value' in v) {
      const o = v.$extensions?.[EXT]?.platform?.[platform];
      if (o !== undefined) {
        if (typeof o === 'object' && !Array.isArray(o)) {
          // keep native-only hints (textStyle, slot) out of the SD value
          const { textStyle, slot, ...rest } = o;
          v.$value = rest;
          v.$extensions = { [EXT]: { ...v.$extensions[EXT], native: { textStyle, slot } } };
        } else v.$value = o;
      }
    } else applyPlatform(v, platform);
  }
}

export function loadTree(mode) {
  const tree = {};
  for (const f of ['primitive.json', 'semantic.json', `semantic.color.${mode}.json`, 'component.json']) merge(tree, structuredClone(readJson(f)));
  return tree;
}

const unit = (s) => parseFloat(String(s));
export function parseColor(hex) {
  const h = hex.replace('#', '');
  const n = (i) => parseInt(h.slice(i, i + 2), 16);
  return { r: n(0), g: n(2), b: n(4), a: h.length === 8 ? n(6) / 255 : 1, hex: '#' + h.toUpperCase() };
}
function parseTypo(v, native) {
  const fam = Array.isArray(v.fontFamily) ? v.fontFamily : [v.fontFamily];
  return {
    family: fam, mono: fam.some((f) => /mono|menlo/i.test(f)), size: unit(v.fontSize), line: unit(v.lineHeight),
    weight: Number(v.fontWeight), tracking: unit(v.letterSpacing || 0),
    textStyle: native?.textStyle, slot: native?.slot,
  };
}
function parse(type, v, native) {
  switch (type) {
    case 'color': return parseColor(v);
    case 'dimension': return unit(v);
    case 'duration': return unit(v);
    case 'number': case 'fontWeight': return Number(v);
    case 'cubicBezier': return v;
    case 'typography': return parseTypo(v, native);
    case 'shadow': return v.map((s) => ({ color: parseColor(s.color), x: unit(s.offsetX), y: unit(s.offsetY), blur: unit(s.blur), spread: unit(s.spread) }));
    default: return v;
  }
}

export async function resolve(platform, mode) {
  const tree = loadTree(mode);
  applyPlatform(tree, platform);
  const sd = new StyleDictionary({ tokens: tree, log: { verbosity: 'silent' }, platforms: { x: {} } });
  const dict = await sd.getPlatformTokens('x');
  return dict.allTokens.map((t) => {
    const ov = t.original.$value;
    const refm = typeof ov === 'string' && /^\{([^}]+)\}$/.exec(ov);
    return {
      path: t.path, name: t.path.join('.'), type: t.$type, desc: t.$description || t.description || '',
      value: parse(t.$type, t.$value, t.$extensions?.[EXT]?.native), ref: refm ? refm[1] : null,
    };
  });
}

// Tokens that platform code should see (primitives that only exist to be referenced stay out).
export const skip = (t) => /^(color\.raw|duration|easing|opacity)\b/.test(t.name) || t.name.startsWith('font.weight');

export async function resolveAll() {
  const out = {};
  for (const p of [...PLATFORMS, 'web-narrow']) {
    out[p] = {};
    for (const m of MODES) out[p][m] = await resolve(p, m);
  }
  return out;
}

export const camel = (s) => s.replace(/[-_ ]+(\w)/g, (_, c) => c.toUpperCase());
export const pascal = (s) => { const c = camel(s); return c[0].toUpperCase() + c.slice(1); };
export const snake = (s) => s.replace(/-/g, '_');
// A path segment that starts with a digit needs a letter in front of it in code.
export const seg = (group, k) => (/^\d/.test(k) ? ({ space: 's', border: 'w', z: 'z' }[group] ?? 'n') + k : k);
export const write = (rel, text) => { const f = path.join(ROOT, rel); fs.mkdirSync(path.dirname(f), { recursive: true }); fs.writeFileSync(f, text); };
