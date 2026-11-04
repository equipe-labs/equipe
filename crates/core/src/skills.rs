//! Skills: what a member needs only for some kinds of work, kept out of the
//! charter it reads on every turn (progressive disclosure).
//!
//! A skill is a folder under `crates/core/skills/<name>/`: a `SKILL.md` whose
//! front matter gives its `name` and a one-line `description` of when to use
//! it, and optional `references/` the body points to. The charter carries only
//! the index — each skill's name and when to open it; the body is read when the
//! work matches, a reference only when the body says so.
//!
//! Each skill is offered to the roles (and squads) whose charter it came out
//! of, rendered for them: `{{tools_at}}` and `{{hq_in}}` filled in, and
//! `{{#if <flag>}}` … `{{/if}}` lines kept only where the flag holds. How a
//! member's engine gets them:
//! - Claude Code: as a plugin (`--plugin-dir`), written once per content under
//!   `~/.equipe/skills/<version>/`, so its prompt stays the same turn to turn;
//! - an engine that reads files (Codex's CLI): the index with each `SKILL.md`'s
//!   absolute path;
//! - an engine that reads nothing (Codex over the API): every body, inline.

use std::path::{Path, PathBuf};

use crate::roles::{Context, PM, Role};

/// The plugin's name: Claude Code lists its skills as `equipe:<name>`.
pub const PLUGIN: &str = "equipe";

/// How a member's engine gets its skills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery<'a> {
    /// Every skill's text in the charter itself: for an engine that can't read a file.
    Inline,
    /// A Claude Code plugin in this folder (`--plugin-dir`).
    Plugin(&'a str),
    /// Files in this folder, each `SKILL.md` read by its absolute path.
    Files(&'a str),
}

/// One skill's source files, as kept in `crates/core/skills/<dir>/`.
struct Source {
    dir: &'static str,
    /// `SKILL.md` first, then its references, by path inside the folder.
    files: &'static [(&'static str, &'static str)],
    /// Whose charter it came out of: the roles, and the squads, that get it.
    offered: fn(&Role, &Context<'_>) -> bool,
}

macro_rules! file {
    ($dir:literal, $path:literal) => {
        ($path, include_str!(concat!("../skills/", $dir, "/", $path)))
    };
}

fn builder(role: &Role) -> bool {
    matches!(role.id, "frontend" | "backend" | "mobile")
}

const SOURCES: &[Source] = &[
    Source { dir: "office-hours", files: &[file!("office-hours", "SKILL.md")], offered: |r, c| r.id == PM && c.board_tools },
    Source { dir: "planning", files: &[file!("planning", "SKILL.md")], offered: |r, _| r.id == PM },
    Source { dir: "working-with-design", files: &[file!("working-with-design", "SKILL.md")], offered: |r, _| r.id == PM },
    Source { dir: "routines-and-signals", files: &[file!("routines-and-signals", "SKILL.md")], offered: |r, _| r.id == PM },
    Source { dir: "product-ideas", files: &[file!("product-ideas", "SKILL.md")], offered: |r, c| r.id == PM && c.board_tools },
    Source {
        dir: "plan-review",
        files: &[file!("plan-review", "SKILL.md")],
        offered: |r, c| c.board_tools && (builder(r) || matches!(r.id, "designer" | "qa")),
    },
    Source { dir: "research-report", files: &[file!("research-report", "SKILL.md")], offered: |_, _| true },
    Source { dir: "browser", files: &[file!("browser", "SKILL.md")], offered: |_, c| c.browser },
    Source { dir: "desktop-apps", files: &[file!("desktop-apps", "SKILL.md")], offered: |_, c| c.desktop },
    Source { dir: "design-system", files: &[file!("design-system", "SKILL.md")], offered: |r, _| r.id == "designer" },
    Source { dir: "design-process", files: &[file!("design-process", "SKILL.md")], offered: |r, _| r.id == "designer" },
    Source { dir: "design-tools", files: &[file!("design-tools", "SKILL.md")], offered: |r, _| r.id == "designer" },
    Source {
        dir: "ui-map",
        files: &[file!("ui-map", "SKILL.md"), file!("ui-map", "references/other-apps.md"), file!("ui-map", "references/walkthrough.md")],
        offered: |r, _| r.id == "designer",
    },
    Source { dir: "design-qa", files: &[file!("design-qa", "SKILL.md")], offered: |r, _| r.id == "designer" },
    Source { dir: "design-remix", files: &[file!("design-remix", "SKILL.md")], offered: |r, c| r.id == "designer" && c.design },
    Source { dir: "security-review", files: &[file!("security-review", "SKILL.md")], offered: |r, _| r.id == crate::roles::SECURITY },
    Source { dir: "marketing-playbook", files: &[file!("marketing-playbook", "SKILL.md")], offered: |r, _| r.id == crate::roles::MARKETING },
];

/// A skill as one member gets it: its files rendered for its role and squad.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    /// When to use it, in a line: what the index shows.
    pub description: String,
    /// `SKILL.md` first (front matter and all), then its references, by path
    /// inside the skill's folder.
    pub files: Vec<(String, String)>,
}

impl Skill {
    /// The `SKILL.md` without its front matter.
    pub fn body(&self) -> &str {
        front_matter(&self.files[0].1).map(|(_, _, body)| body).unwrap_or(&self.files[0].1)
    }

    /// Everything it says: its body, then each reference.
    pub fn text(&self) -> String {
        std::iter::once(self.body().trim()).chain(self.files[1..].iter().map(|(_, text)| text.trim())).collect::<Vec<_>>().join("\n\n")
    }
}

/// `---\nname: …\ndescription: …\n---\n<body>`: the name, the description and the body.
pub fn front_matter(text: &str) -> Option<(&str, &str, &str)> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    let (head, body) = (&rest[..end], &rest[end + "\n---\n".len()..]);
    let field = |key: &str| head.lines().find_map(|line| line.strip_prefix(key)?.strip_prefix(": ")).map(str::trim);
    Some((field("name")?, field("description")?, body))
}

