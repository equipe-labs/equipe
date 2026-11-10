# Metrics screen mockups (T-311)
`connect` (Connect), `dash` (Dashboard), `state` (empty state·errors), `ph` (phone summary card) × light·dark, 1280 wide. Values come only from `build/css/tokens.css`. The numbers are sample values.
- Every number carries its source service and refresh time (`.src`); change against the previous period is ▲▼ + a sign (colour only supports).
- A rise is `color.ok`, a fall `color.wait` (not the error colour). There are no separate `color.metric.*` tokens yet — to be proposed once approved.
- Visits are counted as 'page loads' (PostHog gives every page load a new ID). GitHub only provides the last 14 days.
- To regenerate: `node gen.mjs`, then `equipe design render <file>.html --widths 1280 --dark`.
