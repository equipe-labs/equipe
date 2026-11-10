//! What equipe knows of each component a registry is expected to have, as
//! data (`assets/design/components.json`): its WAI-ARIA pattern, the part an
//! example holds that a person uses, the states its docs page shows, the
//! keyboard behaviour it must have, how it opens, and the raw markup in
//! product code it replaces. Contributors extend the file, not this code.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

const DATA: &str = include_str!("../../../assets/design/components.json");

/// How an example's surface is opened.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Open {
    /// `click`, `focus`, `hover` or `contextmenu`.
    pub how: String,
    /// The selector of what opens (in the page, not only the example).
    pub surface: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Component {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// `core`: counted in coverage; `extra`: counted when present.
    pub set: String,
    /// The WAI-ARIA Authoring Practices pattern it follows (`none` when it isn't interactive).
    pub pattern: String,
    /// The selector of the part an example holds that a person uses.
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub open: Option<Open>,
    pub states: Vec<String>,
    pub keyboard: Vec<String>,
    /// Raw markup in product code that re-implements it (see [`Matcher`]).
    #[serde(default)]
    pub replaces: Vec<String>,
}

impl Component {
    pub fn core(&self) -> bool {
        self.set == "core"
    }

    /// Whether a person acts on it: it has a part to reach.
    pub fn interactive(&self) -> bool {
        self.target.is_some()
    }

    /// Whether `name` (a registry item's, a file's stem) is this component.
    pub fn is(&self, name: &str) -> bool {
        let n = normal(name);
        normal(&self.name) == n || self.aliases.iter().any(|a| normal(a) == n)
    }

    pub fn matchers(&self) -> Vec<Matcher> {
        self.replaces.iter().filter_map(|m| Matcher::parse(m)).collect()
    }
}

/// A name as files and registries write it: `AlertDialog`, `alert_dialog`, `alert-dialog`.
pub fn normal(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.trim().chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 && !out.ends_with('-') {
            out.push('-');
        }
        match c {
            '_' | ' ' | '.' => out.push('-'),
            c => out.push(c.to_ascii_lowercase()),
        }
    }
    out
}

#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    /// Each state's words.
    pub states: std::collections::BTreeMap<String, String>,
    /// Each keyboard check's words.
    pub keyboard: std::collections::BTreeMap<String, String>,
    pub components: Vec<Component>,
}

impl Catalog {
    pub fn get(&self, name: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.is(name))
    }

    pub fn core(&self) -> impl Iterator<Item = &Component> {
        self.components.iter().filter(|c| c.core())
    }

    pub fn state_words<'a>(&'a self, state: &'a str) -> &'a str {
        self.states.get(state).map_or(state, String::as_str)
    }

    pub fn keyboard_words<'a>(&'a self, check: &'a str) -> &'a str {
        self.keyboard.get(check).map_or(check, String::as_str)
    }
}

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| serde_json::from_str(DATA).expect("assets/design/components.json reads"));

pub fn catalog() -> &'static Catalog {
    &CATALOG
}

/// Raw markup a component replaces, as `components.json` writes it:
/// `button`, `styled button` (only with a class or style of its own),
/// `input[type=|text|email]` (an empty alternative: no type at all),
/// `[role=dialog]`, `[aria-modal]`, `nav[aria-label~=breadcrumb]`
/// (contains, any case) and `.animate-pulse`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matcher {
    pub styled: bool,
    pub tag: Option<String>,
    /// The attribute, and the values it may have (none: present at all).
    pub attr: Option<(String, Vec<String>, bool)>,
    pub class: Option<String>,
}

impl Matcher {
    pub fn parse(text: &str) -> Option<Matcher> {
        let (styled, rest) = match text.trim().strip_prefix("styled ") {
            Some(rest) => (true, rest.trim()),
            None => (false, text.trim()),
        };
        if let Some(class) = rest.strip_prefix('.') {
            return Some(Matcher { styled, tag: None, attr: None, class: Some(class.to_owned()) });
        }
        let (tag, attr) = match rest.find('[') {
            Some(at) => (&rest[..at], Some(rest[at + 1..].trim_end_matches(']'))),
            None => (rest, None),
        };
        let attr = attr.map(|a| {
            if let Some((name, values)) = a.split_once("~=") {
                (name.to_owned(), values.split('|').map(str::to_owned).collect(), true)
            } else if let Some((name, values)) = a.split_once('=') {
                (name.to_owned(), values.split('|').map(str::to_owned).collect(), false)
            } else {
                (a.to_owned(), Vec::new(), false)
            }
        });
        let tag = (!tag.is_empty()).then(|| tag.to_ascii_lowercase());
        (tag.is_some() || attr.is_some()).then_some(Matcher { styled, tag, attr, class: None })
    }

