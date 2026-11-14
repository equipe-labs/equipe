# Component index

One section per component or pattern. Each line is `System: pages — what it is especially good for`. A page is a path
under that system's root (the table at the end); join root and path to get the URL; a full URL stands as it is. Systems are listed in the order
that usually fits a web product first; for a mobile app start from HIG and Material. How to read each system's pages
is in `SKILL.md`.

## Actions and controls

### Button
- Carbon: `core/components/button/guidelines` — hierarchy (primary, secondary, tertiary, ghost, danger), one primary per view, icon-only buttons
- GOV.UK: `components/button/` — plain verbs as labels, start and warning buttons, double-submit prevention
- Atlassian: `components/button/` — appearance and spacing variants, loading and disabled states, button groups
- Primer: `components/button`, `components/icon-button` — sizes, leading and trailing visuals, when an icon needs a label
- Geist: `button`, `split-button` — dense dashboard sizes, loading and prefix/suffix slots
- M3: `components/buttons/overview`, `components/button-groups/overview` — emphasis levels and how many of each fit a screen
- HIG: `buttons` — platform button styles, tap size, role (primary, destructive)
- Radix/shadcn: `primitives/docs/components/toggle`, `shadcn:button` — build reference for variants and `asChild`

### Link
- GOV.UK: `components/back-link/`, `patterns/navigate-a-service/` — link text that makes sense out of context, back links
- Carbon: `core/components/link/guidelines` — inline vs standalone links, visited state
- Primer: `components/link` — underline rules inside prose, muted links
- Atlassian: `components/link/` — link vs button decision, external-link affordance

### Segmented control and toggle group
- Primer: `components/segmented-control` — switching views of the same data, icon-only variant, responsive collapse
- Carbon: `core/components/content-switcher/guidelines` — two to five peer views, not navigation
- HIG: `segmented-controls` — iOS segment limits and when to use tabs instead
- M3: `components/segmented-buttons/overview` — single vs multi select
- Radix/shadcn: `primitives/docs/components/toggle-group`, `shadcn:toggle-group` — roving focus build reference

### Menu and dropdown
- Carbon: `core/components/menu/guidelines`, `core/components/menu-buttons/guidelines` — menu button vs overflow, nested items, danger items
- Primer: `components/action-menu`, `components/action-list` — list item anatomy, selection modes, descriptions, dividers
- Atlassian: `components/dropdown-menu/`, `components/menu/` — grouping, checkbox/radio items, trigger choices
- Geist: `menu`, `dots-menu`, `context-menu` — compact row-action menus
- M3: `components/menus/overview` — dropdown vs exposed menus on touch
- HIG: `menus`, `pull-down-buttons`, `context-menus` — iOS and macOS menu structure, destructive placement
- Radix/shadcn: `primitives/docs/components/dropdown-menu`, `primitives/docs/components/context-menu`, `shadcn:dropdown-menu` — keyboard and typeahead behaviour

### Command palette
- Geist: `command-menu` — global search plus actions, keyboard hints
- shadcn: `command` — cmdk-based build reference
- Primer: `components/keybinding-hint` — showing the shortcut next to the action
- Atlassian: `components/spotlight/` — contextual hints that are not a palette; read for what a palette should not do

## Inputs and forms

### Text input and textarea
- GOV.UK: `components/text-input/`, `components/textarea/`, `components/character-count/` — label and hint placement, input width by expected length, autocomplete and input types
- Carbon: `core/components/text-input/guidelines`, `core/components/number-input/guidelines` — sizes, helper vs error text, read-only vs disabled
- Primer: `components/text-input`, `components/form-control` — label, caption, validation and required wiring
- Atlassian: `components/textfield/`, `components/textarea/` — compact mode, inline validation timing
- Polaris: `web-components/forms/text-field`, `web-components/forms/number-field`, `web-components/forms/money-field` — specialised fields for common data types
- M3: `components/text-fields/overview` — filled vs outlined, supporting text, touch sizing
- HIG: `text-fields`, `text-views`, `entering-data` — keyboard type, secure entry, reducing typing
- Radix/shadcn: `shadcn:input`, `shadcn:field`, `shadcn:input-group` — build reference for composed fields

