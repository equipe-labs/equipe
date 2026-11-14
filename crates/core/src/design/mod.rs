//! A squad's design system (`docs/DESIGN-SYSTEM.md`): the `design/` folder its
//! designer keeps — in the squad's folder for the squad, in a project's
//! repository for a project with a look of its own — read for the screens.
//!
//! - `tokens`: `tokens.json` (W3C Design Tokens): groups, `$type` inherited,
//!   aliases resolved, a value per mode.
//! - `values`: what a token's value is — a colour, a size, a curve, a type style.
//! - `contrast`: WCAG 2.x contrast, and which colour is the background.
//! - `diff`: what changed since the person last looked.
//! - `critique`: the designer's pictures, critiqued by a fresh reviewer.
//! - `reviewer`: which engine reviews the designer's work — the other one when
//!   it can take it, else a fresh session of the member's own — and why.
//! - `lint`: a screen held to rules in code — the floor never waived, the
//!   habits of generated screens kept only with a reason, finishing notes —
//!   in its source and as a browser draws it, and what a hand-in needs.
//! - `second_look`: a reviewer started fresh, given only the pictures, the
//!   brief and the system, whose verdict a hand-in waits on.
//! - `directions`: the directions proposed and chosen, so the next differs.
//! - `brand`: which tickets are brand work, and what a designer starting one is told.
//! - `refs`: a brief's reference board — pages from the galleries a designer
//!   studies, each with what to learn from it, never another's brand.
//! - `palette`: candidate palettes on a real layout, light and dark, with
//!   their contrast, as tokens.
//! - `fonts`: candidate type pairs as a specimen, in the product's
//!   languages, with coverage, licence and weight.
//! - `component`: a public component's source pulled in as a starting
//!   point, with its source and licence, listed for restyling.
//! - `kept`: what a screen offers, recorded before a redesign and checked after.
//! - `flows`: the service's UI map — every flow, screen and state, pictured.
//! - `review`: the designer's weekly review, held back until the setup is done.
//! - `folder`: where the folders are, what they hold, and the stamp that says
//!   one changed; the pictures in them are what `read_image` may give.
//! - `gallery`: the components a design system has, for its Components
//!   section — from the registry, the components' folder, the previews in
//!   `components/` and each matrix — and the ticket at work on each.
//! - `studio`: a ticket's design work as it is on disk, for a screen that
//!   watches it (checks, directions, critiques, references, pictures).
//! - `system`: a project's design system as code — its tokens, its
//!   component registry and docs pages, found wherever they are kept; each
//!   component's matrix of variants, states and themes checked in a browser;
//!   the registry checked and installed into a scratch project.
//! - `versions`: the accepted versions of a folder, from its history, and
//!   the ticket that asks for one back.
//! - `precheck`: design QA's deterministic look at a hand-in's screens —
//!   rendered at each condition, measured, compared with the map's pictures —
//!   before any designer turn (docs/DESIGN-QA.md).
//! - `qa_stats`: what design QA costs and decides, read from a squad's journal.

pub mod brand;
pub mod check;
pub mod component;
pub mod fonts;
pub mod palette;
pub mod precheck;
pub mod qa_stats;
pub mod refgallery;
pub mod refs;
pub mod kit;
pub mod lint;
pub mod contrast;
pub mod critique;
pub mod diff;
pub mod directions;
pub mod flows;
pub mod folder;
pub mod gallery;
pub mod kept;
pub mod review;
pub mod reviewer;
pub mod second_look;
pub mod studio;
pub mod system;
pub mod tokens;
pub mod values;
pub mod versions;

pub use folder::{DIR, catalog, places, stamp};
