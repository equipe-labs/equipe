//! A redesign keeps what the screen does. Before it, `equipe design kept
//! record` writes down the screen's links, fields, actions and headings
//! (`design/kept/<screen>.json`); after it, `kept check` says what went
//! missing — failing unless the brief or the decision log says to remove it
//! (`Remove action: Export CSV`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::lint::source::{Node, Source};

/// What a screen offers a person.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Inventory {
    /// Where its links go.
    pub links: Vec<String>,
    /// Its fields, by name or id.
    pub fields: Vec<String>,
    /// Its buttons' words and the handlers they call.
    pub actions: Vec<String>,
    pub headings: Vec<String>,
}

/// The kinds, as a brief names them.
pub const KINDS: [&str; 4] = ["link", "field", "action", "heading"];

fn norm(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn push(list: &mut Vec<String>, item: &str) {
    let item = norm(item);
    if !item.is_empty() && !list.contains(&item) {
        list.push(item);
    }
}

/// The function a handler calls: `save`, `() => save(id)` → `save`.
fn handler(value: &str) -> Option<String> {
    let v = value.trim();
    let body = v.rsplit("=>").next().unwrap_or(v).trim().trim_start_matches('{').trim();
    let name: String = body.chars().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '$')).collect();
    (!name.is_empty() && name.chars().next().is_some_and(|c| !c.is_ascii_digit())).then_some(name)
}

impl Inventory {
    /// From a source as written.
    pub fn of_source(src: &Source) -> Inventory {
        let mut inv = Inventory::default();
        // The open element whose words are being gathered: a heading or a button.
        let mut gathering: Vec<(String, String)> = Vec::new();
        for node in &src.nodes {
            match node {
                Node::Open(tag) => {
                    if let Some(href) = tag.attr("href").or_else(|| if tag.name == "Link" || tag.name == "router-link" || tag.name == "NuxtLink" { tag.attr("to") } else { None })
                        && !href.is_empty()
                        && href != "#"
                    {
                        push(&mut inv.links, href);
                    }
                    if matches!(tag.name.as_str(), "input" | "select" | "textarea") && tag.attr("type").is_none_or(|t| t != "hidden") {
                        if let Some(name) = tag.attr("name").or_else(|| tag.attr("id")) {
                            push(&mut inv.fields, name);
                        }
                    }
                    for (attr, value) in &tag.attrs {
                        let is_handler = matches!(attr.as_str(), "onClick" | "onclick" | "@click" | "v-on:click" | "on:click" | "(click)" | "onSubmit" | "onsubmit" | "@submit" | "@submit.prevent" | "on:submit" | "(submit)" | "(ngSubmit)" | "action" | "formAction" | "hx-post" | "hx-get");
                        if is_handler && let Some(v) = value {
                            if attr == "action" || attr.starts_with("hx-") {
                                push(&mut inv.actions, &format!("{} {v}", tag.attr("method").unwrap_or(if attr.starts_with("hx-") { &attr[3..] } else { "submit" })));
                            } else if let Some(h) = handler(v) {
                                push(&mut inv.actions, &format!("{h}()"));
                            }
                        }
                    }
                    if tag.is_heading().is_some() || tag.name == "button" || (tag.name == "input" && tag.attr("type").is_some_and(|t| t == "submit")) {
                        if tag.name == "input" {
                            push(&mut inv.actions, tag.attr("value").unwrap_or("submit"));
                        } else if !tag.self_closing {
                            gathering.push((tag.name.clone(), String::new()));
                        }
                    }
                }
                Node::Text { text, .. } => {
                    for (_, words) in gathering.iter_mut() {
                        words.push(' ');
                        words.push_str(text);
                    }
                }
                Node::Close { name, .. } => {
                    if let Some(at) = gathering.iter().rposition(|(n, _)| n == name) {
                        let (n, words) = gathering.remove(at);
                        if n == "button" {
                            push(&mut inv.actions, &words);
                        } else {
                            push(&mut inv.headings, &words);
                        }
                    }
                }
                Node::Comment { .. } => {}
            }
        }
        inv
    }

