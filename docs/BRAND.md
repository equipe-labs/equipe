# equipe — visual identity

North star: **a fashion house for software teams.** Editorial, restrained,
futuristic, a little uncanny — closer to a Gentle Monster campaign than a dev-tool
README. One hero object, lots of negative space, monochrome + chrome, film grain.

## Mood

Premium · minimal · cinematic · cold-luxury · sculptural. Never cluttered, never
cute, never rainbow. A stranger should feel "this is a serious, designed thing"
before they read a word.

## Palette

| Token        | Hex / value              | Use                                   |
| ------------ | ------------------------ | ------------------------------------- |
| Void         | `#0A0A0C`                | background                            |
| Bone         | `#EFEDE7`                | wordmark, primary text                |
| Warm grey    | `#CFCCC4`                | secondary text / tagline              |
| Muted        | `#7C7E85`                | tracked-caps kickers, captions        |
| Hairline     | `rgba(233,230,223,.14)`  | frames, rules                         |
| Chrome ramp  | `#F6F7F8 → #D2D6DC → #8A909B → #3B4049 → #0B0D11` | metallic objects |
| Cool rim     | `rgba(180,205,230,.5)`   | the *only* color — a whisper on chrome edges |

Monochrome + chrome is the whole system. If you reach for a second accent color,
stop — use light, grain, or scale instead.

## Type

System grotesque: **Helvetica Neue** (fallback Arial / system-ui). Two moves only.

- **Wordmark** `equipe.` — lowercase, weight 700, very tight tracking (~-6px at
  150px). The `.` is a chrome-grey period, the one brand mark.
- **Editorial caps** — kickers, credits, footers: UPPERCASE, `letter-spacing`
  2.5–3.5px, 13–15px, muted. Set like film/fashion credits.
- Tagline / body: weight 300, generous size, warm grey.

## Motif

A single **sculptural chrome object** (sphere → liquid-metal blob → mask), boldly
cropped off an edge, lit like a studio still. It stands in for "the squad / the
system" without being a literal diagram. Always paired with:

- a **gallery placard** frame — hairline inset border + small corner ticks,
- a top **kicker** row and a bottom **placard** row (MIT · ACP-first … / year),
- **film grain** (SVG `feTurbulence`, ~0.1 opacity, `overlay`) + a soft studio
  light and vignette.

## Do / Don't

- **Do:** negative space, one hero object, tracked caps, grain, hard crops, silence.
- **Don't:** multiple accent colors, decorative gradients, emoji, boxy flowchart
  diagrams, drop-shadow soup, stock-y icons.

## Assets

- `demo/social-card.html` → render with headless Chrome to `demo/social-card.png`
  (2400×1350, 16:9) for X / GitHub social preview:
  ```bash
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
    --headless=new --disable-gpu --hide-scrollbars \
    --force-device-scale-factor=2 --window-size=1200,675 --virtual-time-budget=1500 \
    --screenshot="$PWD/demo/social-card.png" "file://$PWD/demo/social-card.html"
  ```
- `demo/avatar.html` → `demo/render-avatar.sh` renders `demo/avatar.png` (1024×1024),
  the org avatar: the lowercase **e** and its period as the chrome object — the
  wordmark reduced to what still reads at 20px. `render-avatar.sh object` renders
  the sculpture alone, for large placements only.
- Terminal demo gifs (`demo/*.tape`) inherit the same palette (Dracula-ish dark);
  keep them dark, quiet, and text-forward.
