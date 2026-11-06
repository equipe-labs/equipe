//! The model picker's rules, with no UI types, so they are tested without a
//! window (`views/model_picker.rs` draws it). One picker, engine first:
//! Claude | Codex at the top, each with how much of its plan is used; that
//! engine's models below — Claude's by family, the latest first and the
//! pinned versions behind a disclosure; Codex's as its own live list says,
//! or why there is no list yet — then the effort the member's model takes,
//! and in plain words what a pick changes and when.
//!
//! Nothing here chooses a model on its own: a pick is the person's, sent as
//! the command the daemon already takes (`SetModel`, `SetEffort`, or
//! `MoveEngine` with the model picked there).

use std::time::Duration;

use equipe_core::model_providers as providers;
use equipe_core::models as catalog;
use equipe_core::plan_use::weights;
use equipe_core::protocol::engines::Proposal;
use equipe_core::protocol::{Engine, EngineState, ModelChoice, ProviderStatus};
use equipe_core::t;

use crate::models::{gap_line, label};
use crate::switching::{Meter, Tone, engine_name};

/// How long Codex may take to list its models before the picker says it didn't.
pub const CODEX_WAIT: Duration = Duration::from_secs(12);

/// The member the picker is for, as the screen knows it.
#[derive(Debug, Clone, Default)]
pub struct Member<'a> {
    pub name: &'a str,
    pub role: &'a str,
    pub engine: Option<Engine>,
    /// What it is set to (else what runs).
    pub asked: Option<&'a str>,
    /// What its engine named as running.
    pub ran: Option<&'a str>,
    pub effort: Option<&'a str>,
    /// Mid-turn.
    pub working: bool,
    /// On a ticket: a move to the other engine waits for it to end.
    pub on_ticket: bool,
}

impl Member<'_> {
    /// The engine it is on; one not yet started is taken as Claude Code's.
    pub fn on(&self) -> Engine {
        self.engine.unwrap_or(Engine::Claude)
    }
}

/// A move of the member to the other engine that hasn't happened yet: the
/// person's own (`applied`, with an undo), or one equipe suggests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moving {
    pub proposal: u64,
    pub to: Engine,
    pub model: Option<String>,
    pub applied: bool,
}

/// The move of `name` still to happen, among the cards on show.
pub fn moving<'a>(proposals: impl IntoIterator<Item = &'a Proposal>, name: &str) -> Option<Moving> {
    proposals.into_iter().find_map(|p| {
        let mv = p.moves.iter().find(|m| m.member == name && !m.done && !m.in_place())?;
        Some(Moving { proposal: p.id, to: mv.to, model: mv.model.clone(), applied: p.applied })
    })
}

/// What the picker knows besides the member.
#[derive(Debug, Clone, Default)]
pub struct World<'a> {
    /// Claude Code's models as the daemon listed them (empty: the catalog's).
    pub claude: &'a [ModelChoice],
    /// Codex's as it listed them; `None`: not (yet).
    pub codex: Option<&'a [ModelChoice]>,
    /// Where Codex stands on the daemon's machine; `None`: not said yet.
    pub codex_state: Option<EngineState>,
    /// How long ago Codex was asked for its list; `None`: not asked.
    pub codex_waited: Option<Duration>,
    pub providers: &'a [ProviderStatus],
    pub meters: &'a [Meter],
    /// The time the meters were read at (Unix seconds): a reset is said from it.
    pub now: i64,
    pub moving: Option<Moving>,
    /// The model a new member of its role starts with, on each engine.
    pub role_start: [Option<String>; 2],
}

impl World<'_> {
    fn listed(&self, engine: Engine) -> &[ModelChoice] {
        match engine {
            Engine::Claude => self.claude,
            Engine::Codex => self.codex.unwrap_or_default(),
        }
    }

    fn role_start(&self, engine: Engine) -> Option<&str> {
        self.role_start[slot(engine)].as_deref()
    }
}

fn slot(engine: Engine) -> usize {
    match engine {
        Engine::Claude => 0,
        Engine::Codex => 1,
    }
}

/// Both engines, in the order the tabs show them.
pub const ENGINES: [Engine; 2] = [Engine::Claude, Engine::Codex];

/// Where the keys are in the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The engine tabs (the one shown is the one focused).
    Tabs,
    /// A row of the list, by its place in [`View::rows`].
    Row(usize),
    /// An effort: 0 is the default, then [`Efforts::levels`] in order.
    Effort(usize),
    /// A link at the foot: 0 the plan's usage, 1 new members' defaults.
    Link(usize),
}

/// The picker's own state, while open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ui {
    pub tab: Engine,
    /// The Claude families whose pinned versions show.
    pub open: Vec<&'static str>,
    pub focus: Focus,
}

impl Ui {
    /// Opened on the member's engine, the family it is pinned in showing its
    /// versions — and the keys on what it is set to (`effort`: on its effort).
    pub fn new(member: &Member, world: &World, effort: bool) -> Self {
        let tab = member.on();
        let open = member.asked.filter(|a| !catalog::is_alias(a)).and_then(catalog::family).map(|f| f.alias).into_iter().collect();
        let mut ui = Ui { tab, open, focus: Focus::Tabs };
        let view = build(member, world, &ui);
        ui.focus = match (effort, &view.efforts) {
            (true, Some(e)) => Focus::Effort(e.now_index()),
            _ => view.first_focus(),
        };
        ui
    }
}

/// One engine's tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub engine: Engine,
    /// The member works on it now.
    pub here: bool,
    /// How much of its plan is used, as the window used most says it; `None`: never read.
    pub plan: Option<(String, Tone)>,
}

/// What a row does when picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// The member's model on its own engine (`SetModel`) — or a provider's, which
    /// the daemon moves it to Codex's agent for.
    Model(String),
    /// The member to the other engine (`MoveEngine`), on `model` if picked.
    Move { engine: Engine, model: Option<String> },
    /// A family's versions shown or hidden.
    Versions(&'static str),
    /// Codex asked for its list again.
    Retry,
    /// Settings' AI platforms, to sign in or set up.
    SignIn(Engine),
    /// Claude Code updated on the daemon's machine.
    UpdateClaude,
    /// The person's own move taken back (`UndoEngineProposal`).
    Undo(u64),
}

