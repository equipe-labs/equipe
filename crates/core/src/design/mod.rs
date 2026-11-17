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
//! - `inherit`: machine-level brands a project inherits its system from,
//!   with the project's overrides laid over them (`docs/BRANDS.md`).
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
//! - `results`: what a design ticket made — its renders paired with the
//!   screen as it was, its change notes, preview links and decision record —
//!   one card each, for the Designs tab, the Needs-you item and the PM's
//!   acceptance message.
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
//! - `captures`: the pictures and videos the design tools make, kept out of
//!   git — listed in `captures.json`, their bytes in equipe's store, on disk
//!   where they always were (docs/DESIGN-CAPTURES.md).
//! - `session`: design sessions — the kickoff, the person's pins, the lines
//!   `equipe design session` prints, and the PM's proposals, once per ticket.

pub mod brand;
pub mod captures;
pub mod check;
pub mod component;
pub mod fonts;
pub mod inherit;
pub mod palette;
pub mod precheck;
pub mod qa_stats;
pub mod refgallery;
pub mod results;
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
pub mod session;
pub mod studio;
pub mod system;
pub mod tokens;
pub mod values;
pub mod versions;

pub use folder::{DIR, catalog, places, stamp};

/// `f` of each of `items`, in their order, worked out on up to `at_once`
/// threads: the folders read and the git asked for a squad's design systems
/// are many, each mostly a wait on the disk or on a process.
pub(crate) fn each_at_once<T: Sync, R: Send>(items: &[T], at_once: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    if items.len() < 2 || at_once < 2 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::with_capacity(items.len()));
    std::thread::scope(|scope| {
        for _ in 0..items.len().min(at_once) {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(at) else { break };
                    let out = f(item);
                    done.lock().unwrap_or_else(|e| e.into_inner()).push((at, out));
                }
            });
        }
    });
    let mut done = done.into_inner().unwrap_or_else(|e| e.into_inner());
    done.sort_by_key(|(at, _)| *at);
    done.into_iter().map(|(_, out)| out).collect()
}
