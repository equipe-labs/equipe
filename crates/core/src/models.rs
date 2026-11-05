//! The models members run on, in one place: Claude Code's families with
//! their pinned versions, newest first, and the efforts each model takes.
//!
//! Claude Code resolves an alias (`opus`) by its own version, so an older
//! Claude Code runs an older model for it while the person believes they
//! have the newest. The catalog is what tells the two apart: the model the
//! engine names at startup, against the newest of its family here.
//!
//! Codex lists its own models (`model/list`, [`equipe_protocol::Event::Models`]);
//! what is here for it is only the efforts to offer when its list doesn't say.
//! One more model is one more line in [`CLAUDE_MODELS`].

use equipe_protocol::Engine;

/// A Claude Code family: the alias that follows its newest model, and its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Family {
    pub alias: &'static str,
    /// Its own name, the same in every language.
    pub name: &'static str,
}

/// Claude Code's families, as its menu would list them.
pub const CLAUDE_FAMILIES: [Family; 4] = [
    Family { alias: "fable", name: "Fable" },
    Family { alias: "opus", name: "Opus" },
    Family { alias: "sonnet", name: "Sonnet" },
    Family { alias: "haiku", name: "Haiku" },
];

/// A Claude model by its full id, which runs the same on any Claude Code
/// that knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pinned {
    pub id: &'static str,
    /// Its family's alias.
    pub family: &'static str,
    pub version: &'static str,
}

/// The Claude models offered by their id: by family, each family's newest first.
pub const CLAUDE_MODELS: [Pinned; 7] = [
    Pinned { id: "claude-fable-5-1", family: "fable", version: "5.1" },
    Pinned { id: "claude-opus-5-5", family: "opus", version: "5.5" },
    Pinned { id: "claude-opus-5", family: "opus", version: "5" },
    Pinned { id: "claude-opus-4-8", family: "opus", version: "4.8" },
    Pinned { id: "claude-sonnet-5-5", family: "sonnet", version: "5.5" },
    Pinned { id: "claude-sonnet-5", family: "sonnet", version: "5" },
    Pinned { id: "claude-haiku-5-5", family: "haiku", version: "5.5" },
];

/// Claude Code's efforts (`--effort`), lowest first.
pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Codex's reasoning efforts, for a model its list says nothing of.
pub const CODEX_EFFORTS: [&str; 4] = ["minimal", "low", "medium", "high"];

/// A model's id without what Claude Code adds to it: a context window
/// (`[1m]`) or a release date (`-20250929`).
pub fn bare(model: &str) -> String {
    let model = model.trim();
    let model = model.split_once('[').map_or(model, |(m, _)| m);
    match model.rsplit_once('-') {
        Some((head, date)) if date.len() == 8 && date.chars().all(|c| c.is_ascii_digit()) => head.to_owned(),
        _ => model.to_owned(),
    }
}

/// Whether `model` is a family's alias, which follows the engine's newest.
pub fn is_alias(model: &str) -> bool {
    CLAUDE_FAMILIES.iter().any(|f| f.alias == model.trim())
}

/// The Claude family `model` belongs to: an alias, or an id that names one.
pub fn family(model: &str) -> Option<&'static Family> {
    let bare = bare(model);
    let rest = bare.strip_prefix("claude-").unwrap_or(&bare);
    CLAUDE_FAMILIES.iter().find(|f| bare == f.alias || rest.split('-').next() == Some(f.alias))
}

/// A Claude id's version, as numbers to compare: `claude-opus-4-8` → `[4, 8]`.
/// `None` for an alias or another engine's id.
pub fn version(model: &str) -> Option<Vec<u32>> {
    let bare = bare(model);
    let rest = bare.strip_prefix("claude-")?;
    let numbers: Vec<u32> = rest.split('-').skip(1).map_while(|part| part.parse().ok()).collect();
    (!numbers.is_empty()).then_some(numbers)
}

/// The catalog's entry for `model`.
pub fn pinned(model: &str) -> Option<&'static Pinned> {
    let bare = bare(model);
    CLAUDE_MODELS.iter().find(|p| p.id == bare)
}

/// The newest model of a family, by its alias.
pub fn newest(alias: &str) -> Option<&'static Pinned> {
    CLAUDE_MODELS.iter().find(|p| p.family == alias)
}