/// A model to pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRow {
    pub id: String,
    pub label: String,
    /// Beside the label, quieter: the version the latest is now, Codex's default…
    pub detail: String,
    /// One short line in plain words: what it is for, how hard it hits the plan.
    pub about: String,
    /// How hard it hits the plan, 1 (lightest) to 4, among this engine's: `None` when not known apart.
    pub weight: Option<u8>,
    pub on: bool,
    /// A pinned version, under its family.
    pub nested: bool,
    pub act: Act,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// A group's name (a provider's).
    Heading(String),
    /// Words that do nothing; `warn`: something is off.
    Note { text: String, warn: bool },
    /// Codex being asked for its list.
    Loading(String),
    Model(ModelRow),
    /// A family's pinned versions, shown or not.
    Versions { family: &'static str, count: usize, open: bool },
    Action { label: String, act: Act },
}

impl Row {
    pub fn act(&self) -> Option<Act> {
        match self {
            Row::Model(m) => Some(m.act.clone()),
            Row::Versions { family, .. } => Some(Act::Versions(family)),
            Row::Action { act, .. } => Some(act.clone()),
            Row::Heading(_) | Row::Note { .. } | Row::Loading(_) => None,
        }
    }
}

/// The efforts the member's model takes, to pick from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Efforts {
    pub levels: Vec<String>,
    pub now: Option<String>,
    /// Its engine's default for the model, when its list says.
    pub default: Option<String>,
    /// Set, though this model doesn't take it.
    pub stray: Option<String>,
}

impl Efforts {
    /// The effort a place stands for: 0 the default.
    pub fn at(&self, i: usize) -> Option<Option<String>> {
        match i {
            0 => Some(None),
            i => self.levels.get(i - 1).cloned().map(Some),
        }
    }

    pub fn now_index(&self) -> usize {
        self.now.as_ref().and_then(|n| self.levels.iter().position(|l| l == n)).map_or(0, |i| i + 1)
    }
}

/// What the effort section says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffortPart {
    Pick(Efforts),
    /// The model takes none.
    None(String),
    /// The tab isn't the member's engine.
    Elsewhere(String),
}

/// The picker as drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub tabs: Vec<Tab>,
    pub tab: Engine,
    pub rows: Vec<Row>,
    pub effort: EffortPart,
    pub efforts: Option<Efforts>,
    /// What a pick here changes, and when.
    pub applies: String,
    /// What new members of the role start with instead.
    pub role: Option<String>,
}

impl View {
    /// The rows the keys stop on.
    pub fn stops(&self) -> Vec<usize> {
        self.rows.iter().enumerate().filter(|(_, r)| r.act().is_some()).map(|(i, _)| i).collect()
    }

    /// Where the keys start on this tab: what the member is set to, else the first thing to do.
    pub fn first_focus(&self) -> Focus {
        let on = self.rows.iter().position(|r| matches!(r, Row::Model(m) if m.on));
        on.or_else(|| self.stops().first().copied()).map_or(Focus::Tabs, Focus::Row)
    }
}

/// The window of `engine`'s plan used most, in words: "week 61% used";
/// out, "week · Limit reached · resets in 3d".
pub fn plan_words(meters: &[Meter], engine: Engine, now: i64) -> Option<(String, Tone)> {
    let meter = meters.iter().find(|m| m.engine == engine && m.read_at.is_some())?;
    let g = meter.gauges.iter().min_by_key(|g| (std::cmp::Reverse(g.percent), g.resets_at))?;
    let window = crate::switching::window_name(g.window);
    let words = match g.tone {
        Tone::Out => t!("usage.window_out", window = window, span = crate::switching::span(g.resets_at, now)),
        _ => t!("usage.window_used", window = window, used = g.percent),
    };
    Some((words, if meter.stale { Tone::Calm } else { g.tone }))
}

/// How hard a model hits the plan, from the load weights (`plan_use::weights`):
/// its output price against the heaviest offered on its engine, in four steps.
/// `None` for a model the table doesn't name (it would only be its engine's
/// fallback), and for all when the table doesn't tell them apart.
fn weights_of(engine: Engine, ids: &[&str]) -> Vec<Option<u8>> {
    let price = |id: &str| {
        // An alias is weighed as what it runs now: its family's newest.
        let id = catalog::newest(id).filter(|_| catalog::is_alias(id)).map_or(id, |p| p.id);
        weights::known(engine, id).then(|| weights::prices(engine, id).output)
    };
    let prices: Vec<Option<f64>> = ids.iter().map(|id| price(id)).collect();
    let top = prices.iter().flatten().copied().fold(0., f64::max);
    let bottom = prices.iter().flatten().copied().fold(f64::MAX, f64::min);
    if top <= 0. || (top - bottom).abs() < f64::EPSILON {
        return vec![None; ids.len()];
    }
    prices
        .iter()
        .map(|p| {
            let r = (*p)? / top;
            Some(match r {
                r if r >= 0.75 => 4,
                r if r >= 0.3 => 3,
                r if r >= 0.1 => 2,
                _ => 1,
            })
        })
        .collect()
}

/// A weight in words.
pub fn weight_words(weight: u8) -> &'static str {
    match weight {
        4.. => t!("models.weight_4"),
        3 => t!("models.weight_3"),
        2 => t!("models.weight_2"),
        _ => t!("models.weight_1"),
    }
}

/// What a Claude family is for.
fn family_about(alias: &str) -> Option<&'static str> {
    Some(match alias {
        "fable" => t!("models.about_fable"),
        "opus" => t!("models.about_opus"),
        "sonnet" => t!("models.about_sonnet"),
        "haiku" => t!("models.about_haiku"),
        _ => return None,
    })
}