### Select and combobox
- Carbon: `core/components/dropdown/guidelines`, `core/components/select/guidelines` — dropdown vs native select vs combobox decision, long lists
- Primer: `components/select`, `components/select-panel`, `components/autocomplete` — single vs multi, search inside the panel
- Atlassian: `components/select/` — async options, creatable, grouped and multi values
- Geist: `select`, `combobox`, `multi-select` — dashboard-density pickers
- GOV.UK: `components/select/` — when not to use a select at all (prefer radios or autocomplete)
- M3: `components/menus/overview`, `components/text-fields/overview` — exposed dropdown on mobile
- HIG: `pickers`, `pop-up-buttons`, `combo-boxes` — wheel, menu and inline pickers
- Radix/shadcn: `primitives/docs/components/select`, `shadcn:combobox`, `shadcn:native-select` — accessible listbox build references

### Checkbox, radio and switch
- GOV.UK: `components/checkboxes/`, `components/radios/` — grouping, conditional reveal, "none of these" option
- Carbon: `core/components/checkbox/guidelines`, `core/components/radio-button/guidelines`, `core/components/toggle/guidelines` — indeterminate state, toggle vs checkbox (instant vs submitted)
- Primer: `components/checkbox-group`, `components/radio-group`, `components/toggle-switch` — group labelling, switch that saves immediately
- Atlassian: `components/checkbox/`, `components/radio/`, `components/toggle/` — label and description layout
- Geist: `checkbox`, `radio`, `switch`, `choicebox` — card-style choice boxes for plans and options
- HIG: `toggles` — switch semantics and labelling
- M3: `components/checkbox/overview`, `components/radio-button/overview`, `components/switch/overview` — touch target and state layers
- Radix/shadcn: `primitives/docs/components/checkbox`, `primitives/docs/components/radio-group`, `primitives/docs/components/switch` — build references

### Date picker
- GOV.UK: `components/date-input/`, `patterns/dates/` — when three plain fields beat a calendar (known dates such as birth dates)
- Carbon: `core/components/date-picker/guidelines` — single and range, typed input with calendar
- Atlassian: `components/datetime-picker/`, `components/calendar/` — date, time and datetime combinations
- Polaris: `web-components/forms/date-field`, `web-components/forms/date-picker` — merchant date ranges
- M3: `components/date-pickers/overview`, `components/time-pickers/overview` — modal vs docked on small screens
- HIG: `pickers` — date and time picker styles on iOS
- Radix/shadcn: `shadcn:calendar`, `shadcn:date-picker` — build reference

### Form layout and validation
- GOV.UK: `patterns/validation/`, `components/error-summary/`, `components/error-message/`, `patterns/question-pages/` — the strongest public guidance on error wording, summary plus inline error, one question per page
- Carbon: `core/patterns/forms`, `core/components/form/guidelines` — field grouping, required vs optional, form width
- Primer: `ui-patterns/forms`, `ui-patterns/saving` — validation timing, auto-save vs explicit save
- Atlassian: `components/form/` — field wiring, form-level errors, submit states
- Polaris: `patterns/templates/settings` — settings forms with section layout
- HIG: `entering-data` — fewer fields, defaults, platform controls
- Radix/shadcn: `shadcn:field`, `primitives/docs/components/label` — build reference for labels and errors

### File upload
- GOV.UK: `components/file-upload/` — plain input behaviour, error cases, accessibility
- Carbon: `core/components/file-uploader/guidelines` — button vs drag-and-drop, per-file status and removal
- Polaris: `web-components/forms/drop-zone` — drop zone with accepted types and sizes
- Atlassian: `components/progress-bar/` — upload progress display
- HIG: `file-management` — pickers and documents on iOS

