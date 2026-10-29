//! A squad's design system (`docs/DESIGN-SYSTEM.md`): the `design/` folder its
//! designer keeps — in the squad's folder for the squad, in a project's
//! repository for a project with a look of its own — read for the screens.
//!
//! - `tokens`: `tokens.json` (W3C Design Tokens): groups, `$type` inherited,
//!   aliases resolved, a value per mode.
//! - `values`: what a token's value is — a colour, a size, a curve, a type style.
//! - `contrast`: WCAG 2.x contrast, and which colour is the background.
//! - `diff`: what changed since the person last looked.
//! - `folder`: where the folders are, what they hold, and the stamp that says
//!   one changed; the pictures in them are what `read_image` may give.

pub mod check;
pub mod kit;
pub mod contrast;
pub mod diff;
pub mod folder;
pub mod tokens;
pub mod values;

pub use folder::{DIR, catalog, places, stamp};