/// "what it is for · how hard it hits the plan", either alone when the other isn't known.
fn about_line(about: Option<&str>, weight: Option<u8>) -> String {
    match (about.filter(|a| !a.is_empty()), weight) {
        (Some(about), Some(w)) => t!("models.about_line", about = about, weight = weight_words(w)),
        (Some(about), None) => about.to_owned(),
        (None, Some(w)) => capitalized(weight_words(w)),
        (None, None) => String::new(),
    }
}

fn capitalized(words: &str) -> String {
    let mut chars = words.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// The efforts a Codex model takes, in a few words: "thinks low–xhigh".
fn efforts_range(efforts: &[String]) -> Option<String> {
    match efforts {
        [] => None,
        [one] => Some(t!("models.efforts_one", effort = one.clone())),
        [low, .., high] => Some(t!("models.efforts_range", low = low.clone(), high = high.clone())),
    }
}

/// What picking on `tab` changes for the member, and when — the truth of the
/// daemon's rules: a model on its own engine from the next turn; the other
/// engine at its boundary, between tickets, its work handed over.
pub fn applies(member: &Member, tab: Engine, listed: &[ModelChoice]) -> String {
    let name = member.name;
    if tab == member.on() {
        return match member.ran.or(member.asked).filter(|_| member.working) {
            Some(now) => t!("models.applies_working", name = name, model = label(now, listed)),
            None => t!("models.applies_turn", name = name),
        };
    }
    let (from, to) = (engine_name(member.on()), engine_name(tab));
    if member.on_ticket {
        t!("models.applies_ticket", name = name, from = from, to = to)
    } else if member.working {
        t!("models.applies_turn_end", name = name, to = to)
    } else {
        t!("models.applies_free", name = name, to = to)
    }
}

/// A move waiting, in words: where to and when.
pub fn moving_words(member: &Member, moving: &Moving, listed: &[ModelChoice]) -> String {
    let name = member.name;
    if !moving.applied {
        return t!("models.suggested", name = name, to = engine_name(moving.to));
    }
    let to = match &moving.model {
        Some(model) => format!("{} · {}", engine_name(moving.to), label(model, listed)),
        None => engine_name(moving.to).to_owned(),
    };
    if member.on_ticket {
        t!("models.moving_ticket", name = name, to = to)
    } else if member.working {
        t!("models.moving_turn", name = name, to = to)
    } else {
        t!("models.moving_free", name = name, to = to)
    }
}

/// Where Codex's list stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexList {
    Listed,
    /// Asked, not answered yet.
    Loading,
    /// Asked long enough ago that no answer is coming.
    Unlisted,
    SignedOut,
    Missing,
}

pub fn codex_list(world: &World) -> CodexList {
    if world.codex.is_some_and(|l| !l.is_empty()) {
        return CodexList::Listed;
    }
    match world.codex_state {
        Some(EngineState::NotInstalled) => CodexList::Missing,
        Some(EngineState::SignedOut) => CodexList::SignedOut,
        _ if world.codex_waited.is_some_and(|w| w >= CODEX_WAIT) => CodexList::Unlisted,
        _ => CodexList::Loading,
    }
}