## Navigation and structure

### Tabs
- Carbon: `core/components/tabs/guidelines` — tabs vs content switcher, overflow of many tabs
- Primer: `components/underline-nav`, `components/underline-panels` — page-level nav vs in-page panels
- Atlassian: `components/tabs/` — tab panels, lazy content
- GOV.UK: `components/tabs/` — progressive enhancement, when tabs hide content people need
- M3: `components/tabs/overview` — primary vs secondary tabs, scrollable tabs
- Geist: `tabs` — dashboard tab styling
- Radix/shadcn: `primitives/docs/components/tabs`, `shadcn:tabs` — roving focus build reference

### Navigation: sidebar, top bar, breadcrumbs
- Carbon: `core/components/ui-shell-header/guidelines`, `core/components/ui-shell-left-panel/guidelines`, `core/patterns/global-header` — an enterprise app shell
- Atlassian: `components/side-navigation/`, `components/atlassian-navigation/`, `components/navigation-system/` — multi-product navigation, collapse, nested items
- Primer: `components/nav-list`, `components/page-layout`, `components/split-page-layout`, `ui-patterns/navigation` — list navigation with page layout
- Polaris: `patterns/compositions/interstitial-nav`, `web-components/layout-and-structure/page` — app navigation inside a host shell
- Geist: `breadcrumbs`, `entity` — breadcrumb and scoped entity switchers
- Carbon: `core/components/breadcrumb/guidelines` — when to show a trail and how to truncate
- Primer: `components/breadcrumbs` — breadcrumbs with overflow
- Atlassian: `components/breadcrumbs/` — breadcrumb items and truncation
- GOV.UK: `components/breadcrumbs/`, `components/service-navigation/`, `patterns/step-by-step-navigation/` — service navigation and plain-language labels
- Radix/shadcn: `shadcn:sidebar`, `shadcn:breadcrumb`, `primitives/docs/components/navigation-menu` — build references for an app shell

### Pagination
- Carbon: `core/components/pagination/guidelines` — items per page, page jump, table-attached pagination
- GOV.UK: `components/pagination/` — numbered pages for search results, next and previous labels
- Primer: `components/pagination` — compact vs numbered
- Atlassian: `components/pagination/` — truncation of long ranges
- Geist: `pagination`, `load-more-button`, `show-more` — load more vs pages
- Radix/shadcn: `shadcn:pagination` — build reference

### Stepper and wizard
- Carbon: `core/components/progress-indicator/guidelines` — step status, vertical and horizontal, when steps may be skipped
- Atlassian: `components/progress-tracker/` — stage tracker and completed states
- GOV.UK: `components/task-list/`, `patterns/complete-multiple-tasks/`, `patterns/check-answers/` — task lists and a final review page for long journeys
- Polaris: `patterns/compositions/setup-guide` — a setup checklist with progress
- HIG: `onboarding`, `steppers` — first-run flow; note that HIG's stepper is a number control, not a wizard

### Settings page
- Polaris: `patterns/templates/settings` — grouped sections with a title and description beside each card
- Primer: `ui-patterns/saving`, `components/page-layout` — save behaviour on settings pages
- Carbon: `core/patterns/forms`, `core/patterns/read-only-states` — long forms and read-only values
- HIG: `settings` — what belongs in the in-app settings versus the system settings
- GOV.UK: `patterns/check-answers/`, `components/summary-list/` — key-value summaries with change links

## Overlays

