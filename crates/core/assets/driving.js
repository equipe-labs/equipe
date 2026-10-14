// What a member's browser on the person's screen shows of who drives it, as
// Claude in Chrome does: a glow along its edge and a label at the foot, in the
// member's own colour, and an outline on what it clicks or types into. A member
// doesn't steer a pointer — it picks an element and Playwright acts on it — so
// what it acts on is what's shown. The glow breathes on its own layer, by
// opacity alone, which the compositor does without redrawing the page. Drawn in
// a closed shadow root that no click, page style or accessibility snapshot
// reaches; built without HTML strings or inline <style>, which strict pages
// (Trusted Types, CSP) refuse. Plain JavaScript: it runs in pages, and equipe
// writes it into both its browser scripts.
function driving({ member, color }) {
  if (window.top !== window || window.__equipeDriving) return;
  window.__equipeDriving = true;
  const mount = () => {
    if (!document.documentElement) return;
    const host = document.createElement('equipe-driving');
    host.setAttribute('aria-hidden', 'true');
    host.style.cssText = 'all: initial; position: fixed; inset: 0; pointer-events: none; z-index: 2147483647;';
    const root = host.attachShadow({ mode: 'closed' });
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(`
      * { box-sizing: border-box; pointer-events: none; }
      :host { --fade: rgba(${color}, .34) 0, rgba(${color}, .16) 16px, rgba(${color}, .06) 40px, rgba(${color}, 0) 72px; }
      .glow { position: fixed; inset: 0; will-change: opacity; animation: breathe 3.2s ease-in-out infinite; box-shadow: inset 0 0 2px 1px rgba(${color}, .7), inset 0 0 10px 2px rgba(${color}, .32);
        background: linear-gradient(to bottom, var(--fade)) top / 100% 72px no-repeat, linear-gradient(to top, var(--fade)) bottom / 100% 72px no-repeat,
          linear-gradient(to right, var(--fade)) left / 72px 100% no-repeat, linear-gradient(to left, var(--fade)) right / 72px 100% no-repeat; }
      .label { position: fixed; bottom: 14px; left: 50%; transform: translateX(-50%); display: flex; align-items: center; gap: 7px; padding: 5px 12px 5px 10px; border-radius: 999px; background: rgba(24, 24, 27, .86); color: #fafafa; font: 500 12px/1.3 -apple-system, BlinkMacSystemFont, system-ui, sans-serif; letter-spacing: .01em; box-shadow: 0 0 0 1px rgba(${color}, .7), 0 2px 12px rgba(0, 0, 0, .25); white-space: nowrap; }
      .dot { width: 8px; height: 8px; border-radius: 50%; background: rgb(${color}); box-shadow: 0 0 8px rgb(${color}); animation: breathe 1.6s ease-in-out infinite; }
      .target { position: fixed; border-radius: 6px; border: 2px solid rgb(${color}); background: rgba(${color}, .12); box-shadow: 0 0 0 4px rgba(${color}, .25); opacity: 0; transition: opacity .4s ease; }
      .target.on { opacity: 1; transition: opacity .08s ease; }
      @keyframes breathe { 0%, 100% { opacity: .6; } 50% { opacity: 1; } }
      @media (prefers-reduced-motion: reduce) { .glow, .dot { animation: none; } }
    `);
    root.adoptedStyleSheets = [sheet];
    const el = (cls) => {
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
    let fade;
    const outline = (node) => {
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
