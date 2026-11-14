---
name: ui-data-display
description: Use for tables, KPI tiles, charts, filter bars and dashboards.
---

# Data display for dense tools

Data screens are judged by how fast someone who knows the domain can read them and act. Every pixel that isn't data
or a control for it has to earn its place.

## Choose the form by the question

- Compare many items across several attributes → a **table**.
- Scan items with one or two attributes, often on a phone → a **list**.
- Browse items that are mostly visual (designs, products, people) → a **grid of cards**.
- Watch a handful of numbers that matter now → **KPI tiles**.
- See how something changes or compares → a **chart**, picked by the question below.

## Tables

- **Rows**: 32–40px in dense tools, 48px when rows hold two lines or touch targets; the header row matches the body.
- **Alignment**: text left; numbers right, in tabular figures, with the same decimals down a column; the unit or
  currency in the header ("Amount (EUR)") rather than in every cell; dates in one format. Never centre a column of
  text or numbers.
- **Order**: the identifying column first (and sticky when the table scrolls sideways), then by importance; the
  header sticks on vertical scroll.
- **Separators**: row lines or zebra stripes — one, not both; vertical lines only for very wide numeric tables.
- **Empty cells** show one consistent mark (an em dash, `—`), never a blank that reads as "still loading".
- **Long text** truncates with an ellipsis and the full value on hover and in the detail view; names and IDs never
  truncate to the point of being ambiguous (keep the distinguishing end of an ID).
- **Row actions**: the main one on click of the row (opens the detail), others in a trailing menu that is always
  reachable by keyboard; hover-revealed buttons are a shortcut, not the only way.
- **Selection**: a checkbox column; selecting shows a bulk-action bar with the count ("3 selected") and the actions.
- **Sorting**: clickable headers with a direction indicator; the current sort visible.
- **Many rows**: pagination with the total ("1–50 of 1,284") or a virtualised list; infinite scroll only for feeds.
- **Recent dates** may read relative ("2 h ago") with the absolute time on hover; anything older than a week, absolute.
- **Narrow widths**: the table scrolls sideways inside its own container with the first column fixed, or turns into
  a list that keeps the two or three columns that matter.
- Don't: a card around every row; colour on every status cell; actions only visible on hover; numbers left-aligned.

## KPI tiles

- **Anatomy**: a label that names the metric and its period ("Net revenue · Oct"), the value large in tabular
  figures, the comparison (change against the previous period or a target, with sign and an arrow), and optionally a
  sparkline or a target bar. A number with nothing to compare it to is a fact, not a KPI.
- **Colour the change by meaning**, not direction: costs going up is bad. Pair the colour with an arrow and a sign.
- **Format**: abbreviate large numbers consistently (`1.2M`, `84.3k`) with the exact value on hover; one precision per
  row of tiles; the currency symbol placed as the locale places it.
- **A row of three to five**, equal width and height, aligned to the grid; four to six tiles at most above the fold.
- **Light framing**: a subtle surface or a hairline, or none — not heavy cards with shadows; no icon per tile unless it
  helps tell tiles apart at a glance.
- **States**: a skeleton while loading, "—" with a reason when there is no data, an inline error with retry.
- Each tile links to the view that explains it.

## Charts

| Question | Chart |
|---|---|
| How does it change over time? | Line (area only for one series or a stacked total) |
| How do categories compare? | Bar — horizontal when labels are long |
| What is the share of a whole? | Stacked bar; a pie or donut only for 2–4 slices |
| How is it distributed? | Histogram, box plot |
| Do two things relate? | Scatter |
| Many small series? | Small multiples with a shared scale |

- **Title states the metric and period**, or the takeaway ("Spend is 12% over budget since August").
- **Bars start at zero**; line charts may zoom, with the axis labelled clearly.
- **Label directly** at the line's end when there are five series or fewer; past five to seven, highlight the one that
  matters and grey the rest.
- **Quiet scaffolding**: a few light gridlines, formatted ticks (`10k`, not `10000`), no rotated labels (use horizontal
  bars), no 3D, no second y-axis unless the person asked for it.
- **Colour-blind-safe palette**, and never colour alone — labels, markers or patterns as well.
- **Tooltip** with the exact value, unit and date; a crosshair on time series.
- **One time range** across the charts of a page, set by one control.
- **States**: loading (the frame with a skeleton), no data in the range (say so, suggest another range), error.
- Narrow widths: fewer ticks, a legend below, the chart full width.

## Filter bars

- **Above what they filter**, left-aligned, in one row: the three to five filters most people use, the rest under
  "More filters", and search when the data has names.
- **Applied filters show** as removable chips with "Clear all", and the result count updates ("128 results").
- **Date ranges** offer presets (Today, Last 7 days, Last 30 days, This quarter, Custom) and show the range chosen.
- **Apply instantly** when results come back fast; an explicit "Apply" only when a query is slow or costly; search
  debounced about 200–300ms.
- **The URL holds the filters**; frequent users can save a view.
- Filtering never throws away the person's place without reason; an empty result is the "no results" state.

## Dashboards

- One dashboard answers one family of questions; its title says which.
- Top: the KPI row. Middle: trends. Bottom: the detail table. One shared time range and an "Updated 2 min ago" stamp.
- Everything on one grid; widgets align in height per row; six to nine widgets above the fold at most.
- Don't: a wall of equal cards, decorative charts no one acts on, or a chart and a table saying the same thing side
  by side.

## Review checklist

- [ ] The form fits the question (table, list, cards, tiles, chart).
- [ ] Numbers right-aligned, tabular, one precision per column; units in headers; text left; nothing centred.
- [ ] Sticky header (and first column when scrolling sideways); empty cells marked; long text truncates with the full
      value reachable.
- [ ] Row actions reachable by keyboard; bulk actions show the count.
- [ ] Every KPI has a label with its period, a comparison and a meaning-based colour with an arrow.
- [ ] Each chart's type matches its question; bars from zero; direct labels or a highlighted series; no 3D or rotated
      labels; colour never alone.
- [ ] Filters above the content, applied ones as chips with Clear all, result count shown, state in the URL.
- [ ] Loading, no data and error states for tables, tiles and charts.

Sources: IBM Carbon (data table sizes and patterns); Shopify Polaris (index tables, filters); GitHub Primer; Nielsen
Norman Group on data tables and dashboards; Vercel's Web Interface Guidelines (MIT) — in our own words.