/// The picker for `member`, on `ui`'s tab.
pub fn build(member: &Member, world: &World, ui: &Ui) -> View {
    let tab = ui.tab;
    let here = tab == member.on();
    let listed = world.listed(tab);
    let tabs = ENGINES.iter().map(|&engine| Tab { engine, here: engine == member.on(), plan: plan_words(world.meters, engine, world.now) }).collect();
    let current = member.asked.or(member.ran).map(catalog::bare);
    // A pick on the member's own engine sets its model; on the other, moves it there on that model.
    let pick = |id: &str| if here { Act::Model(id.to_owned()) } else { Act::Move { engine: tab, model: Some(id.to_owned()) } };
    let is_on = |id: &str| here && current.as_deref() == Some(id);
    let mut rows: Vec<Row> = Vec::new();

    if let Some(moving) = world.moving.as_ref().filter(|m| m.to == tab || here) {
        rows.push(Row::Note { text: moving_words(member, moving, world.listed(moving.to)), warn: false });
        if moving.applied {
            rows.push(Row::Action { label: t!("models.undo_move").to_owned(), act: Act::Undo(moving.proposal) });
        }
    }
    if here && let Some((line, update)) = gap_line(member.engine, member.asked, member.ran, listed) {
        rows.push(Row::Note { text: line, warn: true });
        if update {
            rows.push(Row::Action { label: t!("models.update").to_owned(), act: Act::UpdateClaude });
        }
    }

    match tab {
        Engine::Claude => {
            let mut ids: Vec<&str> = catalog::CLAUDE_FAMILIES.iter().map(|f| f.alias).collect();
            ids.extend(catalog::CLAUDE_MODELS.iter().map(|p| p.id));
            let weighed: Vec<(&str, Option<u8>)> = ids.iter().copied().zip(weights_of(Engine::Claude, &ids)).collect();
            let weight = |id: &str| weighed.iter().find(|(i, _)| *i == id).and_then(|(_, w)| *w);
            let start = world.role_start(Engine::Claude);
            for family in catalog::CLAUDE_FAMILIES {
                let on = is_on(family.alias);
                let newest = catalog::newest(family.alias).and_then(|p| catalog::name(p.id));
                // The latest, and — when it is the one in use and runs older — what it runs here.
                let detail = match member.ran.filter(|_| on).and_then(catalog::name) {
                    Some(ran) if Some(&ran) != newest.as_ref() => t!("models.runs_as", model = ran),
                    _ => newest.unwrap_or_default(),
                };
                let detail = if !here && start == Some(family.alias) { t!("models.role_pick").to_owned() } else { detail };
                let about = about_line(family_about(family.alias), weight(family.alias));
                rows.push(Row::Model(ModelRow { id: family.alias.into(), label: label(family.alias, listed), detail, about, weight: weight(family.alias), on, nested: false, act: pick(family.alias) }));
                let versions: Vec<_> = catalog::CLAUDE_MODELS.iter().filter(|p| p.family == family.alias).collect();
                if versions.is_empty() {
                    continue;
                }
                let open = ui.open.contains(&family.alias);
                rows.push(Row::Versions { family: family.alias, count: versions.len(), open });
                if open {
                    for p in versions {
                        let w = weight(p.id);
                        rows.push(Row::Model(ModelRow { id: p.id.into(), label: label(p.id, listed), detail: String::new(), about: String::new(), weight: w, on: is_on(p.id), nested: true, act: pick(p.id) }));
                    }
                }
            }
            // Set to a model the catalog doesn't list (older, or by hand): still there, checked.
            if here
                && let Some(asked) = member.asked.or(member.ran).filter(|a| providers::split_ref(a).is_none() && !catalog::is_alias(a) && catalog::pinned(a).is_none())
                && !rows.iter().any(|r| matches!(r, Row::Model(m) if m.on))
            {
                rows.push(Row::Model(ModelRow { id: asked.into(), label: label(asked, listed), detail: String::new(), about: String::new(), weight: None, on: true, nested: false, act: pick(asked) }));
            }
        }
        Engine::Codex => {
            let state = codex_list(world);
            match state {
                CodexList::Listed => {
                    if world.codex_state == Some(EngineState::SignedOut) {
                        rows.push(Row::Note { text: t!("models.codex_signed_out_listed").to_owned(), warn: true });
                        rows.push(Row::Action { label: t!("models.sign_in").to_owned(), act: Act::SignIn(Engine::Codex) });
                    }
                    let ids: Vec<&str> = listed.iter().map(|m| m.id.as_str()).collect();
                    let weights = weights_of(Engine::Codex, &ids);
                    let start = world.role_start(Engine::Codex);
                    for (m, weight) in listed.iter().zip(weights) {
                        let detail = if !here && start == Some(m.id.as_str()) {
                            t!("models.role_pick").to_owned()
                        } else if m.default {
                            t!("models.codex_default").to_owned()
                        } else {
                            String::new()
                        };
                        let what = match (m.description.as_deref(), efforts_range(&m.efforts)) {
                            (Some(d), Some(r)) => Some(format!("{} · {r}", d.trim_end_matches('.'))),
                            (Some(d), None) => Some(d.trim_end_matches('.').to_owned()),
                            (None, r) => r,
                        };
                        let about = about_line(what.as_deref(), weight);
                        rows.push(Row::Model(ModelRow { id: m.id.clone(), label: label(&m.id, listed), detail, about, weight, on: is_on(&m.id), nested: false, act: pick(&m.id) }));
                    }
                    if here
                        && let Some(asked) = member.asked.or(member.ran).filter(|a| providers::split_ref(a).is_none())
                        && !rows.iter().any(|r| matches!(r, Row::Model(m) if m.on))
                    {
                        rows.push(Row::Model(ModelRow { id: asked.into(), label: label(asked, listed), detail: String::new(), about: String::new(), weight: None, on: true, nested: false, act: pick(asked) }));
                    }
                }
                CodexList::Loading => rows.push(Row::Loading(t!("models.codex_loading").to_owned())),
                CodexList::Unlisted => {
                    rows.push(Row::Note { text: t!("models.codex_unlisted").to_owned(), warn: true });
                    rows.push(Row::Action { label: t!("models.retry").to_owned(), act: Act::Retry });
                }
                CodexList::SignedOut => {
                    rows.push(Row::Note { text: t!("models.codex_signed_out").to_owned(), warn: true });
                    rows.push(Row::Action { label: t!("models.sign_in").to_owned(), act: Act::SignIn(Engine::Codex) });
                }
                CodexList::Missing => {
                    rows.push(Row::Note { text: t!("models.codex_missing").to_owned(), warn: true });
                    rows.push(Row::Action { label: t!("models.set_up").to_owned(), act: Act::SignIn(Engine::Codex) });
                }
            }
            // No list, but Codex is there: the move itself is never out of reach.
            if !here && matches!(state, CodexList::Loading | CodexList::Unlisted) && world.moving.as_ref().is_none_or(|m| m.to != tab) {
                rows.push(Row::Action { label: t!("models.move_plain", name = member.name, engine = engine_name(tab)), act: Act::Move { engine: tab, model: None } });
            }
            // The providers connected there: their models run on Codex's agent.
            let asked = member.asked.filter(|a| providers::split_ref(a).is_some());
            let groups = crate::models::provider_groups(world.providers, asked);
            for group in &groups {
                rows.push(Row::Heading(group.provider.clone()));
                rows.extend(group.choices.iter().map(|c| {
                    Row::Model(ModelRow { id: c.model.clone(), label: c.label.clone(), detail: String::new(), about: String::new(), weight: None, on: c.on, nested: false, act: Act::Model(c.model.clone()) })
                }));
                if group.more > 0 {
                    rows.push(Row::Note { text: t!("models.provider_more", count = group.more, id = group.id.clone()), warn: false });
                }
            }
            if !groups.is_empty() {
                rows.push(Row::Note { text: t!("models.provider_limits").to_owned(), warn: false });
            }
        }
    }

    // The effort: what the member's model takes, on its own engine only.
    let model_now = member.asked.or(member.ran);
    let effort = if here {
        let mut levels = crate::models::efforts(Some(tab), model_now, listed);
        let now = member.effort.map(str::to_owned);
        let stray = now.clone().filter(|n| !levels.contains(n));
        levels.extend(stray.clone());
        let default = model_now.and_then(|m| listed.iter().find(|c| c.id == m)).and_then(|c| c.default_effort.clone());
        if levels.is_empty() {
            EffortPart::None(t!("models.effort_none", model = model_now.map(|m| label(m, listed)).unwrap_or_default()))
        } else {
            EffortPart::Pick(Efforts { levels, now, default, stray })
        }
    } else {
        EffortPart::Elsewhere(t!("models.effort_elsewhere", name = member.name, engine = engine_name(tab)))
    };
    let efforts = match &effort {
        EffortPart::Pick(e) => Some(e.clone()),
        _ => None,
    };
    let role = world.role_start(tab).map(|start| t!("models.role_line", name = member.name, role = member.role, model = label(start, listed)));
    View { tabs, tab, rows, effort, efforts, applies: applies(member, tab, listed), role }
}

