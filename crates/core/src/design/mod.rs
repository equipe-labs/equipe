//! A squad's design system (`docs/DESIGN-SYSTEM.md`): the `design/` folder its
//! designer keeps — in the squad's folder for the squad, in a project's
//! repository for a project with a look of its own — read for the screens.
//!
//! - `tokens`: `tokens.json` (W3C Design Tokens): groups, `$type` inherited,
//!   aliases resolved, a value per mode.
//! - `values`: what a token's value is — a colour, a size, a curve, a type style.
//! - `contrast`: WCAG 2.x contrast, and which colour is the background.
//! - `diff`: what changed since the person last looked.
//! - `critique`: the designer's pictures, critiqued by the other engine.
//! - `lint`: a screen held to rules in code — the floor never waived, the
//!   habits of generated screens kept only with a reason, finishing notes —
//!   in its source and as a browser draws it, and what a hand-in needs.
//! - `second_look`: a reviewer started fresh, given only the pictures, the
//!   brief and the system, whose verdict a hand-in waits on.
//! - `directions`: the directions proposed and chosen, so the next differs.
//! - `kept`: what a screen offers, recorded before a redesign and checked after.
//! - `flows`: the service's UI map — every flow, screen and state, pictured.
//! - `review`: the designer's weekly review, held back until the setup is done.
//! - `folder`: where the folders are, what they hold, and the stamp that says
//!   one changed; the pictures in them are what `read_image` may give.
//! - `studio`: a ticket's design work as it is on disk, for a screen that
//!   watches it (checks, directions, critiques, references, pictures).
//! - `versions`: the accepted versions of a folder, from its history, and
//!   the ticket that asks for one back.

pub mod check;
pub mod kit;
pub mod lint;
pub mod contrast;
pub mod critique;
pub mod diff;
pub mod directions;
pub mod flows;
pub mod folder;
pub mod kept;
pub mod review;
pub mod second_look;
pub mod studio;
pub mod tokens;
pub mod values;
pub mod versions;

pub use folder::{DIR, catalog, places, stamp};
