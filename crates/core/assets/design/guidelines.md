# Guidelines

How the system is used — the rules an engineer and the next designer follow.
Every rule names its tokens; nothing here is a value of its own.

## Colour roles
What each role is for (`color.text.primary`, `color.surface.raised`, `color.action.primary`, …),
what it sits on, and its contrast in light and dark (from `equipe design check`).

## Type
The scale (display → caption), which style each kind of text uses, line heights,
numbers (tabular figures in tables and KPIs), Korean line breaking (keep-all).

## Space, radius, elevation
The spacing steps and where each is used; radii by component size; the shadow levels.

## Components
For each key component: anatomy, states (default, hover, focus, pressed, disabled,
loading, error), sizes, and the tokens it uses. A preview of each in `components/`.

## Layout
Widths supported (390 · 768 · 1280+), grid and margins, how navigation changes with width.

## Motion
Durations and easings, and what moves (never content the person is reading).

## Accessibility
AA contrast for text, 3:1 for icons and focus rings, visible focus, hit areas ≥ 44px on touch.
