//! The sprint loop on screen (`equipe_core::sprint`, docs/SPRINTS.md):
//! Progress leads with "This sprint" — its goal, what done looks like, how
//! far its tickets are, its budget, its hypotheses — then "What we learned"
//! from the last sprint's retro and results, and "Next sprint (proposed)"
//! with its ranked items, answered on its card. No UI types here, so the
//! words are tested without a window.

use equipe_core::protocol::growth::{Risk, Size};
use equipe_core::protocol::sprint::{Experiment, ExperimentStatus, Impact, Item, ItemStatus, Phase, Sprint, SprintView, Verdict};
use equipe_core::t;

/// One row under a section: its id (`H-2`, `P-7`), its words, where it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub state: String,
    /// It went the wrong way.
    pub hot: bool,
    /// It worked, or is recommended.
    pub good: bool,
}

/// A section's head: its sprint and goal, then a line or two under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub title: String,
    pub lines: Vec<String>,
}

/// "This sprint".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct This {
    pub head: Head,
    pub rows: Vec<Row>,
    /// Reviewed now, or in review already.
    pub can_end: bool,
}

/// "What we learned": the retro's parts, a label and lines each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Learned {
    pub head: Head,
    pub parts: Vec<(String, Vec<String>)>,
    pub rows: Vec<Row>,
}

/// "Next sprint (proposed)".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Next {
    pub head: Head,
    pub rows: Vec<Row>,
}

/// The three sections, and a line when the loop isn't running.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Shown {
    pub this: Option<This>,
    pub learned: Option<Learned>,
    pub next: Option<Next>,
    pub note: Option<String>,
}

fn pct(p: Option<f64>) -> String {
    p.map(|p| format!("{}{}%", if p >= 0.0 { "+" } else { "" }, (p * 10.0).round() / 10.0)).unwrap_or_default()
}

/// Where a hypothesis stands, in the person's words: red when it made things worse.
pub fn experiment_state(x: &Experiment) -> (String, bool, bool) {
    match (&x.status, &x.result) {
        (_, Some(r)) => match r.verdict {
            Verdict::Worked => (t!("sprint.exp_worked", pct = pct(r.change_pct)).trim().to_owned(), false, true),
            Verdict::DidNot => (t!("sprint.exp_did_not", pct = pct(r.change_pct)).trim().to_owned(), r.worse, false),
            Verdict::Inconclusive => (t!("sprint.exp_inconclusive").to_owned(), false, false),
        },
        (ExperimentStatus::Waiting, _) => (t!("sprint.exp_waiting").to_owned(), false, false),
        (ExperimentStatus::Live, _) => (t!("sprint.exp_measuring").to_owned(), false, false),
        (ExperimentStatus::Dropped, _) | (ExperimentStatus::Judged, None) => (t!("sprint.exp_dropped").to_owned(), false, false),
    }
}

fn experiment_rows<'a>(v: &'a SprintView, of: &'a Sprint) -> impl Iterator<Item = Row> + 'a {
    v.experiments.iter().filter(move |x| of.experiments.contains(&x.id)).map(|x| {
        let (state, hot, good) = experiment_state(x);
        Row { id: format!("H-{}", x.id), title: x.belief.clone(), state, hot, good }
    })
}

fn impact(i: Impact) -> &'static str {
    match i {
        Impact::High => t!("sprint.impact_high"),
        Impact::Medium => t!("sprint.impact_medium"),
        Impact::Low => t!("sprint.impact_low"),
    }
}

fn size(s: Size) -> &'static str {
    match s {
        Size::S => "S",
        Size::M => "M",
        Size::L => "L",
    }
}

