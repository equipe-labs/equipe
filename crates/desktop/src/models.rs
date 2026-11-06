//! What the model and effort menus offer, from the catalog in core
//! (`equipe_core::models`) and what the daemon's engines list
//! (`Event::Models`): for Claude Code each family's latest — its alias,
//! which follows Claude Code's newest — and its models by id; for Codex what
//! its own list says. The member's model is always among them. When the
//! model that runs isn't the one asked for, a line says so in plain words.

use equipe_core::model_providers as providers;
use equipe_core::models::{self as catalog, Gap};
use equipe_core::protocol::{Engine, ModelChoice, ProviderStatus};
use equipe_core::t;

/// One model to pick: what it reads as, what is said beside it, and whether
/// it is the one in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub model: String,
    pub label: String,
    pub hint: String,
    pub on: bool,
}

/// A model as a menu names it: `Opus · latest`, `Opus 5.5`, or what its
/// engine's list calls it.
pub fn label(model: &str, listed: &[ModelChoice]) -> String {
    // A provider's model by its own id, without the provider (said beside it).
    if providers::split_ref(model).is_some() {
        return providers::display(model).to_owned();
    }
    if catalog::is_alias(model)
        && let Some(family) = catalog::family(model)
    {
        return t!("models.latest", family = family.name);
    }
    if let Some(name) = catalog::name(model) {
        return name;
    }
    listed.iter().find(|m| m.id == model).and_then(|m| m.name.clone()).unwrap_or_else(|| model.to_owned())
}

/// The models a member on `engine` can be set to, the one it is set to
/// (`asked`, else the one that runs) checked. `ran`: what its engine named.
pub fn choices(engine: Option<Engine>, asked: Option<&str>, ran: Option<&str>, listed: &[ModelChoice]) -> Vec<Choice> {
    let now = asked.or(ran).map(catalog::bare);
    let mut out: Vec<Choice> = Vec::new();
    match engine {
        Some(Engine::Claude) => {
            for family in catalog::CLAUDE_FAMILIES {
                let on = now.as_deref() == Some(family.alias);
                // The latest, and — when it is the one in use — what it is here.
                let hint = match ran.filter(|_| on).and_then(catalog::name) {
                    Some(name) => t!("models.runs_as", model = name),
                    None => t!("models.latest_hint").to_owned(),
                };
                out.push(Choice { model: family.alias.to_owned(), label: label(family.alias, listed), hint, on });
                for pinned in catalog::CLAUDE_MODELS.iter().filter(|p| p.family == family.alias) {
                    out.push(Choice { model: pinned.id.to_owned(), label: label(pinned.id, listed), hint: String::new(), on: now.as_deref() == Some(pinned.id) });
                }
            }
        }
        Some(Engine::Codex) => {
            for m in listed {
                let label = label(&m.id, listed);
                let hint = if label == m.id { String::new() } else { m.id.clone() };
                out.push(Choice { model: m.id.clone(), label, hint, on: now.as_deref() == Some(m.id.as_str()) });
            }
        }
        None => {}
    }
    // What it is set to, though no list has it — a provider's is among its provider's ([`provider_groups`]).
    if let Some(current) = asked.or(ran)
        && !out.iter().any(|c| c.on)
        && providers::split_ref(current).is_none()
    {
        out.push(Choice { model: current.to_owned(), label: label(current, listed), hint: String::new(), on: true });
    }
    out
}

/// The most of a provider's models a menu lists; the rest are a `/model` away.
pub const PER_PROVIDER: usize = 6;

/// One provider's models in a menu, under its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub id: String,
    pub provider: String,
    pub choices: Vec<Choice>,
    /// How many more it lists than are shown.
    pub more: usize,
}

/// The models of the providers connected on the daemon's machine, each
/// provider's under its name, its first [`PER_PROVIDER`] — and the one the
/// member is set to (`asked`), checked, wherever it is in the list, or
/// though its provider lists it no longer.
pub fn provider_groups(connected: &[ProviderStatus], asked: Option<&str>) -> Vec<Group> {
    let asked = asked.filter(|a| providers::split_ref(a).is_some());
    let choice = |m: &ModelChoice| Choice { model: m.id.clone(), label: label(&m.id, &[]), hint: String::new(), on: asked == Some(m.id.as_str()) };
    let mut out: Vec<Group> = connected
        .iter()
        .filter(|p| !p.models.is_empty() || asked.and_then(providers::split_ref).is_some_and(|(id, _)| id == p.id))
        .map(|p| {
            let mut choices: Vec<Choice> = p.models.iter().take(PER_PROVIDER).map(choice).collect();
            if let Some(asked) = asked.filter(|a| providers::split_ref(a).is_some_and(|(id, _)| id == p.id) && !choices.iter().any(|c| c.on)) {
                choices.push(Choice { model: asked.to_owned(), label: label(asked, &[]), hint: String::new(), on: true });
            }
            let more = p.models.len().saturating_sub(PER_PROVIDER);
            Group { id: p.id.clone(), provider: p.name.clone(), choices, more }
        })
        .collect();
    // Set to a provider that isn't connected (any more): it still shows, by its id.
    if let Some((id, _)) = asked.and_then(providers::split_ref)
        && !out.iter().any(|g| g.id == id)
    {
        let asked = asked.unwrap_or_default();
        out.push(Group { id: id.to_owned(), provider: id.to_owned(), choices: vec![Choice { model: asked.to_owned(), label: label(asked, &[]), hint: String::new(), on: true }], more: 0 });
    }
    out
}

