//! A public component's source pulled into the worktree as a starting
//! point: a shadcn registry item (what `npx shadcn add <url>` installs),
//! from a public registry, or a 21st.dev component — whose page and its
//! markdown twin are public, and whose registry its robots.txt closes to
//! tools and which wants a key equipe never uses: what can't be fetched is
//! recorded and said, never got round.
//!
//! What comes in is recorded with where it came from and its licence, in a
//! `SOURCE.md` beside it, and listed for restyling: every colour, font and
//! picture it brings is someone else's look until it is our tokens.

use serde::Deserialize;

/// Where a component comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A 21st.dev component: `https://21st.dev/@<author>/components/<slug>`.
    TwentyFirst { author: String, slug: String },
    /// A shadcn registry item's JSON, at its address.
    Registry(String),
}

/// What a person gives — a component's page, a registry address, or the
/// `npx shadcn add <url>` line itself — as an origin. A key in the address
/// is refused: equipe pulls only what is public.
pub fn origin(input: &str) -> Result<Origin, OriginProblem> {
    let url = input.split_whitespace().find(|w| w.trim_matches(['"', '\'']).starts_with("http")).map(|w| w.trim_matches(['"', '\''])).ok_or(OriginProblem::NoAddress)?;
    if url.contains("api_key=") || url.contains("token=") {
        return Err(OriginProblem::Key);
    }
    let site = super::refs::site(url);
    if site == "21st.dev" {
        let path = url.split_once("21st.dev").map(|(_, p)| p).unwrap_or_default();
        let path = path.split(['?', '#']).next().unwrap_or_default().trim_end_matches(".md");
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let found = match parts[..] {
            [author, "components", slug, ..] => author.strip_prefix('@').map(|a| (a, slug)),
            ["r", author, slug] => Some((author, slug)),
            _ => None,
        };
        return found.map(|(author, slug)| Origin::TwentyFirst { author: author.to_owned(), slug: slug.to_owned() }).ok_or(OriginProblem::NotAComponent);
    }
    if !url.starts_with("https://") {
        return Err(OriginProblem::NotHttps);
    }
    Ok(Origin::Registry(url.to_owned()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginProblem {
    NoAddress,
    Key,
    NotAComponent,
    NotHttps,
}

impl Origin {
    /// Its public page's markdown twin (21st's), with its licence and dependencies.
    pub fn twin(&self) -> Option<String> {
        match self {
            Origin::TwentyFirst { author, slug } => Some(format!("https://21st.dev/@{author}/components/{slug}.md")),
            Origin::Registry(_) => None,
        }
    }

    /// The registry item's address.
    pub fn registry(&self) -> String {
        match self {
            Origin::TwentyFirst { author, slug } => format!("https://21st.dev/r/{author}/{slug}"),
            Origin::Registry(url) => url.clone(),
        }
    }

    /// Its page, for a person: where it was found.
    pub fn page(&self) -> String {
        match self {
            Origin::TwentyFirst { author, slug } => format!("https://21st.dev/@{author}/components/{slug}"),
            Origin::Registry(url) => url.clone(),
        }
    }

    pub fn name(&self) -> String {
        match self {
            Origin::TwentyFirst { slug, .. } => slug.clone(),
            Origin::Registry(url) => url.trim_end_matches('/').rsplit('/').next().unwrap_or("component").trim_end_matches(".json").to_owned(),
        }
    }
}

/// What a 21st component's markdown twin says of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    pub title: String,
    pub author: String,
    pub licence: Option<String>,
    pub dependencies: Vec<String>,
}

/// Read from the twin: `# Title`, `- Author: [Name](…)`, `- License: mit`,
/// `- npm dependencies: a, b`.
pub fn meta(markdown: &str) -> Meta {
    let mut m = Meta::default();
    for line in markdown.lines() {
        let line = line.trim();
        if let Some(t) = line.strip_prefix("# ").filter(|_| m.title.is_empty()) {
            m.title = t.trim().to_owned();
        } else if let Some(a) = line.strip_prefix("- Author:") {
            m.author = a.trim().trim_start_matches('[').split(']').next().unwrap_or_default().to_owned();
        } else if let Some(l) = line.strip_prefix("- License:") {
            m.licence = Some(l.trim().to_owned()).filter(|l| !l.is_empty());
        } else if let Some(d) = line.strip_prefix("- npm dependencies:") {
            m.dependencies = d.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
        }
    }
    m
}

/// A registry's answer that it wants a key (21st's `authentication_required`).
pub fn wants_key(status: u16, body: &str) -> bool {
    matches!(status, 401 | 403) || body.contains("\"authentication_required\"")
}

/// A shadcn registry item, as `npx shadcn add` reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Item {
    pub name: String,
    pub dependencies: Vec<String>,
    pub registry_dependencies: Vec<String>,
    pub files: Vec<File>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct File {
    pub path: String,
    pub content: String,
}

pub fn item(json: &str) -> Option<Item> {
    serde_json::from_str::<Item>(json).ok().filter(|i| i.files.iter().any(|f| !f.content.is_empty()))
}

/// Where a file of the item goes under its folder: its own name, never
/// outside (`..` and absolute paths are dropped to the name alone).
pub fn file_name(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or_default();
    if name.is_empty() || name == ".." || name == "." { "file.txt".to_owned() } else { name.to_owned() }
}

/// What in a file is someone else's look, a line each: colours written in
/// it, a palette's colour classes, families named, pictures from elsewhere.
pub fn restyle(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |what: String| {
        if !out.contains(&what) {
            out.push(what);
        }
    };
    const PALETTE: [&str; 22] = [
        "slate", "gray", "zinc", "neutral", "stone", "red", "orange", "amber", "yellow", "lime", "green", "emerald", "teal", "cyan", "sky", "blue", "indigo", "violet", "purple", "fuchsia", "pink", "rose",
    ];
    for (n, line) in content.lines().enumerate() {
        let at = n + 1;
        let lower = line.to_ascii_lowercase();
        let bytes = line.as_bytes();
        let mut i = 0;
        while let Some(off) = line[i..].find('#') {
            let start = i + off + 1;
            let len = bytes[start..].iter().take_while(|b| b.is_ascii_hexdigit()).count();
            if matches!(len, 3 | 4 | 6 | 8) && !bytes.get(start + len).is_some_and(|b| b.is_ascii_alphanumeric()) {
                add(format!("{at}: #{}", &line[start..start + len]));
            }
            i = start;
        }
        for f in ["rgb(", "rgba(", "hsl(", "hsla(", "oklch(", "oklab("] {
            if lower.contains(f) {
                add(format!("{at}: {}…)", f));
            }
        }
        for word in line.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '{' | '}')) {
            let class = word.rsplit(':').next().unwrap_or(word);
            let mut parts = class.splitn(3, '-');
            let (Some(prop), Some(hue), Some(step)) = (parts.next(), parts.next(), parts.next()) else { continue };
            let step = step.split('/').next().unwrap_or_default();
            if ["bg", "text", "border", "from", "via", "to", "ring", "fill", "stroke", "shadow", "outline", "decoration", "divide", "accent", "caret"].contains(&prop) && PALETTE.contains(&hue) && step.parse::<u16>().is_ok() {
                add(format!("{at}: {class}"));
            }
        }
        if lower.contains("font-family") || lower.contains("fontfamily") || lower.contains("next/font/google") {
            add(format!("{at}: {}", line.trim().chars().take(80).collect::<String>()));
        }
        for scheme in ["src=\"http", "src='http", "url(http", "url('http", "url(\"http"] {
            if lower.contains(scheme) {
                add(format!("{at}: {}", line.trim().chars().take(80).collect::<String>()));
            }
        }
    }
    out
}

