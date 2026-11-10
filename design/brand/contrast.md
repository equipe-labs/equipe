# Contrast — every pair of the three palettes

Computed with `src/palettes.py` (the WCAG 2.x relative luminance formula, sRGB). Thresholds: text 4.5:1, graphics (focus ring·brand dot) 3:1. These tables can be made again with `python3 src/palettes.py`/`gen_docs.py`.

## A. Signal blue — warm neutrals + one blue dot

| Mode | Foreground | Background | Contrast | Needed | Result | Use |
|---|---|---|---|---|---|---|
| light | `text` #15161A | `bg` #F6F5F1 | 16.57:1 | 4.5:1 | ✅ | body text |
| light | `text` #15161A | `surface` #FFFFFF | 18.08:1 | 4.5:1 | ✅ | text on a card |
| light | `text` #15161A | `sunken` #EAE7E0 | 14.64:1 | 4.5:1 | ✅ | text on code·pressed surface |
| light | `muted` #4F5159 | `bg` #F6F5F1 | 7.25:1 | 4.5:1 | ✅ | secondary text |
| light | `muted` #4F5159 | `surface` #FFFFFF | 7.91:1 | 4.5:1 | ✅ | secondary text on a card |
| light | `muted` #4F5159 | `sunken` #EAE7E0 | 6.41:1 | 4.5:1 | ✅ | secondary text on a pressed surface |
| light | `accent` #1D4ED8 | `bg` #F6F5F1 | 6.14:1 | 4.5:1 | ✅ | links·text buttons |
| light | `accent` #1D4ED8 | `surface` #FFFFFF | 6.70:1 | 4.5:1 | ✅ | links on a card |
| light | `onAccent` #FFFFFF | `accent` #1D4ED8 | 6.70:1 | 4.5:1 | ✅ | filled button text |
| light | `focus` #1D4ED8 | `bg` #F6F5F1 | 6.14:1 | 3.0:1 | ✅ | focus ring (graphic) |
| light | `dot` #3F6FF5 | `bg` #F6F5F1 | 4.01:1 | 3.0:1 | ✅ | brand dot on the background (graphic) |
| light | `ok` #1F6B33 | `bg` #F6F5F1 | 6.00:1 | 4.5:1 | ✅ | done·success text |
| light | `work` #825600 | `bg` #F6F5F1 | 5.87:1 | 4.5:1 | ✅ | in-progress text |
| light | `wait` #A04610 | `bg` #F6F5F1 | 5.70:1 | 4.5:1 | ✅ | your turn·waiting-for-permission text |
| light | `err` #AD2E27 | `bg` #F6F5F1 | 6.02:1 | 4.5:1 | ✅ | error text |
| light | `ok` #1F6B33 | `surface` #FFFFFF | 6.54:1 | 4.5:1 | ✅ | success on a card |
| light | `err` #AD2E27 | `surface` #FFFFFF | 6.57:1 | 4.5:1 | ✅ | error on a card |
| light | `wait` #A04610 | `surface` #FFFFFF | 6.22:1 | 4.5:1 | ✅ | waiting on a card |
| light | `work` #825600 | `surface` #FFFFFF | 6.40:1 | 4.5:1 | ✅ | in progress on a card |
| light | `dot` #3F6FF5 | `icon tile void` #0A0A0C | 4.52:1 | 3.0:1 | ✅ | the app icon's dot (graphic) |
| dark | `text` #EFEDE7 | `bg` #0A0A0C | 16.90:1 | 4.5:1 | ✅ | body text |
| dark | `text` #EFEDE7 | `surface` #16171A | 15.31:1 | 4.5:1 | ✅ | text on a card |
| dark | `text` #EFEDE7 | `sunken` #111114 | 16.10:1 | 4.5:1 | ✅ | text on code·pressed surface |
| dark | `muted` #A3A5AB | `bg` #0A0A0C | 8.03:1 | 4.5:1 | ✅ | secondary text |
| dark | `muted` #A3A5AB | `surface` #16171A | 7.28:1 | 4.5:1 | ✅ | secondary text on a card |
| dark | `muted` #A3A5AB | `sunken` #111114 | 7.65:1 | 4.5:1 | ✅ | secondary text on a pressed surface |
| dark | `accent` #8FB0FF | `bg` #0A0A0C | 9.25:1 | 4.5:1 | ✅ | links·text buttons |
| dark | `accent` #8FB0FF | `surface` #16171A | 8.38:1 | 4.5:1 | ✅ | links on a card |
| dark | `onAccent` #0B1226 | `accent` #8FB0FF | 8.70:1 | 4.5:1 | ✅ | filled button text |
| dark | `focus` #8FB0FF | `bg` #0A0A0C | 9.25:1 | 3.0:1 | ✅ | focus ring (graphic) |
| dark | `dot` #3F6FF5 | `bg` #0A0A0C | 4.52:1 | 3.0:1 | ✅ | brand dot on the background (graphic) |
| dark | `ok` #7FCB8E | `bg` #0A0A0C | 10.21:1 | 4.5:1 | ✅ | done·success text |
| dark | `work` #E2B865 | `bg` #0A0A0C | 10.62:1 | 4.5:1 | ✅ | in-progress text |
| dark | `wait` #EC9A62 | `bg` #0A0A0C | 8.82:1 | 4.5:1 | ✅ | your turn·waiting-for-permission text |
| dark | `err` #EE7B74 | `bg` #0A0A0C | 7.27:1 | 4.5:1 | ✅ | error text |
| dark | `ok` #7FCB8E | `surface` #16171A | 9.25:1 | 4.5:1 | ✅ | success on a card |
| dark | `err` #EE7B74 | `surface` #16171A | 6.59:1 | 4.5:1 | ✅ | error on a card |
| dark | `wait` #EC9A62 | `surface` #16171A | 7.99:1 | 4.5:1 | ✅ | waiting on a card |
| dark | `work` #E2B865 | `surface` #16171A | 9.63:1 | 4.5:1 | ✅ | in progress on a card |
| dark | `dot` #3F6FF5 | `icon tile void` #0A0A0C | 4.52:1 | 3.0:1 | ✅ | the app icon's dot (graphic) |

