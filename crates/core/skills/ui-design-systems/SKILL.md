---
name: ui-design-systems
description: Use when you design or review a component or pattern — the exact public design-system pages to read for it, by product kind.
---

# Read the real docs for the component in front of you

The craft skills give our rules and numbers. The best public design systems add something they can't: years of
decisions on one component at a time — its anatomy, states, edge cases and the reasons. Before you design or review a
component, open the pages for it, compare them, and design from what they agree on. This skill is the index of those
pages; it does not repeat our rules (`ui-forms`, `ui-data-display`, `ui-layout-and-states`, `ui-mobile`) or the
research steps (`design-references`).

The index is `references/components.md` next to this file: one section per component or pattern, each line a system,
the exact pages, and what it is especially good for. It covers buttons and links, text inputs, selects and comboboxes,
checkbox, radio and switch, date pickers, form layout and validation, file upload, tabs, segmented controls,
navigation (sidebar, top bar, breadcrumbs), pagination, stepper and wizard, settings pages, menus, command palette,
dialogs, drawers and sheets, popovers and tooltips, toasts, notifications and banners, empty states, loading, skeletons
and progress, badges, tags and status, avatars, data tables (sorting, filtering, density, row and bulk actions),
filter bars, chips and search, KPI and stat tiles, charts, cards, lists, accordions, mobile tab and navigation bars,
pull-to-refresh and lists on mobile, and onboarding.

If you build the screen rather than design it, the spec names the kind and holds the designer's notes: read the pages
for each component you build, check your states, sizes and keyboard behaviour against what they agree on, and take any
gap to the designer instead of deciding it alone.

## Which systems fit which kind

Name the kind first (`ui-product-kinds`); read only the systems that fit it.

| System | Landing | Web product | Mobile app | Strongest for |
|---|---|---|---|---|
| IBM Carbon | – | yes | – | dense data tables, filtering, forms, dashboards and chart guidance, notifications |
| GitHub Primer | – | yes (developer tools) | – | lists and tables, navigation, loading, empty states, saving and forms patterns |
| Shopify Polaris | – | yes (admin, merchant tools) | – | page templates (index, settings, details), resource lists, bulk actions, setup guides |
| Vercel Geist | parts (primitives) | yes (consoles) | – | compact dashboard components, destructive confirmations, command menu |
| Atlassian Design System | – | yes (collaboration tools) | – | navigation shells, dynamic tables, message types, onboarding |
| GOV.UK Design System | forms and content only | yes (forms, public services) | – | questions, validation and error wording, check-answers, plain content |
| Material 3 | – | yes (Android-flavoured or cross-platform) | yes (Android) | navigation by window size, sheets, chips, snackbars, lists |
| Apple HIG | – | native desktop only | yes (iOS) | tab bars, sheets, alerts, onboarding, settings, lists, charts |
| Radix primitives, shadcn/ui | yes (build) | yes (build) | – | how to build it accessibly in React: focus, keyboard, composition |

- **Landing**: none of these systems is built for marketing pages. Use them for the controls a landing page contains
  (buttons, links, forms, accordions, navigation, empty and error pages); for layout and story, use the sources of
  `design-references` that fit landing.
- **Web product**: Carbon, Primer, Polaris, Geist and Atlassian are the main references; GOV.UK for any form-heavy flow;
  Radix and shadcn when the stack is React.