/// The skills a member of `role` is offered in this squad, rendered for it.
pub fn offered(role: &Role, ctx: &Context<'_>) -> Vec<Skill> {
    SOURCES
        .iter()
        .filter(|s| (s.offered)(role, ctx))
        .map(|s| {
            let files: Vec<(String, String)> = s.files.iter().map(|(path, text)| ((*path).to_owned(), render(text, role, ctx))).collect();
            let (name, description) =
                front_matter(&files[0].1).map_or_else(|| (s.dir.to_owned(), String::new()), |(name, description, _)| (name.to_owned(), description.to_owned()));
            Skill { name, description, files }
        })
        .collect()
}

/// A template filled in for one member: its placeholders, and its
/// `{{#if <flag>}}` blocks kept only where the flag holds.
fn render(template: &str, role: &Role, ctx: &Context<'_>) -> String {
    let holds = |flag: &str| match flag {
        "board_tools" => ctx.board_tools,
        "browser" => ctx.browser,
        "desktop" => ctx.desktop,
        "design" => ctx.design,
        "builder" => builder(role),
        flag => flag.strip_prefix("role:").is_some_and(|id| id == role.id),
    };
    let mut out = String::with_capacity(template.len());
    let mut keep = true;
    for line in template.split_inclusive('\n') {
        let directive = line.trim();
        if let Some(flag) = directive.strip_prefix("{{#if ").and_then(|d| d.strip_suffix("}}")) {
            keep = holds(flag.trim());
        } else if directive == "{{/if}}" {
            keep = true;
        } else if keep {
            out.push_str(line);
        }
    }
    let tools_at = ctx.equipe.map(|path| format!(" (`{path}` if `equipe` isn't on your PATH)")).unwrap_or_default();
    let hq_in = ctx.hq.map(|hq| format!(" — in {hq}")).unwrap_or_default();
    out.replace("{{tools_at}}", &tools_at).replace("{{hq_in}}", &hq_in)
}