Pairs below threshold: none

## B. Steel chrome — chrome grey + a cold steel blue

| Mode | Foreground | Background | Contrast | Needed | Result | Use |
|---|---|---|---|---|---|---|
| light | `text` #15161A | `bg` #F6F5F1 | 16.57:1 | 4.5:1 | ✅ | body text |
| light | `text` #15161A | `surface` #FFFFFF | 18.08:1 | 4.5:1 | ✅ | text on a card |
| light | `text` #15161A | `sunken` #ECEBE6 | 15.15:1 | 4.5:1 | ✅ | text on code·pressed surface |
| light | `muted` #5E6068 | `bg` #F6F5F1 | 5.75:1 | 4.5:1 | ✅ | secondary text |
| light | `muted` #5E6068 | `surface` #FFFFFF | 6.27:1 | 4.5:1 | ✅ | secondary text on a card |
| light | `muted` #5E6068 | `sunken` #ECEBE6 | 5.25:1 | 4.5:1 | ✅ | secondary text on a pressed surface |
| light | `accent` #2E5C8A | `bg` #F6F5F1 | 6.39:1 | 4.5:1 | ✅ | links·text buttons |
| light | `accent` #2E5C8A | `surface` #FFFFFF | 6.97:1 | 4.5:1 | ✅ | links on a card |
| light | `onAccent` #F6F5F1 | `accent` #2E5C8A | 6.39:1 | 4.5:1 | ✅ | filled button text |
| light | `focus` #2E5C8A | `bg` #F6F5F1 | 6.39:1 | 3.0:1 | ✅ | focus ring (graphic) |
| light | `dot` #7F8591 | `bg` #F6F5F1 | 3.40:1 | 3.0:1 | ✅ | brand dot on the background (graphic) |
| light | `ok` #1F6B33 | `bg` #F6F5F1 | 6.00:1 | 4.5:1 | ✅ | done·success text |
| light | `work` #825600 | `bg` #F6F5F1 | 5.87:1 | 4.5:1 | ✅ | in-progress text |
| light | `wait` #A04610 | `bg` #F6F5F1 | 5.70:1 | 4.5:1 | ✅ | your turn·waiting-for-permission text |
| light | `err` #AD2E27 | `bg` #F6F5F1 | 6.02:1 | 4.5:1 | ✅ | error text |
| light | `ok` #1F6B33 | `surface` #FFFFFF | 6.54:1 | 4.5:1 | ✅ | success on a card |
| light | `err` #AD2E27 | `surface` #FFFFFF | 6.57:1 | 4.5:1 | ✅ | error on a card |
| light | `wait` #A04610 | `surface` #FFFFFF | 6.22:1 | 4.5:1 | ✅ | waiting on a card |
| light | `work` #825600 | `surface` #FFFFFF | 6.40:1 | 4.5:1 | ✅ | in progress on a card |
| light | `dot` #7F8591 | `icon tile void` #0A0A0C | 5.34:1 | 3.0:1 | ✅ | the app icon's dot (graphic) |
| dark | `text` #EFEDE7 | `bg` #0A0A0C | 16.90:1 | 4.5:1 | ✅ | body text |
| dark | `text` #EFEDE7 | `surface` #16171A | 15.31:1 | 4.5:1 | ✅ | text on a card |
| dark | `text` #EFEDE7 | `sunken` #111114 | 16.10:1 | 4.5:1 | ✅ | text on code·pressed surface |
| dark | `muted` #8A8C93 | `bg` #0A0A0C | 5.89:1 | 4.5:1 | ✅ | secondary text |
| dark | `muted` #8A8C93 | `surface` #16171A | 5.34:1 | 4.5:1 | ✅ | secondary text on a card |
| dark | `muted` #8A8C93 | `sunken` #111114 | 5.61:1 | 4.5:1 | ✅ | secondary text on a pressed surface |
| dark | `accent` #B4CDE6 | `bg` #0A0A0C | 12.07:1 | 4.5:1 | ✅ | links·text buttons |
| dark | `accent` #B4CDE6 | `surface` #16171A | 10.94:1 | 4.5:1 | ✅ | links on a card |
| dark | `onAccent` #0A0A0C | `accent` #B4CDE6 | 12.07:1 | 4.5:1 | ✅ | filled button text |
| dark | `focus` #B4CDE6 | `bg` #0A0A0C | 12.07:1 | 3.0:1 | ✅ | focus ring (graphic) |
| dark | `dot` #8A909B | `bg` #0A0A0C | 6.16:1 | 3.0:1 | ✅ | brand dot on the background (graphic) |
| dark | `ok` #7FCB8E | `bg` #0A0A0C | 10.21:1 | 4.5:1 | ✅ | done·success text |
| dark | `work` #E2B865 | `bg` #0A0A0C | 10.62:1 | 4.5:1 | ✅ | in-progress text |
| dark | `wait` #EC9A62 | `bg` #0A0A0C | 8.82:1 | 4.5:1 | ✅ | your turn·waiting-for-permission text |
| dark | `err` #EE7B74 | `bg` #0A0A0C | 7.27:1 | 4.5:1 | ✅ | error text |
| dark | `ok` #7FCB8E | `surface` #16171A | 9.25:1 | 4.5:1 | ✅ | success on a card |
| dark | `err` #EE7B74 | `surface` #16171A | 6.59:1 | 4.5:1 | ✅ | error on a card |
| dark | `wait` #EC9A62 | `surface` #16171A | 7.99:1 | 4.5:1 | ✅ | waiting on a card |
| dark | `work` #E2B865 | `surface` #16171A | 9.63:1 | 4.5:1 | ✅ | in progress on a card |
| dark | `dot` #8A909B | `icon tile void` #0A0A0C | 6.16:1 | 3.0:1 | ✅ | the app icon's dot (graphic) |