- **Mobile app**: HIG for iOS and Material 3 (with Android's developer docs) for Android first; a web system's page is a
  reference for the reasoning (states, copy, data), never for the layout or controls.
- A web product at narrow widths keeps its web references and adds `ui-mobile`'s touch rules.

## The steps

1. **List the components** the screen uses, named as `design-references` names patterns (`data-table`, `filter-bar`).
2. **Open two or three pages for each** from `references/components.md`, chosen for the kind: for a web product
   usually one dense system (Carbon, Primer, Atlassian), one product-flavoured one (Polaris, Geist) and, for forms, GOV.UK;
   for a mobile app the platform's page and one more. Read the usage or guidelines tab, then states, then accessibility;
   the code tab last, if at all. Prefer the page for the pattern (`core/patterns/filtering`, `ui-patterns/forms`) over the
   page for a single control when both exist.
3. **Write what you learned** in the pattern's `design/references/<pattern>/notes.md`, in the entry format of
   `design-references`, one entry per page, with the URL as the source and the date read. End with two short lists:
   *Agree* (what every page does: anatomy, states, placement, labels, sizes, keyboard behaviour) and *Differ* (where they
   split and why — density, platform, product type).
4. **Decide**: follow the agreement. Where the pages differ, choose by the product kind, its density and its users, and
   say which and why in the notes. The product's own accepted system (`design/`, a locked brand, a registry) wins over
   all of it: use the docs to fill what it leaves open and to judge it, never to override it. A gap in the accepted system
   is a request to the system, not a one-off.
5. **Carry it into the spec and the review**: name the states, sizes and keyboard behaviour the pages agreed on in the
   hand-off spec; when you review a component (`ui-review`, design QA), put it beside the same pages and name each
   difference as a finding against the agreement.

## Reading the pages

- Pages open in a browser or by a plain fetch. Material 3 (`m3.material.io`) and Atlassian (`atlassian.design`) render in
  the browser, so a plain fetch returns an empty shell: read them in your own browser (or `equipe design refs shoot <url>`).
  Apple's pages render in the browser too, but their text is served as JSON at
  `developer.apple.com/tutorials/data/design/human-interface-guidelines/<page>.json`. Shopify's pages serve markdown
  when `.md` is added to the URL. Carbon, Primer, Geist, GOV.UK, Radix and shadcn read fine as plain pages.
- Systems rename and move pages. If a page returns 404, search the system's own site for the component, and fix the
  index in the same change (or say so in your hand-in) rather than quote a dead link.
- A component can be missing from one system on purpose (HIG has no tooltip or empty-state page; Atlassian has no
  chart component; M3 has no table). Say "none" in the notes; don't stretch a neighbour page to cover it.

## Using them properly

- **Cite, summarise in your own words, never copy.** The notes hold a link, the date and a sentence of your own per
  point. No pasted passages, no copied images, icons, illustrations, screenshots or fonts into the product or its
  deliverables; a reference picture stays in the design folder as study material (`design-references`).
- **Respect licences.** Reading and learning from docs is open to everyone; reusing their assets or code is not, unless
  the licence says so. Carbon and Polaris code is open source, Primer and Radix and shadcn are MIT, GOV.UK content is under
  the Open Government Licence (its crest and brand are not), and Apple's and Google's guidance and artwork carry their own
  terms. When you take code from a registry (`equipe design component`, shadcn), note its source and licence in the
  notes. Never paste a premium or unlicensed component.
- **No brand leakage.** Their names, logos, colours and wording are theirs; the product's copy and tokens are its own.
- **Dates matter.** Docs change. A note older than six months for a pattern is re-read before you rely on it.

## Review checklist

- [ ] The product kind is named and the pages read fit it (no Apple page behind a web product's layout, no web system
      behind a mobile app's controls).
- [ ] Each component has two or three pages read, with URL and date in its `notes.md`.
- [ ] `notes.md` ends with what the pages agree on and where they differ, in our own words.
- [ ] The design follows the agreement, or says why not; the product's accepted system is not overridden.
- [ ] A component under review was held beside the same pages, and each difference is a finding.
- [ ] Nothing copied: no text, images, icons or code without its licence noted.

Sources: the public documentation of IBM Carbon, GitHub Primer, Shopify Polaris, Vercel Geist, Atlassian Design System,
GOV.UK Design System, Material Design 3, Apple Human Interface Guidelines, Radix and shadcn/ui, indexed here by page and
summarised in our own words (see `ui-review`'s `references/sources.md`).