/// The line that says the model running isn't the one asked for, and
/// whether updating the engine's tool is the fix.
pub fn gap_line(engine: Option<Engine>, asked: Option<&str>, ran: Option<&str>, listed: &[ModelChoice]) -> Option<(String, bool)> {
    let engine = engine?;
    match catalog::gap(engine, asked, ran)? {
        Gap::Behind { family, ran, newest } => {
            let line = t!("models.behind", ran = label(&ran, listed), asked = label(family.alias, listed), newest = label(newest.id, listed));
            Some((line, true))
        }
        Gap::Elsewhere { asked, ran } => Some((t!("models.elsewhere", asked = label(&asked, listed), ran = label(&ran, listed)), false)),
        Gap::Older { newest } => Some((t!("models.older", newest = label(newest.id, listed)), false)),
    }
}

/// The efforts `model` takes on `engine`, lowest first: Codex's as its list
/// says, else what the catalog knows.
pub fn efforts(engine: Option<Engine>, model: Option<&str>, listed: &[ModelChoice]) -> Vec<String> {
    let listed = model.and_then(|m| listed.iter().find(|c| c.id == m)).map(|c| c.efforts.clone()).filter(|e| !e.is_empty());
    listed.unwrap_or_else(|| catalog::efforts(engine.unwrap_or(Engine::Claude), model).iter().map(|e| (*e).to_owned()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(choices: &[Choice]) -> Vec<(&str, bool)> {
        choices.iter().map(|c| (c.label.as_str(), c.on)).collect()
    }

    #[test]
    fn claude_offers_each_familys_latest_then_its_models_by_id_newest_first() {
        let listed = choices(Some(Engine::Claude), Some("opus"), None, &[]);
        let labels: Vec<String> = listed.iter().map(|c| c.label.clone()).collect();
        let latest = |family: &str| t!("models.latest", family = family);
        assert_eq!(
            labels,
            [latest("Fable"), "Fable 5.1".into(), latest("Opus"), "Opus 5.5".into(), "Opus 5".into(), "Opus 4.8".into(), latest("Sonnet"), "Sonnet 5.5".into(), "Sonnet 5".into(), latest("Haiku"), "Haiku 5.5".into()]
        );
        assert_eq!(listed.iter().filter(|c| c.on).map(|c| c.model.as_str()).collect::<Vec<_>>(), ["opus"]);
        assert_eq!(listed[2].hint, t!("models.latest_hint"), "the latest says it follows Claude Code");
        assert_eq!(listed[3].model, "claude-opus-5-5");
        assert!(listed[3].hint.is_empty());
    }

    #[test]
    fn the_latest_in_use_says_what_it_runs_as_here() {
        let listed = choices(Some(Engine::Claude), Some("opus"), Some("claude-opus-4-8"), &[]);
        let opus = listed.iter().find(|c| c.model == "opus").unwrap();
        assert!(opus.on);
        assert_eq!(opus.hint, t!("models.runs_as", model = "Opus 4.8"));
        // Pinned, it is that one that is checked.
        let pinned = choices(Some(Engine::Claude), Some("claude-opus-5-5"), Some("claude-opus-5-5[1m]"), &[]);
        assert_eq!(pinned.iter().filter(|c| c.on).map(|c| c.label.as_str()).collect::<Vec<_>>(), ["Opus 5.5"]);
    }

    #[test]
    fn the_members_own_model_is_always_listed_and_checked() {
        let older = choices(Some(Engine::Claude), None, Some("claude-sonnet-4-6"), &[]);
        assert_eq!(rows(&older).last(), Some(&("Sonnet 4.6", true)));
        let codex = choices(Some(Engine::Codex), Some("gpt-5.6-terra"), None, &[]);
        assert_eq!(rows(&codex), [("gpt-5.6-terra", true)]);
    }

    #[test]
    fn codex_offers_what_its_list_says_by_its_names() {
        let listed = [
            ModelChoice { id: "gpt-6.1-sol".into(), name: Some("GPT-6.1 Sol".into()), efforts: vec!["low".into(), "medium".into(), "xhigh".into()], default_effort: Some("medium".into()) },
            ModelChoice { id: "gpt-6-luna".into(), ..Default::default() },
        ];
        let offered = choices(Some(Engine::Codex), Some("gpt-6-luna"), None, &listed);
        assert_eq!(rows(&offered), [("GPT-6.1 Sol", false), ("gpt-6-luna", true)]);
        assert_eq!(offered[0].hint, "gpt-6.1-sol");
        assert_eq!(efforts(Some(Engine::Codex), Some("gpt-6.1-sol"), &listed), ["low", "medium", "xhigh"]);
        assert_eq!(efforts(Some(Engine::Codex), Some("gpt-6-luna"), &listed), ["minimal", "low", "medium", "high"], "its list said nothing");
    }

    fn provider(id: &str, name: &str, models: &[&str]) -> ProviderStatus {
        let models = models.iter().map(|m| ModelChoice { id: providers::model_ref(id, m), ..Default::default() }).collect();
        ProviderStatus { id: id.into(), preset: id.into(), name: name.into(), models, ..Default::default() }
    }

    #[test]
    fn each_providers_models_show_under_its_name_the_one_in_use_checked_wherever_it_is() {
        let many: Vec<String> = (0..12).map(|i| format!("org/model-{i}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let connected = [provider("openrouter", "OpenRouter", &many), provider("groq", "Groq", &["llama-3.3-70b-versatile"]), provider("ollama", "Ollama", &[])];
        let groups = provider_groups(&connected, None);
        assert_eq!(groups.iter().map(|g| (g.provider.as_str(), g.choices.len(), g.more)).collect::<Vec<_>>(), [("OpenRouter", PER_PROVIDER, 12 - PER_PROVIDER), ("Groq", 1, 0)], "none listed: not shown");
        assert_eq!((groups[1].choices[0].label.as_str(), groups[1].choices[0].model.as_str()), ("llama-3.3-70b-versatile", "@groq/llama-3.3-70b-versatile"));
        assert!(groups.iter().flat_map(|g| &g.choices).all(|c| !c.on));
        // Set to one past the first few: shown too, checked.
        let groups = provider_groups(&connected, Some("@openrouter/org/model-11"));
        let on: Vec<&str> = groups.iter().flat_map(|g| &g.choices).filter(|c| c.on).map(|c| c.label.as_str()).collect();
        assert_eq!(on, ["org/model-11"]);
        // Set to one whose provider is gone: still there, by its id.
        let groups = provider_groups(&connected, Some("@mistral/devstral-medium-latest"));
        assert_eq!(groups.last().map(|g| (g.provider.as_str(), g.choices[0].on)), Some(("mistral", true)));
        // The engine's own list doesn't list it again.
        let codex = choices(Some(Engine::Codex), Some("@groq/llama-3.3-70b-versatile"), Some("llama-3.3-70b-versatile"), &[ModelChoice { id: "gpt-6.1-sol".into(), ..Default::default() }]);
        assert_eq!(rows(&codex), [("gpt-6.1-sol", false)]);
        assert_eq!(label("@gemini/models/gemini-2.5-pro", &[]), "gemini-2.5-pro");
        assert_eq!(gap_line(Some(Engine::Codex), Some("@groq/llama-3.3-70b-versatile"), Some("llama-3.3-70b-versatile"), &[]), None, "it runs what it was set to");
    }

    #[test]
    fn efforts_follow_the_model() {
        assert_eq!(efforts(Some(Engine::Claude), Some("claude-opus-5-5"), &[]), ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(efforts(Some(Engine::Claude), Some("claude-sonnet-4-6"), &[]), ["low", "medium", "high", "max"]);
        assert!(efforts(Some(Engine::Claude), Some("claude-haiku-4-5"), &[]).is_empty());
    }

    #[test]
    fn a_latest_an_older_claude_code_runs_older_says_so_and_offers_the_update() {
        let (line, update) = gap_line(Some(Engine::Claude), Some("opus"), Some("claude-opus-4-8"), &[]).unwrap();
        assert!(update);
        assert_eq!(line, t!("models.behind", ran = "Opus 4.8", asked = t!("models.latest", family = "Opus"), newest = "Opus 5.5"));
        assert_eq!(gap_line(Some(Engine::Claude), Some("opus"), Some("claude-opus-5-5"), &[]), None);
        let (older, update) = gap_line(Some(Engine::Claude), Some("claude-opus-4-8"), None, &[]).unwrap();
        assert_eq!((older, update), (t!("models.older", newest = "Opus 5.5"), false));
        let (elsewhere, _) = gap_line(Some(Engine::Codex), Some("gpt-6.1-sol"), Some("gpt-6-luna"), &[]).unwrap();
        assert_eq!(elsewhere, t!("models.elsewhere", asked = "gpt-6.1-sol", ran = "gpt-6-luna"));
        assert_eq!(gap_line(None, Some("opus"), Some("claude-opus-4-8"), &[]), None);
    }
}