Pairs below threshold: none

## C. Mono ink — no colour; the ink is the emphasis

| Mode | Foreground | Background | Contrast | Needed | Result | Use |
|---|---|---|---|---|---|---|
| light | `text` #0A0A0C | `bg` #F6F5F1 | 18.13:1 | 4.5:1 | ✅ | body text |
| light | `text` #0A0A0C | `surface` #FFFFFF | 19.78:1 | 4.5:1 | ✅ | text on a card |
| light | `text` #0A0A0C | `sunken` #EFEDE7 | 16.90:1 | 4.5:1 | ✅ | text on code·pressed surface |
| light | `muted` #5B5E66 | `bg` #F6F5F1 | 5.95:1 | 4.5:1 | ✅ | secondary text |
| light | `muted` #5B5E66 | `surface` #FFFFFF | 6.49:1 | 4.5:1 | ✅ | secondary text on a card |
| light | `muted` #5B5E66 | `sunken` #EFEDE7 | 5.54:1 | 4.5:1 | ✅ | secondary text on a pressed surface |
| light | `accent` #0A0A0C | `bg` #F6F5F1 | 18.13:1 | 4.5:1 | ✅ | links·text buttons |
| light | `accent` #0A0A0C | `surface` #FFFFFF | 19.78:1 | 4.5:1 | ✅ | links on a card |
| light | `onAccent` #F6F5F1 | `accent` #0A0A0C | 18.13:1 | 4.5:1 | ✅ | filled button text |
| light | `focus` #2F5E8C | `bg` #F6F5F1 | 6.21:1 | 3.0:1 | ✅ | focus ring (graphic) |
| light | `dot` #5B5E66 | `bg` #F6F5F1 | 5.95:1 | 3.0:1 | ✅ | brand dot on the background (graphic) |
| light | `ok` #1F6B33 | `bg` #F6F5F1 | 6.00:1 | 4.5:1 | ✅ | done·success text |
| light | `work` #825600 | `bg` #F6F5F1 | 5.87:1 | 4.5:1 | ✅ | in-progress text |
| light | `wait` #A04610 | `bg` #F6F5F1 | 5.70:1 | 4.5:1 | ✅ | your turn·waiting-for-permission text |
| light | `err` #A32A24 | `bg` #F6F5F1 | 6.61:1 | 4.5:1 | ✅ | error text |
| light | `ok` #1F6B33 | `surface` #FFFFFF | 6.54:1 | 4.5:1 | ✅ | success on a card |
| light | `err` #A32A24 | `surface` #FFFFFF | 7.21:1 | 4.5:1 | ✅ | error on a card |
| light | `wait` #A04610 | `surface` #FFFFFF | 6.22:1 | 4.5:1 | ✅ | waiting on a card |
| light | `work` #825600 | `surface` #FFFFFF | 6.40:1 | 4.5:1 | ✅ | in progress on a card |
| light | `dot` #5B5E66 | `icon tile void` #0A0A0C | 3.05:1 | 3.0:1 | ✅ | the app icon's dot (graphic) |
| dark | `text` #EFEDE7 | `bg` #0A0A0C | 16.90:1 | 4.5:1 | ✅ | body text |
| dark | `text` #EFEDE7 | `surface` #16171A | 15.31:1 | 4.5:1 | ✅ | text on a card |
| dark | `text` #EFEDE7 | `sunken` #111114 | 16.10:1 | 4.5:1 | ✅ | text on code·pressed surface |
| dark | `muted` #A3A5AB | `bg` #0A0A0C | 8.03:1 | 4.5:1 | ✅ | secondary text |
| dark | `muted` #A3A5AB | `surface` #16171A | 7.28:1 | 4.5:1 | ✅ | secondary text on a card |
| dark | `muted` #A3A5AB | `sunken` #111114 | 7.65:1 | 4.5:1 | ✅ | secondary text on a pressed surface |
| dark | `accent` #EFEDE7 | `bg` #0A0A0C | 16.90:1 | 4.5:1 | ✅ | links·text buttons |
| dark | `accent` #EFEDE7 | `surface` #16171A | 15.31:1 | 4.5:1 | ✅ | links on a card |
| dark | `onAccent` #0A0A0C | `accent` #EFEDE7 | 16.90:1 | 4.5:1 | ✅ | filled button text |
| dark | `focus` #B4CDE6 | `bg` #0A0A0C | 12.07:1 | 3.0:1 | ✅ | focus ring (graphic) |
| dark | `dot` #7C7E85 | `bg` #0A0A0C | 4.88:1 | 3.0:1 | ✅ | brand dot on the background (graphic) |
| dark | `ok` #7FCB8E | `bg` #0A0A0C | 10.21:1 | 4.5:1 | ✅ | done·success text |
| dark | `work` #E2B865 | `bg` #0A0A0C | 10.62:1 | 4.5:1 | ✅ | in-progress text |
| dark | `wait` #EC9A62 | `bg` #0A0A0C | 8.82:1 | 4.5:1 | ✅ | your turn·waiting-for-permission text |
| dark | `err` #E8837D | `bg` #0A0A0C | 7.51:1 | 4.5:1 | ✅ | error text |
| dark | `ok` #7FCB8E | `surface` #16171A | 9.25:1 | 4.5:1 | ✅ | success on a card |
| dark | `err` #E8837D | `surface` #16171A | 6.81:1 | 4.5:1 | ✅ | error on a card |
| dark | `wait` #EC9A62 | `surface` #16171A | 7.99:1 | 4.5:1 | ✅ | waiting on a card |
| dark | `work` #E2B865 | `surface` #16171A | 9.63:1 | 4.5:1 | ✅ | in progress on a card |
| dark | `dot` #7C7E85 | `icon tile void` #0A0A0C | 4.88:1 | 3.0:1 | ✅ | the app icon's dot (graphic) |

Pairs below threshold: none