### Dialog and modal
- Carbon: `core/components/modal/guidelines`, `core/patterns/dialogs` — dialog types (passive, transactional, danger), sizes, focus
- Atlassian: `components/modal-dialog/` — header, footer and scrolling behaviour, size scale
- Primer: `components/dialog`, `components/confirmation-dialog` — dialog vs confirmation, narrow-screen behaviour
- Geist: `modal`, `destructive-action-modal`, `destructive-confirmation-input` — destructive confirmations that make people type a name
- M3: `components/dialogs/overview` — basic vs full-screen dialog, action alignment
- HIG: `alerts`, `action-sheets`, `modality` — when to interrupt at all, button order, alert vs sheet
- Radix/shadcn: `primitives/docs/components/dialog`, `primitives/docs/components/alert-dialog` — focus trap and scroll lock build reference

### Drawer and sheet
- Geist: `sheet`, `drawer` — side sheet versus bottom drawer
- Carbon: `core/components/ui-shell-right-panel/guidelines` — right-hand contextual panels
- M3: `components/side-sheets/overview`, `components/bottom-sheets/overview` — modal vs standard sheets by window size
- HIG: `sheets` — detents, grabbers, dismissal
- Atlassian: `components/drawer/` — side drawer sizes and use
- shadcn: `sheet`, `drawer` — build references

### Popover and tooltip
- Carbon: `core/components/popover/guidelines`, `core/components/tooltip/guidelines`, `core/components/toggletip/guidelines` — tooltip (label) vs toggletip (rich, interactive) vs popover
- Atlassian: `components/popup/`, `components/inline-dialog/`, `components/tooltip/` — placement and delay
- Primer: `components/popover`, `components/tooltip`, `components/anchored-overlay` — keyboard-accessible tooltips and overlays
- Geist: `tooltip`, `context-card` — hover cards for entities
- HIG: `popovers`, `offering-help` — popovers and help tags on Apple platforms
- Radix/shadcn: `primitives/docs/components/popover`, `primitives/docs/components/tooltip`, `primitives/docs/components/hover-card` — build references

## Feedback and status

### Toast, notification and banner
- Carbon: `core/components/notification/guidelines`, `core/patterns/notifications` — inline, toast and actionable notifications, when each persists
- Primer: `components/banner`, `ui-patterns/notification-messaging` — banner vs inline message vs toast, severity
- Atlassian: `components/flag/`, `components/banner/`, `components/section-message/`, `components/inline-message/` — four message types with clear roles
- Polaris: `web-components/feedback-and-status-indicators/banner` — page-level banners for merchants
- GOV.UK: `components/notification-banner/`, `components/warning-text/` — success and important banners, text that must not be missed
- Geist: `toast`, `banner`, `note`, `error` — dashboard messaging set
- M3: `components/snackbar/overview` — snackbar duration and single action
- HIG: `notifications`, `alerts` — push notifications vs in-app alerts
- shadcn: `sonner`, `alert` — build references

### Empty state
- Carbon: `core/patterns/empty-states` — first use, no results and error empties
- Primer: `components/blankslate`, `ui-patterns/empty-states` — heading, description and one action
- Polaris: `web-components/feedback-and-status-indicators/empty-state` — setup nudges in lists
- Geist: `empty-state` — dashboard empties
- Atlassian: `components/empty-state/` — layout and illustration use
- GOV.UK: `patterns/page-not-found-pages/`, `patterns/problem-with-the-service-pages/` — error and not-found pages written for people
- shadcn: `empty` — build reference

### Loading, skeleton and progress
- Carbon: `core/patterns/loading`, `core/components/loading/guidelines`, `core/components/inline-loading/guidelines` — which loader for which duration
- Primer: `ui-patterns/loading`, `components/skeleton-box`, `components/progress-bar`, `components/spinner` — loading patterns and skeleton parts
- Atlassian: `components/skeleton/`, `components/spinner/`, `components/progress-bar/` — skeleton vs spinner
- Geist: `skeleton`, `spinner`, `progress`, `loading-dots` — small loaders
- M3: `components/progress-indicators/overview`, `components/loading-indicator/overview` — determinate vs indeterminate
- HIG: `loading`, `progress-indicators` — launch and content loading on iOS
- Polaris: `web-components/feedback-and-status-indicators/progress`, `web-components/feedback-and-status-indicators/spinner` — admin loaders
- Radix/shadcn: `shadcn:skeleton`, `shadcn:spinner`, `primitives/docs/components/progress` — build references