/// The record beside what was pulled: where it came from, when, under what
/// licence, what it needs, and what to restyle before it is ours.
pub fn source_md(origin: &Origin, meta: &Meta, item: Option<&Item>, date: &str, restyle: &[(String, Vec<String>)]) -> String {
    let mut md = format!("# {}\n\n", if meta.title.is_empty() { origin.name() } else { meta.title.clone() });
    md.push_str(&format!("- Source: {}\n- Registry: {}\n- Pulled: {date}\n", origin.page(), origin.registry()));
    if !meta.author.is_empty() {
        md.push_str(&format!("- Author: {}\n", meta.author));
    }
    md.push_str(&format!("- Licence: {}\n", meta.licence.as_deref().unwrap_or("not stated: find it before anything ships")));
    let deps: Vec<&String> = meta.dependencies.iter().chain(item.map(|i| i.dependencies.iter()).into_iter().flatten()).collect();
    if !deps.is_empty() {
        md.push_str(&format!("- npm dependencies: {}\n", deps.iter().map(|d| d.as_str()).collect::<Vec<_>>().join(", ")));
    }
    if let Some(i) = item.filter(|i| !i.registry_dependencies.is_empty()) {
        md.push_str(&format!("- Registry dependencies: {}\n", i.registry_dependencies.join(", ")));
    }
    md.push_str("\nA starting point, not ours yet: take its structure and behaviour; restyle every colour, font, radius and picture to our tokens, keep the licence's notice where it asks for one, and never ship another brand's look or words.\n");
    if !restyle.is_empty() {
        md.push_str("\n## To restyle\n\n");
        for (file, lines) in restyle {
            for line in lines {
                md.push_str(&format!("- [ ] {file}:{line}\n"));
            }
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_component_is_found_from_its_page_its_registry_or_the_install_line() {
        let want = Origin::TwentyFirst { author: "serafimcloud".into(), slug: "gradient-button".into() };
        assert_eq!(origin("https://21st.dev/@serafimcloud/components/gradient-button"), Ok(want.clone()));
        assert_eq!(origin("https://21st.dev/@serafimcloud/components/gradient-button.md"), Ok(want.clone()));
        assert_eq!(origin("npx shadcn@latest add \"https://21st.dev/r/serafimcloud/gradient-button\""), Ok(want.clone()));
        assert_eq!(origin("npx shadcn@latest add \"https://21st.dev/r/serafimcloud/gradient-button?api_key=$API_KEY_21ST\""), Err(OriginProblem::Key), "never with a key");
        assert_eq!(origin("https://21st.dev/community/components/s/hero"), Err(OriginProblem::NotAComponent));
        assert_eq!(origin("http://example.com/r/button.json"), Err(OriginProblem::NotHttps));
        assert_eq!(origin("add a button"), Err(OriginProblem::NoAddress));
        let shadcn = origin("npx shadcn@latest add https://ui.shadcn.com/r/styles/new-york-v4/button.json").unwrap();
        assert_eq!(shadcn.name(), "button");
        assert_eq!(shadcn.twin(), None);
        assert_eq!(want.twin().unwrap(), "https://21st.dev/@serafimcloud/components/gradient-button.md");
        assert_eq!(want.registry(), "https://21st.dev/r/serafimcloud/gradient-button");
    }

    #[test]
    fn the_twin_gives_licence_and_dependencies_and_a_key_wall_is_recognised() {
        let md = "# Gradient Button\n\n> A button.\n\n- Author: [Serafim](https://21st.dev/@serafimcloud)\n- License: mit\n- Tags: Button\n- npm dependencies: @radix-ui/react-slot, class-variance-authority\n";
        let m = meta(md);
        assert_eq!((m.title.as_str(), m.author.as_str(), m.licence.as_deref()), ("Gradient Button", "Serafim", Some("mit")));
        assert_eq!(m.dependencies, ["@radix-ui/react-slot", "class-variance-authority"]);
        assert!(wants_key(403, "{\"error\":\"Authentication required\",\"reason\":\"authentication_required\"}"));
        assert!(!wants_key(200, "{\"name\":\"button\"}"));
    }

    #[test]
    fn a_registry_item_lands_by_file_name_with_what_to_restyle() {
        let json = r##"{"name":"hero","dependencies":["motion"],"registryDependencies":["button"],"files":[{"path":"registry/ui/hero.tsx","content":"<div className=\"bg-indigo-500 text-white hover:bg-indigo-600/90\" style={{color:'#1D4ED8'}}>\n<img src=\"https://cdn.example.com/a.png\"/>\n<p style={{fontFamily:'Geist'}}>x</p> rgb(0 0 0)\n</div>"},{"path":"../../etc/passwd","content":"x"}]}"##;
        let i = item(json).unwrap();
        assert_eq!(i.files.len(), 2);
        assert_eq!(file_name(&i.files[0].path), "hero.tsx");
        assert_eq!(file_name(&i.files[1].path), "passwd", "never outside its folder");
        assert_eq!(file_name("a/.."), "file.txt");
        let todo = restyle(&i.files[0].content);
        assert!(todo.contains(&"1: bg-indigo-500".to_owned()) && todo.contains(&"1: bg-indigo-600/90".to_owned()), "{todo:?}");
        assert!(todo.contains(&"1: #1D4ED8".to_owned()));
        assert!(todo.iter().any(|t| t.starts_with("2: ") && t.contains("cdn.example.com")));
        assert!(todo.iter().any(|t| t.starts_with("3: ") && t.contains("fontFamily")) && todo.iter().any(|t| t == "3: rgb(…)"));
        assert!(!todo.iter().any(|t| t.contains("text-white")), "white isn't a palette's hue step");
        assert!(restyle("const id = '#main-content'; // #fff is fine in a comment? no:").iter().any(|t| t.ends_with("#fff")));
        assert!(restyle("<a href=\"#section-2\">").is_empty(), "an anchor isn't a colour");
        assert!(item("{\"name\":\"empty\",\"files\":[]}").is_none());

        let origin = Origin::Registry("https://ui.shadcn.com/r/styles/new-york-v4/hero.json".into());
        let md = source_md(&origin, &Meta::default(), Some(&i), "2026-11-07", &[("hero.tsx".into(), todo)]);
        assert!(md.contains("- Source: https://ui.shadcn.com/r/styles/new-york-v4/hero.json") && md.contains("- Pulled: 2026-11-07"));
        assert!(md.contains("- Licence: not stated") && md.contains("- npm dependencies: motion") && md.contains("- Registry dependencies: button"));
        assert!(md.contains("never ship another brand's look") && md.contains("- [ ] hero.tsx:1: bg-indigo-500"));
    }
}