    /// Whether an element with `name`, attributes looked up by `attr` and
    /// `classes` is this markup.
    pub fn matches(&self, name: &str, attr: &dyn Fn(&str) -> Option<String>, classes: &[&str], styled: bool) -> bool {
        if self.styled && !styled {
            return false;
        }
        if let Some(tag) = &self.tag
            && tag != name
        {
            return false;
        }
        if let Some(class) = &self.class {
            return classes.iter().any(|c| c.rsplit(':').next() == Some(class.as_str()));
        }
        match &self.attr {
            None => true,
            Some((name, values, contains)) => {
                let value = attr(name);
                if values.is_empty() {
                    return value.is_some();
                }
                let value = value.unwrap_or_default().to_ascii_lowercase();
                if *contains { values.iter().any(|v| value.contains(&v.to_ascii_lowercase())) } else { values.contains(&value) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_holds_every_core_component_with_words_for_each_state_and_key() {
        let c = catalog();
        let core: Vec<&str> = c.core().map(|x| x.name.as_str()).collect();
        assert_eq!(core.len(), 49, "{core:?}");
        for name in ["accordion", "alert-dialog", "calendar", "combobox", "data-table", "input-otp", "navigation-menu", "resizable", "sidebar", "sonner", "toggle-group", "tooltip"] {
            assert!(c.get(name).is_some(), "{name}");
        }
        assert_eq!(c.get("Toast").map(|x| x.name.as_str()), Some("sonner"), "found by its alias");
        assert_eq!(c.get("AlertDialog").map(|x| x.name.as_str()), Some("alert-dialog"));
        let mut seen = std::collections::HashSet::new();
        for comp in &c.components {
            assert!(seen.insert(comp.name.clone()), "{} twice", comp.name);
            assert!(["core", "extra"].contains(&comp.set.as_str()), "{}", comp.name);
            assert!(comp.states.first().is_some_and(|s| s == "default"), "{}: default first", comp.name);
            for s in &comp.states {
                assert!(c.states.contains_key(s), "{}: state {s} has no words", comp.name);
            }
            for k in &comp.keyboard {
                assert!(c.keyboard.contains_key(k), "{}: key check {k} has no words", comp.name);
            }
            if !comp.keyboard.is_empty() || comp.states.len() > 1 {
                assert!(comp.target.is_some(), "{}: what is reached?", comp.name);
            }
            if let Some(open) = &comp.open {
                assert!(["click", "focus", "hover", "contextmenu"].contains(&open.how.as_str()), "{}", comp.name);
                assert!(comp.states.contains(&"open".to_owned()), "{}: opens, so it is shown open", comp.name);
            }
            assert_eq!(comp.matchers().len(), comp.replaces.len(), "{}: {:?}", comp.name, comp.replaces);
        }
        // The patterns that require them get the keys they need.
        let keys = |n: &str| c.get(n).unwrap().keyboard.clone();
        assert!(keys("dialog").contains(&"focus-trapped".to_owned()) && keys("dialog").contains(&"esc-closes".to_owned()));
        assert!(keys("tabs").contains(&"arrows-move".to_owned()) && keys("dropdown-menu").contains(&"arrows-move".to_owned()));
        assert!(keys("slider").contains(&"arrows-change".to_owned()));
    }

    #[test]
    fn markup_matchers_read_as_written() {
        let m = Matcher::parse("styled input[type=|text|email]").unwrap();
        let none = |_: &str| -> Option<String> { None };
        assert!(m.matches("input", &none, &[], true), "no type is a text input");
        assert!(!m.matches("input", &none, &[], false), "only a styled one");
        let checkbox = |n: &str| (n == "type").then(|| "checkbox".to_owned());
        assert!(!m.matches("input", &checkbox, &[], true));
        let nav = Matcher::parse("nav[aria-label~=breadcrumb]").unwrap();
        assert!(nav.matches("nav", &|n: &str| (n == "aria-label").then(|| "Breadcrumb".to_owned()), &[], false));
        assert!(Matcher::parse(".animate-pulse").unwrap().matches("div", &none, &["md:animate-pulse"], false));
        assert!(Matcher::parse("[aria-modal]").unwrap().matches("div", &|n: &str| (n == "aria-modal").then(String::new), &[], false));
        assert_eq!(normal("AlertDialog"), "alert-dialog");
        assert_eq!(normal("input_otp"), "input-otp");
    }
}