/// What the charter says of the skills: their index, or with an engine that
/// can't open a file, all of them.
pub fn for_charter(skills: &[Skill], delivery: Delivery<'_>) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let item = |s: &Skill, path: Option<String>| match path {
        Some(path) => format!("\n- `{}` (`{path}`): {}", s.name, s.description),
        None => format!("\n- `{}`: {}", s.name, s.description),
    };
    match delivery {
        Delivery::Inline => skills.iter().map(|s| format!("\n\nYour `{}` skill. {}\n\n{}", s.name, s.description, s.text())).collect(),
        Delivery::Plugin(dir) => {
            let list: String = skills.iter().map(|s| item(s, None)).collect();
            format!(
                "\n\nYour skills hold what only some work needs — part of this charter all the same. Before you do \
that kind of work, open the matching skill with your Skill tool (`{PLUGIN}:<name>`; if it can't, Read \
`{dir}/skills/<name>/SKILL.md`) and follow it; open a file it points to when it says so:{list}"
            )
        }
        Delivery::Files(dir) => {
            let list: String = skills.iter().map(|s| item(s, Some(skill_md(Path::new(dir), &s.name).to_string_lossy().into_owned()))).collect();
            format!(
                "\n\nYour skills hold what only some work needs — part of this charter all the same. Before you do \
that kind of work, read the matching skill's `SKILL.md` at its path and follow it; open a file it points to, beside \
it, when it says so:{list}"
            )
        }
    }
}

/// Where a skill's `SKILL.md` is in a folder of them.
pub fn skill_md(dir: &Path, name: &str) -> PathBuf {
    dir.join("skills").join(name).join("SKILL.md")
}

/// The roles whose edits to UI files are checked as they are made: the floor
/// of `equipe design lint`, handed back in the same turn.
pub fn checks_edits(role: &str) -> bool {
    matches!(role, "designer" | "frontend")
}

/// The plugin's `hooks/hooks.json` for a member of `role` on Claude Code:
/// after each edit to a file, `equipe design lint --hook` reads the floor in
/// what was just written and hands failures back (a designer's edits are
/// also written down, to be checked whole before its hand-in). `None` for a
/// role whose edits aren't checked.
pub fn hooks(role: &str, equipe: &Path) -> Option<String> {
    if !checks_edits(role) {
        return None;
    }
    let mut command = format!("'{}' design lint --hook", equipe.to_string_lossy().replace('\'', "'\\''"));
    if role == "designer" {
        command.push_str(" --ledger");
    }
    let hooks = serde_json::json!({
        "hooks": {
            "PostToolUse": [{
                "matcher": "Edit|MultiEdit|Write",
                "hooks": [{ "type": "command", "command": command, "timeout": 20 }],
            }],
        },
    });
    Some(format!("{hooks:#}\n"))
}

/// The folder a member's skills go in: one per version and content, so a
/// member's engine sees the same files — and the same prompt — turn after turn.
pub fn folder(home: &Path, version: &str, role: &str, skills: &[Skill]) -> PathBuf {
    folder_with(home, version, role, skills, None)
}

fn folder_with(home: &Path, version: &str, role: &str, skills: &[Skill], hooks: Option<&str>) -> PathBuf {
    let mut hash = Fnv::default();
    hash.add(version);
    if let Some(hooks) = hooks {
        hash.add(hooks);
    }
    for skill in skills {
        hash.add(&skill.name);
        for (path, text) in &skill.files {
            hash.add(path);
            hash.add(text);
        }
    }
    let role: String = role.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    home.join("skills").join(version).join(format!("{role}-{:016x}", hash.0))
}