/// A key, as the picker takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Tab,
    BackTab,
    Enter,
}

impl Key {
    pub fn parse(key: &str, shift: bool) -> Option<Key> {
        Some(match key {
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "tab" if shift => Key::BackTab,
            "tab" => Key::Tab,
            "enter" | "space" => Key::Enter,
            _ => return None,
        })
    }
}

/// What a key asks of the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Do {
    /// Show this engine's tab.
    Show(Engine),
    Run(Act),
    Effort(Option<String>),
    Link(usize),
}

/// The links at the foot: the plan's usage, then new members' defaults when said.
pub fn links(view: &View) -> usize {
    1 + usize::from(view.role.is_some())
}

/// Where `key` moves the keys from `focus`, and what it does. The zones, in
/// order: the tabs, the list, the efforts, the links. ↑↓ walk them, ←→ the
/// tabs or the efforts (on a family's versions: open, close), Tab jumps zones.
pub fn key(view: &View, focus: Focus, key: Key) -> (Focus, Option<Do>) {
    let stops = view.stops();
    let efforts = view.efforts.as_ref().map_or(0, |e| e.levels.len() + 1);
    let links = links(view);
    let other = |e: Engine| if e == Engine::Claude { Engine::Codex } else { Engine::Claude };
    let first_row = || stops.first().map(|&i| Focus::Row(i));
    let last_row = || stops.last().map(|&i| Focus::Row(i));
    let after_rows = || if efforts > 0 { Focus::Effort(view.efforts.as_ref().map_or(0, Efforts::now_index)) } else { Focus::Link(0) };
    match (focus, key) {
        (Focus::Tabs, Key::Left | Key::Right) => (Focus::Tabs, Some(Do::Show(other(view.tab)))),
        (Focus::Tabs, Key::Down | Key::Tab) => (first_row().unwrap_or_else(after_rows), None),
        (Focus::Tabs, Key::Up | Key::BackTab) => (Focus::Link(links - 1), None),
        (Focus::Tabs, Key::Enter) => (Focus::Tabs, None),
        (Focus::Row(at), Key::Down) => match stops.iter().position(|&i| i == at).and_then(|p| stops.get(p + 1)) {
            Some(&next) => (Focus::Row(next), None),
            None => (after_rows(), None),
        },
        (Focus::Row(at), Key::Up) => match stops.iter().position(|&i| i == at).filter(|&p| p > 0) {
            Some(p) => (Focus::Row(stops[p - 1]), None),
            None => (Focus::Tabs, None),
        },
        (Focus::Row(_), Key::Tab) => (after_rows(), None),
        (Focus::Row(_), Key::BackTab) => (Focus::Tabs, None),
        (Focus::Row(at), Key::Right | Key::Left) => match view.rows.get(at) {
            Some(Row::Versions { family, open, .. }) if *open == (key == Key::Left) => (focus, Some(Do::Run(Act::Versions(family)))),
            Some(Row::Versions { .. }) => (focus, None),
            _ => (Focus::Tabs, Some(Do::Show(other(view.tab)))),
        },
        (Focus::Row(at), Key::Enter) => (focus, view.rows.get(at).and_then(Row::act).map(Do::Run)),
        (Focus::Effort(i), Key::Left) => (Focus::Effort(i.saturating_sub(1)), None),
        (Focus::Effort(i), Key::Right) => (Focus::Effort((i + 1).min(efforts.saturating_sub(1))), None),
        (Focus::Effort(_), Key::Up | Key::BackTab) => (last_row().unwrap_or(Focus::Tabs), None),
        (Focus::Effort(_), Key::Down | Key::Tab) => (Focus::Link(0), None),
        (Focus::Effort(i), Key::Enter) => (focus, view.efforts.as_ref().and_then(|e| e.at(i)).map(Do::Effort)),
        (Focus::Link(i), Key::Left) => (Focus::Link(i.saturating_sub(1)), None),
        (Focus::Link(i), Key::Right) => (Focus::Link((i + 1).min(links - 1)), None),
        (Focus::Link(_), Key::Up | Key::BackTab) => (if efforts > 0 { after_rows() } else { last_row().unwrap_or(Focus::Tabs) }, None),
        (Focus::Link(_), Key::Down | Key::Tab) => (Focus::Tabs, None),
        (Focus::Link(i), Key::Enter) => (focus, Some(Do::Link(i))),
    }
}