### Badge, tag and status
- Carbon: `core/components/tag/guidelines`, `core/patterns/status-indicators` — tags vs status (icon plus text, never colour alone)
- Atlassian: `components/lozenge/`, `components/badge/`, `components/tag/` — lozenge for status, badge for counts, tag for categories
- Primer: `components/label`, `components/state-label`, `components/counter-label` — label vs counter vs state
- Geist: `badge`, `status-dot` — compact status marks
- GOV.UK: `components/tag/` — status tag colours and wording
- Polaris: `web-components/feedback-and-status-indicators/badge` — merchant status badges
- M3: `components/badges/overview`, `components/chips/overview` — notification badges vs filter chips
- HIG: `labels` — text labels and how to keep them short
- shadcn: `badge` — build reference

### Avatar
- Primer: `components/avatar`, `components/avatar-stack` — sizes and stacks
- Atlassian: `components/avatar/`, `components/avatar-group/` — presence and group overflow
- Geist: `avatar` — fallback initials
- Polaris: `web-components/media-and-visuals/avatar` — size and fallback
- shadcn: `avatar` — build reference

## Data

### Data table (sorting, filtering, density, row and bulk actions)
- Carbon: `core/components/data-table/guidelines`, `core/components/data-table/specifications` — density sizes, batch actions, expandable rows, selection, toolbar order
- Primer: `components/data-table` — sortable, paginated tables with loading and empty states
- Atlassian: `components/dynamic-table/`, `components/table/`, `components/table-tree/` — sortable tables, tree tables, loading and empty states
- Polaris: `patterns/compositions/index-table`, `patterns/templates/resource-index` — a full resource list with selection, bulk actions and filters
- Geist: `table` — a quiet developer-console table
- GOV.UK: `components/table/`, `components/summary-list/` — simple tables for few columns, numeric alignment
- HIG: `lists-and-tables`, `collections`, `outline-views` — row structure on Apple platforms
- Radix/shadcn: `shadcn:data-table`, `shadcn:table` — TanStack Table build reference (sorting, filtering, pagination, row selection)

### Filter bar, chips and search
- Carbon: `core/patterns/filtering`, `core/patterns/search`, `core/components/search/guidelines` — filter placement (toolbar, side panel, inline), applied-filter tags
- Primer: `components/text-input-with-tokens`, `components/token`, `components/label-group` — tokens as applied filters
- Polaris: `patterns/templates/resource-index`, `web-components/forms/search-field` — saved views plus filters on lists
- Atlassian: `components/tag-group/`, `components/tag/` — removable tags
- M3: `components/chips/overview`, `components/search/overview` — filter chips, search bar and search view
- HIG: `search-fields`, `searching` — search scopes and suggestions
- Geist: `search-input`, `clearable-input`, `filter-params` — search and clear behaviour
- GOV.UK: `components/accordion/`, `components/details/` — hiding optional filters

### KPI and stat tile
- Polaris: `patterns/compositions/metrics-card` — a metric with comparison and trend
- Carbon: `core/components/tile/guidelines`, `data-visualization/dashboards` — tile types and dashboards composition
- Primer: `components/card` — the card container used for stats
- Geist: `gauge`, `pie-gauge` — small gauges
- Atlassian: `components/primitives/metric-text` — numeric text primitive with sizes

### Charts and data visualisation
- Carbon: `data-visualization/overview`, `data-visualization/chart-anatomy`, `data-visualization/color-palettes`, `data-visualization/legends`, `data-visualization/axes-and-labels`, `data-visualization/dashboards`, `data-visualization/simple-charts` — the most complete public chart guidance: anatomy, categorical and sequential palettes, legends, axes
- Primer: `ui-patterns/data-visualization` — accessible colour and patterns for charts
- HIG: `charting-data`, `gauges` — Apple's guidance on chart design and descriptions
- shadcn: `chart` — Recharts build reference