/// Write a member's skills as a Claude Code plugin — `.claude-plugin/plugin.json`,
/// `skills/<name>/…` and, given, its `hooks/hooks.json` ([`hooks`]) — unless
/// they are there already. Returns the folder.
pub fn materialise(home: &Path, version: &str, role: &str, skills: &[Skill], hooks: Option<&str>) -> std::io::Result<PathBuf> {
    let dir = folder_with(home, version, role, skills, hooks);
    let manifest = dir.join(".claude-plugin").join("plugin.json");
    if manifest.is_file() {
        return Ok(dir);
    }
    // Written beside it, then moved into place: an engine never sees half of one.
    let draft = dir.with_extension(format!("draft-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&draft);
    let written = (|| {
        std::fs::create_dir_all(draft.join(".claude-plugin"))?;
        let plugin = serde_json::json!({
            "name": PLUGIN,
            "version": version,
            "description": "What an equipe squad member needs only for some kinds of work.",
        });
        std::fs::write(draft.join(".claude-plugin").join("plugin.json"), format!("{plugin:#}\n"))?;
        if let Some(hooks) = hooks {
            std::fs::create_dir_all(draft.join("hooks"))?;
            std::fs::write(draft.join("hooks").join("hooks.json"), hooks)?;
        }
        for skill in skills {
            for (path, text) in &skill.files {
                let file = draft.join("skills").join(&skill.name).join(path);
                if let Some(parent) = file.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(file, text)?;
            }
        }
        std::fs::rename(&draft, &dir)
    })();
    match written {
        Ok(()) => Ok(dir),
        // Another turn wrote the same at the same time: theirs is as good.
        Err(_) if manifest.is_file() => {
            let _ = std::fs::remove_dir_all(&draft);
            Ok(dir)
        }
        Err(err) => {
            let _ = std::fs::remove_dir_all(&draft);
            Err(err)
        }
    }
}

/// What Claude Code lets a member do with its skills without asking: open
/// each one, and read the files in their folder.
pub fn allowed_tools(dir: &Path, skills: &[Skill]) -> Vec<String> {
    skills
        .iter()
        .map(|s| format!("Skill({PLUGIN}:{})", s.name))
        .chain(std::iter::once(format!("Read(/{}/**)", dir.display())))
        .collect()
}

/// FNV-1a, 64 bits: the same name for the same content on every build.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn add(&mut self, text: &str) {
        for byte in text.bytes().chain([0xff]) {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roles::{CUSTOM, ROLES, Teammate, role, role_or_custom};

    fn ctx<'a>(team: &'a [Teammate<'a>]) -> Context<'a> {
        Context {
            team,
            projects: &[],
            hq: None,
            equipe: None,
            board_tools: true,
            language: None,
            screen_language: None,
            browser: false,
            profiles: &[],
            pm_accepts: false,
            desktop: false,
            design: false,
            home: None,
            memory: None,
            facts: 0,
            edits: false,
            skills: Delivery::Inline,
        }
    }

    fn names(role_id: &str, ctx: &Context<'_>) -> Vec<String> {
        offered(role_or_custom(role_id), ctx).into_iter().map(|s| s.name).collect()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-skills-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn every_skill_folder_in_the_source_is_built_in_whole() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("skills");
        let mut on_disk: Vec<String> = Vec::new();
        fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, root, out);
                } else {
                    out.push(path.strip_prefix(root).unwrap().to_string_lossy().into_owned());
                }
            }
        }
        walk(&root, &root, &mut on_disk);
        on_disk.sort();
        let mut built: Vec<String> = SOURCES.iter().flat_map(|s| s.files.iter().map(move |(path, _)| format!("{}/{path}", s.dir))).collect();
        built.sort();
        assert_eq!(on_disk, built, "a file under skills/ that no skill carries, or one it names that isn't there");
    }

    #[test]
    fn each_skill_has_valid_front_matter_a_short_when_and_a_short_body() {
        let mut seen = Vec::new();
        for source in SOURCES {
            let (path, text) = source.files[0];
            assert_eq!(path, "SKILL.md", "{}", source.dir);
            let (name, description, body) = front_matter(text).unwrap_or_else(|| panic!("{}: no front matter", source.dir));
            assert_eq!(name, source.dir, "a skill's name is its folder's");
            assert!(name.len() <= 64 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'), "{name}");
            assert!(!seen.contains(&name), "{name} twice");
            seen.push(name);
            // One plain YAML line, saying when.
            assert!(description.starts_with("Use "), "{name}: {description}");
            assert!(description.chars().count() <= 220, "{name}: {} chars", description.chars().count());
            assert!(!description.contains(": ") && !description.contains(" #") && !description.contains('"'), "{name}: plain YAML: {description}");
            let head = &text[4..text.find("\n---\n").unwrap()];
            assert_eq!(head.lines().count(), 2, "{name}: only its name and description");
            assert!(text.lines().count() <= 500, "{name}: {} lines", text.lines().count());
            assert!(body.trim_start().starts_with("# "), "{name}: a title");
            // A reference the body names is there, and each one is named.
            for (reference, _) in &source.files[1..] {
                assert!(body.contains(&format!("`{reference}`")), "{name}: {reference} is never pointed to");
            }
            for word in body.split('`').filter(|w| w.starts_with("references/")) {
                assert!(source.files.iter().any(|(p, _)| *p == word), "{name}: {word} missing");
            }
            // Agents read English.
            for (path, text) in source.files {
                assert!(!text.chars().any(|c| ('\u{ac00}'..='\u{d7a3}').contains(&c)), "{name}/{path}: Hangul");
            }
        }
    }

    #[test]
    fn a_skill_is_filled_in_for_its_member_and_keeps_only_what_holds_for_it() {
        let team = [Teammate { name: "pm", role: PM }];
        let full = Context { equipe: Some("/opt/equipe"), hq: Some("ops"), browser: true, desktop: true, design: true, ..ctx(&team) };
        for r in ROLES.iter().chain([&CUSTOM]) {
            for c in [&full, &ctx(&team), &Context { board_tools: false, ..ctx(&team) }] {
                for skill in offered(r, c) {
                    for (path, text) in &skill.files {
                        assert!(!text.contains("{{"), "{}: {}/{path}: {text}", r.id, skill.name);
                    }
                }
            }
        }
        let text = |id: &str, c: &Context<'_>, name: &str| offered(role(id).unwrap(), c).into_iter().find(|s| s.name == name).unwrap().text();
        assert!(text(PM, &full, "office-hours").contains("`equipe brief critique <its path>` (`/opt/equipe` if `equipe` isn't on your PATH)"));
        assert!(text(PM, &ctx(&team), "office-hours").contains("`equipe brief critique <its path>`; fold in"));
        assert!(text(PM, &full, "routines-and-signals").contains("found again — in ops."));
        assert!(text(PM, &ctx(&team), "routines-and-signals").contains("found again. A message"));
        // A role's own part, and no one else's.
        let qa = text("qa", &full, "plan-review");
        assert!(qa.contains("say whether it can be tested") && !qa.contains("score each design dimension") && !qa.contains("as the one who will build it"));
        assert!(text("mobile", &full, "plan-review").contains("as the one who will build it"));
        assert!(text("designer", &full, "design-tools").contains("`data:` URL"));
        assert!(!text("designer", &ctx(&team), "design-tools").contains("`data:` URL"), "no browser, no render advice");
    }

    #[test]
    fn each_role_is_offered_the_skills_its_charter_came_with() {
        let team = [Teammate { name: "pm", role: PM }];
        let base = ctx(&team);
        let all = Context { browser: true, desktop: true, design: true, ..ctx(&team) };
        assert_eq!(names(PM, &base), ["office-hours", "planning", "working-with-design", "routines-and-signals", "product-ideas", "research-report"]);
        assert_eq!(names("designer", &base), ["plan-review", "research-report", "design-system", "design-process", "design-tools", "ui-map", "design-qa"]);
        assert!(names("designer", &all).contains(&"design-remix".to_owned()));
        for id in ["frontend", "backend", "mobile", "qa"] {
            assert_eq!(names(id, &base), ["plan-review", "research-report"], "{id}");
        }
        assert_eq!(names("data", &base), ["research-report"]);
        assert_eq!(names(crate::roles::SECURITY, &base), ["research-report", "security-review"]);
        assert_eq!(names(crate::roles::MARKETING, &base), ["research-report", "marketing-playbook"]);
        assert_eq!(names(CUSTOM.id, &all), ["research-report", "browser", "desktop-apps"]);
        // Without board tools: nothing that needs them.
        let bare = Context { board_tools: false, ..ctx(&team) };
        assert_eq!(names(PM, &bare), ["planning", "working-with-design", "routines-and-signals", "research-report"]);
        assert!(!names("backend", &bare).contains(&"plan-review".to_owned()));
    }

    #[test]
    fn skills_are_written_once_as_a_claude_code_plugin_and_again_only_when_they_change() {
        let home = scratch("plugin");
        let team = [Teammate { name: "pm", role: PM }];
        let skills = offered(role(PM).unwrap(), &ctx(&team));
        let dir = materialise(&home, "1.2.3", PM, &skills, None).unwrap();
        assert!(dir.starts_with(home.join("skills").join("1.2.3")), "{}", dir.display());
        let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(".claude-plugin/plugin.json")).unwrap()).unwrap();
        assert_eq!(manifest["name"], PLUGIN);
        for skill in &skills {
            assert_eq!(std::fs::read_to_string(skill_md(&dir, &skill.name)).unwrap(), skill.files[0].1);
        }
        // The same skills: the same folder, left as it was.
        let marker = dir.join("skills/planning/SKILL.md");
        std::fs::write(&marker, "kept").unwrap();
        assert_eq!(materialise(&home, "1.2.3", PM, &skills, None).unwrap(), dir);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "kept", "not written again");
        // Other content, another version: another folder.
        let other = offered(role(PM).unwrap(), &Context { hq: Some("ops"), ..ctx(&team) });
        assert_ne!(materialise(&home, "1.2.3", PM, &other, None).unwrap(), dir);
        assert_ne!(folder(&home, "1.2.4", PM, &skills), dir);
        assert!(!std::fs::read_dir(home.join("skills/1.2.3")).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("draft")), "no draft left");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_designers_and_a_frontend_engineers_plugin_checks_their_edits_as_they_make_them() {
        let equipe = Path::new("/Applications/equipe.app/Contents/MacOS/equipe");
        let designer: serde_json::Value = serde_json::from_str(&hooks("designer", equipe).unwrap()).unwrap();
        let hook = &designer["hooks"]["PostToolUse"][0];
        assert_eq!(hook["matcher"], "Edit|MultiEdit|Write");
        assert_eq!(hook["hooks"][0]["command"], "'/Applications/equipe.app/Contents/MacOS/equipe' design lint --hook --ledger");
        let frontend: serde_json::Value = serde_json::from_str(&hooks("frontend", equipe).unwrap()).unwrap();
        assert!(frontend["hooks"]["PostToolUse"][0]["hooks"][0]["command"].as_str().unwrap().ends_with("design lint --hook"), "an engineer's edits aren't written down for a hand-in");
        for role in [PM, "backend", "qa"] {
            assert!(hooks(role, equipe).is_none(), "{role}");
        }
        let home = scratch("hooks");
        let skills = offered(crate::roles::role("designer").unwrap(), &ctx(&[]));
        let with = hooks("designer", equipe);
        let dir = materialise(&home, "1.2.3", "designer", &skills, with.as_deref()).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("hooks/hooks.json")).unwrap(), with.unwrap());
        let without = materialise(&home, "1.2.3", "designer", &skills, None).unwrap();
        assert_ne!(dir, without, "a plugin with hooks is another folder");
        assert!(!without.join("hooks").exists());
    }

    #[test]
    fn the_index_names_each_skill_and_when_and_how_to_open_it() {
        let home = scratch("index");
        let team = [Teammate { name: "pm", role: PM }];
        let skills = offered(role(PM).unwrap(), &ctx(&team));
        let dir = materialise(&home, "1.2.3", PM, &skills, None).unwrap();
        let at = dir.to_string_lossy().into_owned();
        let files = for_charter(&skills, Delivery::Files(&at));
        let plugin = for_charter(&skills, Delivery::Plugin(&at));
        for skill in &skills {
            let path = skill_md(&dir, &skill.name);
            assert!(path.is_absolute() && path.is_file());
            assert!(files.contains(&format!("- `{}` (`{}`): {}", skill.name, path.display(), skill.description)), "{files}");
            assert!(plugin.contains(&format!("- `{}`: {}", skill.name, skill.description)), "{plugin}");
            assert!(!files.contains(skill.body().trim()) && !plugin.contains(skill.body().trim()), "only the index");
        }
        assert!(files.contains("read the matching skill's `SKILL.md` at its path"), "{files}");
        assert!(plugin.contains("with your Skill tool (`equipe:<name>`") && plugin.contains(&format!("Read `{at}/skills/<name>/SKILL.md`")), "{plugin}");
        // An engine that can't open a file gets every word.
        let inline = for_charter(&skills, Delivery::Inline);
        assert!(skills.iter().all(|s| inline.contains(&s.text())));
        assert_eq!(for_charter(&[], Delivery::Plugin(&at)), "");
        let rules = allowed_tools(&dir, &skills);
        assert!(rules.contains(&"Skill(equipe:office-hours)".to_owned()), "{rules:?}");
        assert!(rules.contains(&format!("Read(/{}/**)", dir.display())), "{rules:?}");
        let _ = std::fs::remove_dir_all(&home);
    }
}