/// A Claude id as people say it: `claude-opus-5-5` → `Opus 5.5`. `None` for
/// an alias (the screen says it is the latest) or what isn't Claude's.
pub fn name(model: &str) -> Option<String> {
    let family = family(model).filter(|_| !is_alias(model))?;
    let version = version(model)?;
    let version: Vec<String> = version.iter().map(u32::to_string).collect();
    Some(format!("{} {}", family.name, version.join(".")))
}

/// The efforts `model` takes on `engine`, lowest first. Claude's current
/// models take all five; Opus and Sonnet 4.6 have no `xhigh`, Opus 4.5 only
/// the first three, and older ones none. Codex's say so in its list
/// ([`equipe_protocol::ModelChoice::efforts`]); this is for when it doesn't.
pub fn efforts(engine: Engine, model: Option<&str>) -> &'static [&'static str] {
    match engine {
        Engine::Codex => &CODEX_EFFORTS,
        Engine::Claude => {
            let Some(version) = model.and_then(version) else { return &EFFORTS };
            let family = model.and_then(family).map(|f| f.alias);
            let (major, minor) = (version[0], version.get(1).copied().unwrap_or(0));
            match (major, minor) {
                _ if family.is_none() => &EFFORTS,
                (5.., _) | (4, 7..) => &EFFORTS,
                (4, 6) => &["low", "medium", "high", "max"],
                (4, 5) if family == Some("opus") => &["low", "medium", "high"],
                _ => &[],
            }
        }
    }
}

/// How what runs differs from what was asked, when it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gap {
    /// The alias runs an older model than its family's newest: the engine's
    /// tool is older than that model. The fix is the newest by its id, or
    /// the tool updated.
    Behind { family: &'static Family, ran: String, newest: &'static Pinned },
    /// Set to one model, the engine runs another.
    Elsewhere { asked: String, ran: String },
    /// Set by id to an older model than its family's newest.
    Older { newest: &'static Pinned },
}