    fn lists(&self) -> [(&'static str, &Vec<String>); 4] {
        [("link", &self.links), ("field", &self.fields), ("action", &self.actions), ("heading", &self.headings)]
    }

    pub fn is_empty(&self) -> bool {
        self.lists().iter().all(|(_, l)| l.is_empty())
    }

    pub fn merge(&mut self, other: Inventory) {
        for item in other.links {
            push(&mut self.links, &item);
        }
        for item in other.fields {
            push(&mut self.fields, &item);
        }
        for item in other.actions {
            push(&mut self.actions, &item);
        }
        for item in other.headings {
            push(&mut self.headings, &item);
        }
    }
}

/// A removal the brief allows: its kind and the words it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub kind: String,
    pub item: String,
}

/// `Remove <kind>: <item>` lines, in any text: a brief, a decision.
pub fn removals(text: &str) -> Vec<Removal> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim().trim_start_matches(['-', '*', ' ']).trim();
        let lower = t.to_lowercase();
        let Some(rest) = lower.strip_prefix("remove ") else { continue };
        let Some((kind, item)) = rest.split_once(':') else { continue };
        let kind = kind.trim().trim_end_matches('s');
        if KINDS.contains(&kind) && !item.trim().is_empty() {
            out.push(Removal { kind: kind.to_owned(), item: norm(item.trim().trim_matches('`')) });
        }
    }
    out
}

/// What went missing between `before` and `after`, a `(kind, item)` each,
/// less what may go.
pub fn missing(before: &Inventory, after: &Inventory, allowed: &[Removal]) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for ((kind, was), (_, now)) in before.lists().into_iter().zip(after.lists()) {
        for item in was {
            if now.contains(item) || allowed.iter().any(|r| r.kind == kind && (item.contains(&r.item) || r.item.contains(item.as_str()))) {
                continue;
            }
            out.push((kind, item.clone()));
        }
    }
    out
}

pub fn path(design: &Path, screen: &str) -> PathBuf {
    design.join("kept").join(format!("{}.json", super::lint::record::safe(screen)))
}

/// Every removal the design's decisions allow.
pub fn allowed_by_decisions(design: &Path) -> Vec<Removal> {
    let Ok(entries) = std::fs::read_dir(design.join("decisions")) else { return Vec::new() };
    entries.filter_map(|e| e.ok()).filter_map(|e| std::fs::read_to_string(e.path()).ok()).flat_map(|t| removals(&t)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inv(name: &str, text: &str) -> Inventory {
        Inventory::of_source(&Source::read(Path::new(name), text.to_owned()).unwrap())
    }

    #[test]
    fn a_screen_lists_its_links_fields_actions_and_headings() {
        let i = inv(
            "a.tsx",
            "export const A = () => (\n  <main>\n    <h1>Billing</h1>\n    <Link to=\"/plans\">Plans</Link><a href=\"#\">top</a>\n    <form onSubmit={save}>\n      <input name=\"card\" /><input type=\"hidden\" name=\"csrf\" />\n      <button onClick={() => exportCsv(id)}>Export CSV</button>\n      <button type=\"submit\">Save</button>\n    </form>\n  </main>\n);",
        );
        assert_eq!(i.links, vec!["/plans"]);
        assert_eq!(i.fields, vec!["card"]);
        assert_eq!(i.actions, vec!["save()", "exportcsv()", "export csv", "save"]);
        assert_eq!(i.headings, vec!["billing"]);
    }

    #[test]
    fn what_a_redesign_dropped_is_missing_unless_the_brief_says_to_remove_it() {
        let before = inv("a.html", "<h1>Billing</h1><h2>Invoices</h2><a href=\"/help\">Help</a><input name=\"card\"><button>Export CSV</button><button>Save</button><form action=\"/pay\" method=\"post\"></form>");
        let after = inv("b.html", "<h1>Billing</h1><a href=\"/help\">Help</a><input name=\"card\"><button>Save</button>");
        let gone = missing(&before, &after, &[]);
        assert_eq!(gone, vec![("action", "export csv".to_owned()), ("action", "post /pay".to_owned()), ("heading", "invoices".to_owned())]);
        let brief = "## Brief\n- Remove action: Export CSV (moves to Reports)\nRemove headings: Invoices\nremove nothing: x";
        let allowed = removals(brief);
        assert_eq!(allowed.len(), 2);
        assert_eq!(missing(&before, &after, &allowed), vec![("action", "post /pay".to_owned())]);
        assert!(missing(&before, &before, &[]).is_empty());
    }
}