/// The focus once the view changed under it (a tab switched, versions folded):
/// kept when it still lands on something, else where the tab starts.
pub fn settle(view: &View, focus: Focus) -> Focus {
    match focus {
        Focus::Row(at) if view.rows.get(at).is_some_and(|r| r.act().is_some()) => focus,
        Focus::Row(_) => view.first_focus(),
        Focus::Effort(i) => match &view.efforts {
            Some(e) => Focus::Effort(i.min(e.levels.len())),
            None => view.first_focus(),
        },
        Focus::Link(i) => Focus::Link(i.min(links(view) - 1)),
        Focus::Tabs => Focus::Tabs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::switching::{Gauge, tone};
    use equipe_core::protocol::engines::{Move, Why, Window};

    const NOW: i64 = 1_800_000_000;

    fn pm<'a>() -> Member<'a> {
        Member { name: "pm", role: "pm", engine: Some(Engine::Claude), asked: Some("claude-opus-5-5"), ran: Some("claude-opus-5-5"), ..Default::default() }
    }

    fn codex_models() -> Vec<ModelChoice> {
        let efforts = |e: &[&str]| e.iter().map(|e| (*e).to_owned()).collect();
        vec![
            ModelChoice { id: "gpt-6.1-sol".into(), name: Some("GPT-6.1 Sol".into()), efforts: efforts(&["low", "medium", "high", "xhigh"]), default_effort: Some("medium".into()), description: Some("Frontier agentic coding.".into()), default: true },
            ModelChoice { id: "gpt-6-luna".into(), name: Some("GPT-6 Luna".into()), efforts: efforts(&["minimal", "low"]), default_effort: Some("low".into()), ..Default::default() },
            ModelChoice { id: "gpt-6-mini".into(), ..Default::default() },
        ]
    }

    fn models(view: &View) -> Vec<(&str, bool, bool)> {
        view.rows.iter().filter_map(|r| if let Row::Model(m) = r { Some((m.label.as_str(), m.on, m.nested)) } else { None }).collect()
    }

    fn model<'v>(view: &'v View, id: &str) -> &'v ModelRow {
        view.rows.iter().find_map(|r| if let Row::Model(m) = r { (m.id == id).then_some(m) } else { None }).unwrap_or_else(|| panic!("no {id}"))
    }

    fn ui(tab: Engine) -> Ui {
        Ui { tab, open: vec![], focus: Focus::Tabs }
    }

    #[test]
    fn claude_shows_each_family_latest_first_its_versions_folded_away() {
        let member = pm();
        let world = World::default();
        let view = build(&member, &world, &ui(Engine::Claude));
        let latest = |f: &str| t!("models.latest", family = f);
        assert_eq!(models(&view).iter().map(|(l, ..)| l.to_string()).collect::<Vec<_>>(), [latest("Fable"), latest("Opus"), latest("Sonnet"), latest("Haiku")], "four rows, not a wall");
        let versions: Vec<(&str, usize, bool)> = view.rows.iter().filter_map(|r| if let Row::Versions { family, count, open } = r { Some((*family, *count, *open)) } else { None }).collect();
        assert_eq!(versions, [("fable", 1, false), ("opus", 3, false), ("sonnet", 2, false), ("haiku", 1, false)]);
        assert_eq!(model(&view, "opus").detail, "Opus 5.5", "the latest says what it is now");
        // Unfolded, every pinned version is still reachable — the one in use checked.
        let open = Ui { open: vec!["opus"], ..ui(Engine::Claude) };
        let view = build(&member, &world, &open);
        let opus: Vec<_> = models(&view).into_iter().filter(|(_, _, nested)| *nested).collect();
        assert_eq!(opus, [("Opus 5.5", true, true), ("Opus 5", false, true), ("Opus 4.8", false, true)]);
        assert!(!model(&view, "opus").on, "pinned is not the alias");
    }

    #[test]
    fn opening_on_a_pinned_version_shows_its_family_and_the_keys_land_on_it() {
        let member = pm();
        let world = World::default();
        let ui = Ui::new(&member, &world, false);
        assert_eq!((ui.tab, ui.open.clone()), (Engine::Claude, vec!["opus"]));
        let view = build(&member, &world, &ui);
        let Focus::Row(at) = ui.focus else { panic!("{:?}", ui.focus) };
        assert!(matches!(&view.rows[at], Row::Model(m) if m.id == "claude-opus-5-5" && m.on));
        // On the alias: nothing unfolds.
        let alias = Member { asked: Some("opus"), ..pm() };
        assert!(Ui::new(&alias, &world, false).open.is_empty());
    }

    #[test]
    fn each_model_says_what_it_is_for_and_how_hard_it_hits_the_plan_from_the_weights() {
        let view = build(&pm(), &World::default(), &Ui { open: vec!["opus"], ..ui(Engine::Claude) });
        let w = |id: &str| model(&view, id).weight;
        assert_eq!((w("fable"), w("opus"), w("sonnet"), w("haiku")), (Some(4), Some(3), Some(2), Some(1)));
        assert_eq!(w("claude-opus-5"), Some(3), "Opus 5 weighs as its own prices say");
        assert_eq!(model(&view, "opus").about, t!("models.about_line", about = t!("models.about_opus"), weight = t!("models.weight_3")));
        assert_eq!((model(&view, "claude-opus-4-8").about.as_str(), model(&view, "claude-opus-4-8").weight), ("", Some(3)), "a version: one line, its weight by the bars (and said to a screen reader)");
        // Codex's list: the table names only its small models, and one alone
        // has nothing to be weighed against — nothing is made up for the rest.
        let list = codex_models();
        let world = World { codex: Some(&list), ..Default::default() };
        let view = build(&pm(), &world, &ui(Engine::Codex));
        assert_eq!([model(&view, "gpt-6.1-sol").weight, model(&view, "gpt-6-luna").weight, model(&view, "gpt-6-mini").weight], [None, None, None]);
        assert_eq!(weights_of(Engine::Codex, &["gpt-6-mini", "gpt-6-nano", "gpt-6.1-sol"]), [Some(4), Some(2), None], "nano: a fifth of mini");
    }

    #[test]
    fn codex_shows_its_own_list_with_what_it_says_of_each() {
        let list = codex_models();
        let world = World { codex: Some(&list), codex_state: Some(EngineState::SignedIn), ..Default::default() };
        let view = build(&pm(), &world, &ui(Engine::Codex));
        assert_eq!(models(&view).iter().map(|(l, ..)| *l).collect::<Vec<_>>(), ["GPT-6.1 Sol", "GPT-6 Luna", "gpt-6-mini"]);
        let sol = model(&view, "gpt-6.1-sol");
        assert_eq!(sol.detail, t!("models.codex_default"));
        assert!(sol.about.starts_with(&format!("Frontier agentic coding · {}", t!("models.efforts_range", low = "low", high = "xhigh"))), "{}", sol.about);
        // From Claude, a pick there moves the member there, on that model.
        assert_eq!(sol.act, Act::Move { engine: Engine::Codex, model: Some("gpt-6.1-sol".into()) });
        assert!(!sol.on, "the member isn't on Codex");
        // On Codex, a pick sets the model and the one in use is checked.
        let backend = Member { name: "backend", role: "backend", engine: Some(Engine::Codex), asked: Some("gpt-6-luna"), ran: Some("gpt-6-luna"), ..Default::default() };
        let view = build(&backend, &world, &ui(Engine::Codex));
        assert_eq!(model(&view, "gpt-6-luna").act, Act::Model("gpt-6-luna".into()));
        assert!(model(&view, "gpt-6-luna").on);
    }

    #[test]
    fn without_codexs_list_the_picker_says_why_and_never_dead_ends() {
        let member = pm();
        let rows = |world: &World| build(&member, world, &ui(Engine::Codex)).rows;
        let loading = rows(&World { codex_state: Some(EngineState::SignedIn), codex_waited: Some(Duration::from_secs(1)), ..Default::default() });
        assert_eq!(loading[0], Row::Loading(t!("models.codex_loading").into()));
        assert!(loading.iter().any(|r| matches!(r.act(), Some(Act::Move { engine: Engine::Codex, model: None }))), "the move is still there");
        let unlisted = rows(&World { codex_state: Some(EngineState::SignedIn), codex_waited: Some(CODEX_WAIT), ..Default::default() });
        assert!(unlisted.iter().any(|r| r.act() == Some(Act::Retry)));
        let out = rows(&World { codex_state: Some(EngineState::SignedOut), ..Default::default() });
        assert_eq!(out[0], Row::Note { text: t!("models.codex_signed_out").into(), warn: true });
        assert_eq!(out[1].act(), Some(Act::SignIn(Engine::Codex)));
        assert!(!out.iter().any(|r| matches!(r.act(), Some(Act::Move { .. }))), "signed out: nothing to move to");
        let missing = rows(&World { codex_state: Some(EngineState::NotInstalled), ..Default::default() });
        assert_eq!(missing[0], Row::Note { text: t!("models.codex_missing").into(), warn: true });
        // Listed but signed out since: the list shows, and why a move would fail.
        let list = codex_models();
        let stale = rows(&World { codex: Some(&list), codex_state: Some(EngineState::SignedOut), ..Default::default() });
        assert_eq!(stale[0], Row::Note { text: t!("models.codex_signed_out_listed").into(), warn: true });
        assert_eq!(codex_list_state(None, None, None), CodexList::Loading, "not asked yet: asking");
    }

    fn codex_list_state(codex: Option<&[ModelChoice]>, state: Option<EngineState>, waited: Option<Duration>) -> CodexList {
        codex_list(&World { codex, codex_state: state, codex_waited: waited, ..Default::default() })
    }

    #[test]
    fn effort_offers_only_what_the_members_model_takes() {
        let efforts = |member: &Member, world: &World, tab| build(member, world, &ui(tab)).effort;
        let EffortPart::Pick(opus) = efforts(&pm(), &World::default(), Engine::Claude) else { panic!() };
        assert_eq!(opus.levels, ["low", "medium", "high", "xhigh", "max"]);
        let sonnet46 = Member { asked: Some("claude-sonnet-4-6"), ran: None, effort: Some("xhigh"), ..pm() };
        let EffortPart::Pick(older) = efforts(&sonnet46, &World::default(), Engine::Claude) else { panic!() };
        assert_eq!(older.levels, ["low", "medium", "high", "max", "xhigh"], "the one set though not taken, last");
        assert_eq!(older.stray.as_deref(), Some("xhigh"));
        let haiku45 = Member { asked: Some("claude-haiku-4-5"), ..pm() };
        assert!(matches!(efforts(&haiku45, &World::default(), Engine::Claude), EffortPart::None(_)));
        // Codex's: as its list says for that model, its default said.
        let list = codex_models();
        let world = World { codex: Some(&list), ..Default::default() };
        let luna = Member { engine: Some(Engine::Codex), asked: Some("gpt-6-luna"), ran: None, ..pm() };
        let EffortPart::Pick(e) = efforts(&luna, &world, Engine::Codex) else { panic!() };
        assert_eq!((e.levels.as_slice(), e.default.as_deref()), (&["minimal".to_owned(), "low".to_owned()][..], Some("low")));
        // The other engine's tab: set once it is there.
        assert_eq!(efforts(&pm(), &world, Engine::Codex), EffortPart::Elsewhere(t!("models.effort_elsewhere", name = "pm", engine = "Codex")));
        assert_eq!(Efforts { levels: vec!["low".into(), "high".into()], now: Some("high".into()), default: None, stray: None }.now_index(), 2);
    }

    #[test]
    fn what_a_pick_changes_and_when_in_plain_words() {
        let idle = pm();
        assert_eq!(applies(&idle, Engine::Claude, &[]), t!("models.applies_turn", name = "pm"));
        let working = Member { working: true, ..pm() };
        assert_eq!(applies(&working, Engine::Claude, &[]), t!("models.applies_working", name = "pm", model = "Opus 5.5"));
        let on_ticket = Member { on_ticket: true, working: true, ..pm() };
        assert_eq!(applies(&on_ticket, Engine::Codex, &[]), t!("models.applies_ticket", name = "pm", from = "Claude", to = "Codex"));
        assert_eq!(applies(&working, Engine::Codex, &[]), t!("models.applies_turn_end", name = "pm", to = "Codex"));
        assert_eq!(applies(&idle, Engine::Codex, &[]), t!("models.applies_free", name = "pm", to = "Codex"));
        // Only this member: what new members of its role start with is said beside it.
        let world = World { role_start: [Some("sonnet".into()), None], ..Default::default() };
        let view = build(&idle, &world, &ui(Engine::Claude));
        assert_eq!(view.role, Some(t!("models.role_line", name = "pm", role = "pm", model = t!("models.latest", family = "Sonnet"))));
        assert_eq!(build(&idle, &world, &ui(Engine::Codex)).role, None);
    }

    #[test]
    fn a_move_on_its_way_shows_with_its_undo_and_a_suggestion_without() {
        let mv = |done| Move { member: "pm".into(), role: "pm".into(), from: Engine::Claude, to: Engine::Codex, model: Some("gpt-6.1-sol".into()), effort: None, was_model: None, done };
        let card = |applied, done| Proposal { id: 7, why: Why::Person, engine: None, window: None, used_percent: 0, resets_at: 0, moves: vec![mv(done)], applied, proposed_at: 0 };
        let mine = [card(true, false)];
        let moving = super::moving(&mine, "pm").unwrap();
        assert_eq!(moving, Moving { proposal: 7, to: Engine::Codex, model: Some("gpt-6.1-sol".into()), applied: true });
        assert_eq!(super::moving(&[card(true, true)], "pm"), None, "made already");
        assert_eq!(super::moving(&mine, "backend"), None);
        let list = codex_models();
        let member = Member { on_ticket: true, ..pm() };
        let world = World { codex: Some(&list), moving: Some(moving.clone()), ..Default::default() };
        let view = build(&member, &world, &ui(Engine::Codex));
        assert_eq!(view.rows[0], Row::Note { text: t!("models.moving_ticket", name = "pm", to = "Codex · GPT-6.1 Sol"), warn: false });
        assert_eq!(view.rows[1].act(), Some(Act::Undo(7)));
        let suggested = Moving { applied: false, ..moving };
        let world = World { moving: Some(suggested), ..world };
        let view = build(&member, &world, &ui(Engine::Claude));
        assert_eq!(view.rows[0], Row::Note { text: t!("models.suggested", name = "pm", to = "Codex"), warn: false });
        assert!(!view.rows.iter().any(|r| matches!(r.act(), Some(Act::Undo(_)))));
    }

    #[test]
    fn each_tab_says_how_much_of_its_plan_is_used() {
        let gauge = |window, percent: u8| Gauge { window, percent, resets_at: NOW + 3600, tone: tone(percent) };
        let meters = [
            Meter { engine: Engine::Claude, gauges: vec![gauge(Window::FiveHour, 20), gauge(Window::Weekly, 61)], read_at: Some(NOW), stale: false },
            Meter { engine: Engine::Codex, gauges: vec![], read_at: None, stale: false },
        ];
        // The window used most, its number labelled: never a bare percent, never what is left.
        assert_eq!(plan_words(&meters, Engine::Claude, NOW), Some(("주간 61% 사용".to_owned(), Tone::Calm)));
        assert_eq!(plan_words(&meters, Engine::Codex, NOW), None, "never read");
        let near = [Meter { engine: Engine::Claude, gauges: vec![gauge(Window::FiveHour, 80), gauge(Window::Weekly, 61)], read_at: Some(NOW), stale: false }];
        assert_eq!(plan_words(&near, Engine::Claude, NOW), Some(("5시간 80% 사용".to_owned(), Tone::Near)));
        let out = [Meter { engine: Engine::Codex, gauges: vec![gauge(Window::FiveHour, 100)], read_at: Some(NOW), stale: false }];
        assert_eq!(plan_words(&out, Engine::Codex, NOW), Some(("5시간 한도 도달 · 1시간 뒤 초기화".to_owned(), Tone::Out)));
        let stale = [Meter { stale: true, ..near[0].clone() }];
        assert_eq!(plan_words(&stale, Engine::Claude, NOW).map(|(_, tone)| tone), Some(Tone::Calm), "an old reading isn't coloured");
        let world = World { meters: &meters, ..Default::default() };
        let view = build(&pm(), &world, &ui(Engine::Codex));
        assert_eq!(view.tabs.iter().map(|t| (t.engine, t.here)).collect::<Vec<_>>(), [(Engine::Claude, true), (Engine::Codex, false)]);
    }

    #[test]
    fn the_keys_walk_tabs_list_efforts_and_links() {
        let member = pm();
        let world = World { role_start: [Some("sonnet".into()), None], ..Default::default() };
        let view = build(&member, &world, &ui(Engine::Claude));
        let stops = view.stops();
        // Tabs: ←→ switch engines; ↓ into the list.
        assert_eq!(key(&view, Focus::Tabs, Key::Right), (Focus::Tabs, Some(Do::Show(Engine::Codex))));
        assert_eq!(key(&view, Focus::Tabs, Key::Down).0, Focus::Row(stops[0]));
        // The list: ↑ from the first goes back to the tabs, ↓ from the last to the efforts.
        assert_eq!(key(&view, Focus::Row(stops[0]), Key::Up).0, Focus::Tabs);
        assert_eq!(key(&view, Focus::Row(*stops.last().unwrap()), Key::Down).0, Focus::Effort(0));
        // A family's versions: → opens, ← closes; on a model, ←→ switch engines.
        let versions = view.rows.iter().position(|r| matches!(r, Row::Versions { family: "opus", .. })).unwrap();
        assert_eq!(key(&view, Focus::Row(versions), Key::Right).1, Some(Do::Run(Act::Versions("opus"))));
        assert_eq!(key(&view, Focus::Row(versions), Key::Left).1, None, "closed already");
        assert_eq!(key(&view, Focus::Row(stops[0]), Key::Right).1, Some(Do::Show(Engine::Codex)));
        assert_eq!(key(&view, Focus::Row(stops[0]), Key::Enter).1, Some(Do::Run(Act::Model("fable".into()))));
        // Efforts: ←→ along them, ⏎ sets one; 0 is the default.
        assert_eq!(key(&view, Focus::Effort(0), Key::Right).0, Focus::Effort(1));
        assert_eq!(key(&view, Focus::Effort(5), Key::Right).0, Focus::Effort(5), "max is the last");
        assert_eq!(key(&view, Focus::Effort(2), Key::Enter).1, Some(Do::Effort(Some("medium".into()))));
        assert_eq!(key(&view, Focus::Effort(0), Key::Enter).1, Some(Do::Effort(None)));
        // Links, then round to the tabs.
        assert_eq!(key(&view, Focus::Effort(0), Key::Tab).0, Focus::Link(0));
        assert_eq!(key(&view, Focus::Link(0), Key::Right).0, Focus::Link(1));
        assert_eq!(key(&view, Focus::Link(1), Key::Enter).1, Some(Do::Link(1)));
        assert_eq!(key(&view, Focus::Link(1), Key::Tab).0, Focus::Tabs);
        assert_eq!(key(&view, Focus::Tabs, Key::BackTab).0, Focus::Link(1));
        // A row gone after a change: the keys land where the tab starts.
        let codex = build(&member, &World::default(), &ui(Engine::Codex));
        assert_eq!(settle(&codex, Focus::Row(40)), codex.first_focus());
        assert_eq!(settle(&codex, Focus::Effort(3)), codex.first_focus(), "no efforts on the other engine");
    }
}