/// An item's row: its title, then its impact, size, cost and risk — or that it started.
pub fn item_row(i: &Item) -> Row {
    let state = match i.status {
        ItemStatus::Started if i.auto_started => t!("sprint.item_auto").to_owned(),
        ItemStatus::Started => t!("sprint.item_started").to_owned(),
        _ => {
            let meta = t!("sprint.item_meta", impact = impact(i.impact), size = size(i.size), cost = equipe_core::sprint::tokens(i.cost_tokens));
            if i.recommended { format!("{meta} · {}", t!("sprint.recommended")) } else { meta }
        }
    };
    Row { id: format!("P-{}", i.id), title: i.title.clone(), state, hot: i.risk == Risk::High && i.status == ItemStatus::Proposed, good: i.recommended }
}

/// What Progress shows of the loop at `now`, `span` saying a duration in the person's words.
pub fn shown(v: &SprintView, now: u64, span: impl Fn(u64) -> String) -> Shown {
    let mut out = Shown::default();
    if !v.on && v.current.is_none() && v.last.is_none() {
        out.note = v.why_off.as_deref().map(|why| if why.contains("turned") { t!("sprint.off").to_owned() } else { t!("sprint.not_live").to_owned() });
        return out;
    }
    if let Some(c) = &v.current {
        let mut lines = vec![t!("sprint.done_when", done = c.done_when)];
        let when = match (c.phase, c.ends_at) {
            (Phase::Review, _) => t!("sprint.in_review").to_owned(),
            (_, Some(end)) if end > now => t!("sprint.review_in", span = span(end - now)),
            _ => t!("sprint.review_soon").to_owned(),
        };
        lines.push(t!("sprint.progress", done = v.current_done, total = c.tickets.len(), when = when));
        lines.push(t!("sprint.budget", spent = equipe_core::sprint::tokens(c.spent_tokens), budget = equipe_core::sprint::tokens(c.budget_tokens)));
        if !v.measurable && !c.experiments.is_empty() {
            lines.push(t!("sprint.not_measured").to_owned());
        }
        let rows = experiment_rows(v, c).collect();
        out.this = Some(This { head: Head { title: format!("S-{} · {}", c.id, c.goal), lines }, rows, can_end: c.phase == Phase::Active });
    }
    if let Some(l) = &v.last {
        let mut lines = Vec::new();
        if let Some(f) = &l.facts {
            lines.push(t!("sprint.facts", done = f.done, tickets = f.tickets, worked = f.worked, did_not = f.did_not, inconclusive = f.inconclusive));
        }
        let parts = l
            .retro
            .as_ref()
            .map(|r| {
                [(t!("sprint.shipped"), &r.shipped), (t!("sprint.learned"), &r.learned), (t!("sprint.wrong"), &r.wrong), (t!("sprint.next_change"), &r.next)]
                    .into_iter()
                    .filter(|(_, lines)| !lines.is_empty())
                    .map(|(label, lines)| (label.to_owned(), lines.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let rows = experiment_rows(v, l).filter(|r| !r.state.is_empty()).collect();
        out.learned = Some(Learned { head: Head { title: format!("S-{} · {}", l.id, l.goal), lines }, parts, rows });
    }
    if let Some(p) = &v.proposed {
        let mut items: Vec<&Item> = v.items.iter().filter(|i| p.items.contains(&i.id)).collect();
        items.sort_by_key(|i| i.rank);
        let lines = vec![t!("sprint.done_when", done = p.done_when), t!("sprint.waits").to_owned()];
        out.next = Some(Next { head: Head { title: format!("S-{} · {}", p.id, p.goal), lines }, rows: items.into_iter().map(item_row).collect() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::growth::Better;
    use equipe_core::protocol::sprint::{Facts, ItemKind, Judged, Metric, Retro, Rule};

    fn sprint(id: u32, phase: Phase) -> Sprint {
        Sprint { id, goal: format!("goal {id}"), done_when: "done".into(), phase, proposed_at: 0, started_at: Some(0), ends_at: Some(10_000), closed_at: None, items: vec![], tickets: vec![1, 2, 3], experiments: vec![], budget_tokens: 25_000_000, spent_tokens: 4_000_000, set: 0, retro: None, facts: None }
    }

    fn experiment(id: u32, sprint: u32, verdict: Option<(Verdict, bool)>) -> Experiment {
        Experiment {
            id,
            sprint,
            belief: format!("belief {id}"),
            metric: Metric::Event { event: "signup".into(), filter: String::new() },
            rule: Rule { better: Better::Up, min_change_pct: 10.0, window_days: 7, min_sample: 100 },
            split: None,
            events: vec![],
            ticket: None,
            item: None,
            defined_by: "pm".into(),
            defined_at: 0,
            status: if verdict.is_some() { ExperimentStatus::Judged } else { ExperimentStatus::Live },
            live_at: Some(0),
            due_at: Some(1),
            checked: false,
            result: verdict.map(|(verdict, worse)| Judged { verdict, why: String::new(), worse, before: None, after: None, arms: vec![], change_pct: Some(if worse { -12.0 } else { 15.0 }), from: String::new(), to: String::new(), before_from: String::new(), before_to: String::new(), query: String::new(), source: String::new(), at: 0 }),
        }
    }

    fn item(id: u32, rank: u32, recommended: bool, status: ItemStatus) -> Item {
        Item { id, sprint: 4, kind: ItemKind::New, title: format!("item {id}"), change: String::new(), why: String::new(), evidence: vec![], expected: String::new(), impact: Impact::High, size: Size::S, risk: Risk::Low, cost_tokens: 2_000_000, owner: None, project: None, hypothesis: None, outside: false, rank, recommended, may_auto_start: true, status, auto_started: false, ticket: None, experiment: None }
    }

    #[test]
    fn progress_shows_this_sprint_what_we_learned_and_the_next_proposed() {
        let mut current = sprint(3, Phase::Active);
        current.experiments = vec![5];
        let mut last = sprint(2, Phase::Closed);
        last.experiments = vec![4];
        last.retro = Some(Retro { shipped: vec!["A shorter form".into()], learned: vec!["Fewer fields win".into()], wrong: vec![], next: vec!["One-tap sign-in".into()] });
        last.facts = Some(Facts { tickets: 4, done: 3, worked: 1, ..Facts::default() });
        let mut proposed = sprint(4, Phase::Proposed);
        proposed.items = vec![7, 8];
        let v = SprintView {
            on: true,
            current: Some(current),
            current_done: 2,
            last: Some(last),
            proposed: Some(proposed),
            items: vec![item(8, 2, false, ItemStatus::Proposed), item(7, 1, true, ItemStatus::Proposed)],
            experiments: vec![experiment(5, 3, None), experiment(4, 2, Some((Verdict::DidNot, true)))],
            measurable: true,
            ..SprintView::default()
        };
        let s = shown(&v, 1_000, |secs| format!("{secs}s"));
        let this = s.this.unwrap();
        assert_eq!(this.head.title, "S-3 · goal 3");
        assert_eq!(this.rows.len(), 1);
        assert!(this.can_end);
        let learned = s.learned.unwrap();
        assert_eq!(learned.parts.len(), 3, "an empty part isn't shown");
        assert!(learned.rows[0].hot, "it went the wrong way");
        let next = s.next.unwrap();
        assert_eq!(next.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["P-7", "P-8"], "by rank");
        assert!(next.rows[0].good && !next.rows[1].good);
        assert!(s.note.is_none());
    }

    #[test]
    fn before_anything_is_live_progress_says_why_there_is_no_sprint() {
        let off = SprintView { on: false, why_off: Some("nothing is live yet: sprints start once the product is".into()), ..SprintView::default() };
        let s = shown(&off, 0, |_| String::new());
        assert!(s.this.is_none() && s.next.is_none());
        assert!(s.note.is_some());
        let started = item(1, 1, true, ItemStatus::Started);
        assert_ne!(item_row(&started).state, item_row(&item(1, 1, true, ItemStatus::Proposed)).state);
    }
}