### Card and tile
- Carbon: `core/components/tile/guidelines` — clickable, selectable and expandable tiles
- Primer: `components/card` — padding and content structure
- Polaris: `patterns/compositions/app-card`, `patterns/compositions/callout-card` — card compositions with one action
- M3: `components/cards/overview` — elevated, filled and outlined cards
- shadcn: `card` — build reference
- Atlassian: `components/tile/` — tile for entity previews

### List and structured list
- Carbon: `core/components/list/guidelines`, `core/components/structured-list/guidelines`, `core/components/contained-list/guidelines` — plain, structured and contained lists
- Primer: `components/action-list`, `components/tree-view` — rich list rows and trees
- Polaris: `patterns/compositions/resource-list` — a list of resources with a primary field
- M3: `components/lists/overview` — one, two and three-line items, leading and trailing elements
- HIG: `lists-and-tables` — list row anatomy, swipe actions
- GOV.UK: `components/summary-list/` — key and value pairs with actions
- shadcn: `item` — build reference

### Accordion and disclosure
- Carbon: `core/components/accordion/guidelines`, `core/patterns/disclosures` — disclosure choices
- GOV.UK: `components/accordion/`, `components/details/` — hiding secondary content without losing it
- Primer: `ui-patterns/progressive-disclosure`, `components/details` — progressive disclosure rules
- HIG: `disclosure-controls` — chevrons and disclosure groups

## Mobile

### Mobile tab bar and navigation bar
- HIG: `tab-bars`, `toolbars`, `sidebars`, `tab-views` — iOS tab bar, toolbar and sidebar rules; a navigation bar is covered in `toolbars`
- M3: `components/navigation-bar/overview`, `components/navigation-rail/overview`, `components/navigation-drawer/overview`, `components/app-bars/overview`, `components/toolbars/overview` — Android navigation by window size
- M3: `foundations/layout/scaffold/overview` — how bars and content sit together

### Pull to refresh and lists on mobile
- HIG: `lists-and-tables`, `scroll-views`, `loading` — list and loading guidance; HIG has no refresh page, so check how the platform's refresh control is described elsewhere before designing one
- Android: `compose/components/pull-to-refresh` — pull-to-refresh behaviour and state
- M3: `components/lists/overview`, `components/progress-indicators/overview` — list rows and the refresh indicator
- Android: `views/touch-and-input/swipe/respond-refresh-request` — when a refresh should trigger and how to respond

### Onboarding
- HIG: `onboarding` — fast, optional first run
- Carbon: `https://www.carbondesignsystem.com/building-experiences/onboard/articles/introduction-to-onboarding` — onboarding for products (articles, not components)
- Primer: `ui-patterns/feature-onboarding` — introducing a feature without blocking
- Polaris: `patterns/compositions/setup-guide` — a setup checklist
- Atlassian: `components/onboarding/`, `components/spotlight/` — spotlight tours and when to use them

## Roots

| System | Root |
|---|---|
| Carbon | `https://www.carbondesignsystem.com/building-blocks/` |
| Primer | `https://primer.style/product/` |
| Polaris | `https://shopify.dev/docs/api/app-home/latest/` |
| Geist | `https://vercel.com/geist/` |
| Atlassian | `https://atlassian.design/` |
| M3 | `https://m3.material.io/` |
| HIG | `https://developer.apple.com/design/human-interface-guidelines/` |
| GOV.UK | `https://design-system.service.gov.uk/` |
| Radix primitives | `https://www.radix-ui.com/` (paths start `primitives/`) |
| shadcn | `https://ui.shadcn.com/docs/components/radix/` (written `shadcn:<name>` on a mixed line) |
| Android | `https://developer.android.com/develop/ui/` |