/// What differs between the model a member is set to (`asked`) and the one
/// its engine named (`ran`), if anything worth a line.
pub fn gap(engine: Engine, asked: Option<&str>, ran: Option<&str>) -> Option<Gap> {
    let asked = asked.or(ran)?;
    let ran = ran.filter(|r| bare(r) != bare(asked));
    if engine != Engine::Claude {
        return ran.map(|ran| Gap::Elsewhere { asked: asked.to_owned(), ran: ran.to_owned() });
    }
    let asked_family = family(asked);
    if is_alias(asked) {
        let (family, ran) = (asked_family?, ran?);
        let newest = newest(family.alias)?;
        return match self::family(ran) {
            Some(f) if f == family => (version(ran)? < version(newest.id)?).then(|| Gap::Behind { family, ran: ran.to_owned(), newest }),
            _ => Some(Gap::Elsewhere { asked: asked.to_owned(), ran: ran.to_owned() }),
        };
    }
    if let Some(ran) = ran {
        return Some(Gap::Elsewhere { asked: asked.to_owned(), ran: ran.to_owned() });
    }
    let newest = newest(asked_family?.alias)?;
    (version(asked)? < version(newest.id)?).then_some(Gap::Older { newest })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_lists_each_family_newest_first_and_every_family_has_one() {
        for family in CLAUDE_FAMILIES {
            let versions: Vec<Vec<u32>> = CLAUDE_MODELS.iter().filter(|p| p.family == family.alias).map(|p| version(p.id).unwrap()).collect();
            assert!(!versions.is_empty(), "{} has a model", family.alias);
            assert!(versions.windows(2).all(|w| w[0] > w[1]), "{} newest first", family.alias);
        }
        // Families together, in the families' order.
        let order: Vec<&str> = CLAUDE_MODELS.iter().map(|p| p.family).collect();
        let mut seen: Vec<&str> = order.clone();
        seen.dedup();
        assert_eq!(seen, CLAUDE_FAMILIES.map(|f| f.alias));
        for p in CLAUDE_MODELS {
            assert_eq!(family(p.id).map(|f| f.alias), Some(p.family));
            assert_eq!(name(p.id).unwrap(), format!("{} {}", family(p.id).unwrap().name, p.version));
        }
    }

    #[test]
    fn an_id_reads_as_its_family_and_version_whatever_claude_code_adds() {
        assert_eq!(bare("claude-opus-5-5[1m]"), "claude-opus-5-5");
        assert_eq!(bare("claude-sonnet-4-5-20250929"), "claude-sonnet-4-5");
        assert_eq!(version("claude-opus-4-8"), Some(vec![4, 8]));
        assert_eq!(version("claude-opus-5"), Some(vec![5]));
        assert_eq!(version("opus"), None);
        assert_eq!(name("claude-opus-5-5[1m]").as_deref(), Some("Opus 5.5"));
        assert_eq!(name("claude-haiku-4-5-20251001").as_deref(), Some("Haiku 4.5"));
        assert_eq!(name("opus"), None, "an alias is the latest, said by the screen");
        assert_eq!(name("gpt-6.1-sol"), None);
        assert_eq!(family("sonnet").map(|f| f.name), Some("Sonnet"));
        assert!(is_alias("fable") && !is_alias("claude-fable-5-1"));
        assert_eq!(newest("opus").map(|p| p.id), Some("claude-opus-5-5"));
    }

    #[test]
    fn efforts_are_only_those_the_model_takes() {
        assert_eq!(efforts(Engine::Claude, Some("claude-opus-5-5")), EFFORTS);
        assert_eq!(efforts(Engine::Claude, Some("opus")), EFFORTS);
        assert_eq!(efforts(Engine::Claude, None), EFFORTS);
        assert_eq!(efforts(Engine::Claude, Some("claude-opus-4-8")), EFFORTS);
        assert_eq!(efforts(Engine::Claude, Some("claude-sonnet-4-6")), ["low", "medium", "high", "max"]);
        assert_eq!(efforts(Engine::Claude, Some("claude-opus-4-5-20251101")), ["low", "medium", "high"]);
        assert!(efforts(Engine::Claude, Some("claude-haiku-4-5")).is_empty());
        assert_eq!(efforts(Engine::Claude, Some("some-proxy-model")), EFFORTS, "what it can't place keeps every effort");
        assert_eq!(efforts(Engine::Codex, Some("gpt-6.1-sol")), CODEX_EFFORTS);
    }

    #[test]
    fn an_alias_an_older_claude_code_resolves_to_an_older_model_is_behind() {
        let gap = gap(Engine::Claude, Some("opus"), Some("claude-opus-4-8")).unwrap();
        let Gap::Behind { family, ran, newest } = gap else { panic!("{gap:?}") };
        assert_eq!((family.alias, ran.as_str(), newest.id), ("opus", "claude-opus-4-8", "claude-opus-5-5"));
        // The newest: nothing to say.
        assert_eq!(super::gap(Engine::Claude, Some("opus"), Some("claude-opus-5-5[1m]")), None);
        assert_eq!(super::gap(Engine::Claude, Some("opus"), None), None, "not run yet");
        // Another family than asked.
        assert!(matches!(super::gap(Engine::Claude, Some("fable"), Some("claude-opus-5-5")), Some(Gap::Elsewhere { .. })));
    }

    #[test]
    fn a_pinned_model_is_older_or_runs_elsewhere_or_is_fine() {
        assert_eq!(gap(Engine::Claude, Some("claude-opus-4-8"), Some("claude-opus-4-8")), Some(Gap::Older { newest: newest("opus").unwrap() }));
        assert_eq!(gap(Engine::Claude, None, Some("claude-opus-4-8")), Some(Gap::Older { newest: newest("opus").unwrap() }), "only what runs is known");
        assert_eq!(gap(Engine::Claude, Some("claude-opus-5-5"), Some("claude-opus-5-5[1m]")), None);
        assert_eq!(gap(Engine::Claude, Some("claude-sonnet-5-5"), None), None);
        assert_eq!(
            gap(Engine::Claude, Some("claude-opus-5-5"), Some("claude-opus-4-8")),
            Some(Gap::Elsewhere { asked: "claude-opus-5-5".into(), ran: "claude-opus-4-8".into() })
        );
        assert_eq!(gap(Engine::Codex, Some("gpt-6.1-sol"), Some("gpt-6.1-sol")), None);
        assert!(matches!(gap(Engine::Codex, Some("gpt-6.1-sol"), Some("gpt-6-luna")), Some(Gap::Elsewhere { .. })));
        assert_eq!(gap(Engine::Claude, Some("some-proxy-model"), None), None);
    }
}
