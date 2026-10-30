//! The screens' words in the person's language.
//!
//! Each front end keeps its words in `locales/<code>.json` — one flat object
//! of `"key": "text"` per language, `{name}` where a value goes in — and loads
//! the person's language with English behind it: a word not translated yet
//! shows in English, never as a blank. A word that changes with a number has
//! a key per plural form, `key.one` and `key.other`, and is asked for with a
//! `count`: `t!("board.tickets", count = n)`.
//!
//! Which language ([`resolve`]): `EQUIPE_LANG` (an override), the language the
//! person picked — one choice for every front end, in `<home>/state.json` —
//! then, in a terminal, the locale (`LANGUAGE`, `LC_ALL`, `LC_MESSAGES`,
//! `LANG`), then the first of the system's preferred languages equipe speaks;
//! English when none is. The person is never asked up front, and nothing is
//! guessed from where they are. A front end switches language while it runs
//! ([`Words::set`]).
//!
//! equipe core has words of its own ([`core`]): what the daemon says to the
//! person — notices in conversations, errors — in the language their screen
//! said it speaks, and the labels of the protocol's names ([`label`]).
//! What equipe tells the models stays in English.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// The languages equipe speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Ko,
    En,
    Ja,
    ZhHans,
    ZhHant,
    Es,
    Fr,
    De,
}

impl Lang {
    pub const ALL: [Lang; 8] = [Lang::Ko, Lang::En, Lang::Ja, Lang::ZhHans, Lang::ZhHant, Lang::Es, Lang::Fr, Lang::De];

    /// Its code, as locale files are named: `ko`, `zh-Hans`.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Ko => "ko",
            Lang::En => "en",
            Lang::Ja => "ja",
            Lang::ZhHans => "zh-Hans",
            Lang::ZhHant => "zh-Hant",
            Lang::Es => "es",
            Lang::Fr => "fr",
            Lang::De => "de",
        }
    }

    /// Its name in itself, for a list to pick from.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::Ko => "한국어",
            Lang::En => "English",
            Lang::Ja => "日本語",
            Lang::ZhHans => "简体中文",
            Lang::ZhHant => "繁體中文",
            Lang::Es => "Español",
            Lang::Fr => "Français",
            Lang::De => "Deutsch",
        }
    }

    /// Its name in English, as the squad's prompts name the person's language.
    pub fn english_name(self) -> &'static str {
        match self {
            Lang::Ko => "Korean",
            Lang::En => "English",
            Lang::Ja => "Japanese",
            Lang::ZhHans => "Simplified Chinese",
            Lang::ZhHant => "Traditional Chinese",
            Lang::Es => "Spanish",
            Lang::Fr => "French",
            Lang::De => "German",
        }
    }

    /// Its place in [`Lang::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }

    /// The language a locale tag names — `ko_KR.UTF-8`, `zh-Hant-TW`,
    /// `zh_HK`, `en-US`, `es-419`, `de` — if equipe speaks it.
    pub fn parse(tag: &str) -> Option<Lang> {
        let tag = tag.trim().split(['.', '@']).next().unwrap_or_default().replace('_', "-").to_ascii_lowercase();
        let mut parts = tag.split('-');
        Some(match parts.next()? {
            "ko" => Lang::Ko,
            "en" => Lang::En,
            "ja" => Lang::Ja,
            "es" => Lang::Es,
            "fr" => Lang::Fr,
            "de" => Lang::De,
            "zh" => {
                let rest: Vec<&str> = parts.collect();
                if rest.iter().any(|p| matches!(*p, "hant" | "tw" | "hk" | "mo")) { Lang::ZhHant } else { Lang::ZhHans }
            }
            _ => return None,
        })
    }

    /// Which plural form `n` takes: `one` or `other`. Korean, Japanese and
    /// Chinese have none; English, Spanish and German say `one` for 1 only,
    /// French for 0 and 1.
    pub fn plural(self, n: u64) -> &'static str {
        match self {
            Lang::Ko | Lang::Ja | Lang::ZhHans | Lang::ZhHant => "other",
            Lang::En | Lang::Es | Lang::De if n == 1 => "one",
            Lang::Fr if n <= 1 => "one",
            _ => "other",
        }
    }

    /// The plural forms a word in it needs.
    pub fn plural_forms(self) -> &'static [&'static str] {
        match self {
            Lang::Ko | Lang::Ja | Lang::ZhHans | Lang::ZhHant => &["other"],
            _ => &["one", "other"],
        }
    }

    /// The person's language, for a front end of this kind: see [`resolve`].
    pub fn resolve(front: Front) -> Lang {
        Lang::resolve_in(&home(), front)
    }

    /// [`Lang::resolve`], with the pick kept in `home`.
    pub fn resolve_in(home: &std::path::Path, front: Front) -> Lang {
        resolve(&env, saved(home), front, &system_languages())
    }

    /// What "System" — no language picked — comes to right now: see [`auto`].
    /// A settings screen shows it beside "System".
    pub fn resolve_auto(front: Front) -> Lang {
        auto(&env, front, &system_languages())
    }
}

/// Which kind of front end asks: a terminal has a locale of its own; an app
/// has only the system's languages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Front {
    Terminal,
    App,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn home() -> std::path::PathBuf {
    equipe_providers::credentials::equipe_home()
}

/// The system's preferred languages, most preferred first. On a Mac,
/// `CFLocaleCopyPreferredLanguages`: it honours the language picked for
/// equipe alone in System Settings › General › Language & Region › Applications.
fn system_languages() -> Vec<String> {
    sys_locale::get_locales().collect()
}

/// A locale value that names no language: `C`, `POSIX`, `C.UTF-8`.
fn names_no_language(value: &str) -> bool {
    matches!(value.split(['.', '@']).next().unwrap_or_default(), "" | "C" | "POSIX")
}

/// The person's language: `EQUIPE_LANG` (for development, or to override),
/// the language they picked (`saved`), else what [`auto`] finds.
pub fn resolve(env: &dyn Fn(&str) -> Option<String>, saved: Option<Lang>, front: Front, system: &[String]) -> Lang {
    env("EQUIPE_LANG").and_then(|tag| Lang::parse(&tag)).or(saved).unwrap_or_else(|| auto(env, front, system))
}

/// The language nobody picked. In a terminal: GNU's `LANGUAGE` list, first
/// language equipe speaks — unless the locale is `C` or `POSIX`, as gettext
/// does; then the first of `LC_ALL`, `LC_MESSAGES`, `LANG` that names a
/// language. Then the system's preferred languages, walked in order: the first
/// one equipe speaks, so `[pt-BR, ko]` is Korean. English when none is.
pub fn auto(env: &dyn Fn(&str) -> Option<String>, front: Front, system: &[String]) -> Lang {
    if front == Front::Terminal {
        let set = |name: &str| env(name).filter(|v| !v.trim().is_empty());
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|name| set(name));
        if locale.as_deref().is_some_and(|l| !names_no_language(l))
            && let Some(lang) = set("LANGUAGE").and_then(|list| list.split(':').find_map(Lang::parse))
        {
            return lang;
        }
        let named = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|name| set(name).filter(|v| !names_no_language(v)));
        if let Some(lang) = named.as_deref().and_then(Lang::parse) {
            return lang;
        }
    }
    system.iter().find_map(|tag| Lang::parse(tag)).unwrap_or(Lang::En)
}

/// The language the person picked, for every front end and the daemon alike
/// (`lang` in `<home>/state.json`); `None`: none, "System".
pub fn saved(home: &std::path::Path) -> Option<Lang> {
    crate::squads::state(home).get("lang")?.as_str().and_then(Lang::parse)
}

/// Keep `lang` as the person's pick — or, `None` ("System"), forget it: only
/// a choice is kept, never what "System" came to.
pub fn save(home: &std::path::Path, lang: Option<Lang>) -> Result<(), String> {
    let mut state = crate::squads::state(home);
    if state.get("lang").and_then(|v| v.as_str()) == lang.map(Lang::code) {
        return Ok(());
    }
    match lang {
        Some(lang) => state.insert("lang".into(), lang.code().into()),
        None => state.remove("lang"),
    };
    crate::squads::save_state(home, state)
}

/// A front end's words in one language, with English behind it.
#[derive(Debug, Clone)]
pub struct Catalog {
    pub lang: Lang,
    words: HashMap<String, String>,
    english: HashMap<String, String>,
}

impl Catalog {
    /// Load `lang` from `files` — `(code, json)` for each language, as
    /// `include_str!` brings them in. A file that doesn't parse counts as empty.
    pub fn load(lang: Lang, files: &[(&str, &str)]) -> Self {
        let table = |lang: Lang| -> HashMap<String, String> {
            files.iter().find(|(code, _)| *code == lang.code()).and_then(|(_, json)| serde_json::from_str(json).ok()).unwrap_or_default()
        };
        Self { lang, words: table(lang), english: table(Lang::En) }
    }

    /// The word for `key`: in the language, else in English, else the key.
    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        self.words.get(key).or_else(|| self.english.get(key)).map_or(key, String::as_str)
    }

    /// The word for `key` in this language only — no English in its place.
    pub fn own(&self, key: &str) -> Option<&str> {
        self.words.get(key).map(String::as_str)
    }

    /// The word for `key` said of `n` things: its plural form for `n` in the
    /// language (`key.one`, `key.other`), else in English, else `key` itself.
    pub fn plural<'a>(&'a self, key: &'a str, n: u64) -> &'a str {
        fn pick<'t>(table: &'t HashMap<String, String>, key: &str, form: &str) -> Option<&'t str> {
            table.get(&format!("{key}.{form}")).or_else(|| table.get(&format!("{key}.other"))).or_else(|| table.get(key)).map(String::as_str)
        }
        pick(&self.words, key, self.lang.plural(n)).or_else(|| pick(&self.english, key, Lang::En.plural(n))).unwrap_or(key)
    }

    /// The word for `key` with each `{name}` filled in. With a `count` among
    /// them, the plural form for it.
    pub fn fill(&self, key: &str, args: &[(&str, String)]) -> String {
        let count = args.iter().find(|(name, _)| *name == "count").and_then(|(_, v)| v.trim().trim_start_matches('-').parse::<u64>().ok());
        match count {
            Some(n) => fill(self.plural(key, n), args),
            None => fill(self.get(key), args),
        }
    }
}

/// `text` with each `{name}` replaced by its value.
pub fn fill(text: &str, args: &[(&str, String)]) -> String {
    let mut out = text.to_owned();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// The `{name}`s a text takes, sorted — what every translation of it must take too.
pub fn placeholders(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) if after[..end].chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && end > 0 => {
                out.push(after[..end].to_owned());
                rest = &after[end + 1..];
            }
            _ => rest = after,
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A key's plural form, if it is one: `("board.tickets", "one")`.
fn plural_form(key: &str) -> Option<(&str, &str)> {
    let (base, form) = key.rsplit_once('.')?;
    matches!(form, "one" | "other").then_some((base, form))
}

/// What is wrong with a front end's translations: a key English lacks, one a
/// language lacks, or a translation taking other `{name}`s than English's.
/// A word with plural forms needs, in each language, the forms that language
/// has — Korean `.other` only, French `.one` and `.other` — and English both.
pub fn problems(files: &[(&str, &str)]) -> Vec<String> {
    let table = |code: &str| -> Result<HashMap<String, String>, String> {
        let json = files.iter().find(|(c, _)| *c == code).map(|(_, j)| *j).ok_or(format!("no {code}.json"))?;
        serde_json::from_str(json).map_err(|e| format!("{code}.json: {e}"))
    };
    let english = match table("en") {
        Ok(t) => t,
        Err(e) => return vec![e],
    };
    let mut out = Vec::new();
    for key in english.keys() {
        if let Some((base, _)) = plural_form(key) {
            for form in ["one", "other"] {
                if !english.contains_key(&format!("{base}.{form}")) {
                    out.push(format!("en: missing {base}.{form}"));
                }
            }
        }
    }
    for lang in Lang::ALL {
        let words = match table(lang.code()) {
            Ok(t) => t,
            Err(e) => {
                out.push(e);
                continue;
            }
        };
        for (key, text) in &english {
            let needed = match plural_form(key) {
                Some((_, form)) => lang.plural_forms().contains(&form),
                None => true,
            };
            match words.get(key) {
                None if needed => out.push(format!("{}: missing {key}", lang.code())),
                Some(word) if placeholders(word) != placeholders(text) => {
                    out.push(format!("{}: {key} takes {:?}, English {:?}", lang.code(), placeholders(word), placeholders(text)))
                }
                _ => {}
            }
        }
        for key in words.keys().filter(|k| !english.contains_key(*k)) {
            out.push(format!("{}: {key} isn't in English", lang.code()));
        }
        // A word in another language's letters: Korean outside Korean, Chinese
        // characters or kana in a language written in Latin letters.
        for (key, word) in &words {
            if let Some(stray) = word.chars().find(|c| foreign(lang, *c)) {
                out.push(format!("{}: {key} has '{stray}' from another language", lang.code()));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Whether `c` is written in a script `lang` doesn't use.
fn foreign(lang: Lang, c: char) -> bool {
    let hangul = matches!(c, '\u{AC00}'..='\u{D7A3}' | '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}');
    let kana = matches!(c, '\u{3040}'..='\u{30FF}');
    let han = matches!(c, '\u{4E00}'..='\u{9FFF}');
    match lang {
        Lang::Ko => kana,
        Lang::Ja => hangul,
        Lang::ZhHans | Lang::ZhHant => hangul || kana,
        _ => hangul || kana || han,
    }
}

/// A front end's words in every language, one of them in use — switched while
/// the front end runs ([`Words::set`]); each language is loaded the first time
/// it is used and kept, so a word stays a `&'static str`.
pub struct Words {
    files: &'static [(&'static str, &'static str)],
    first: fn() -> Lang,
    /// `Lang::index` of the language in use; `u8::MAX` until one is.
    current: AtomicU8,
    tables: [OnceLock<Catalog>; 8],
    /// [`Words::every`]'s lists, by key.
    lists: std::sync::Mutex<std::collections::BTreeMap<String, &'static [&'static str]>>,
}

impl Words {
    /// `files` as `include_str!` brings them in; `first` says the language
    /// to start in, asked the first time a word is.
    pub const fn new(files: &'static [(&'static str, &'static str)], first: fn() -> Lang) -> Self {
        Self { files, first, current: AtomicU8::new(u8::MAX), tables: [const { OnceLock::new() }; 8], lists: std::sync::Mutex::new(std::collections::BTreeMap::new()) }
    }

    /// The language in use.
    pub fn lang(&self) -> Lang {
        match self.current.load(Ordering::Acquire) {
            u8::MAX => {
                let first = (self.first)();
                let _ = self.current.compare_exchange(u8::MAX, first.index() as u8, Ordering::AcqRel, Ordering::Acquire);
                Lang::ALL[self.current.load(Ordering::Acquire) as usize]
            }
            i => Lang::ALL[i as usize],
        }
    }

    /// The words in the language in use.
    pub fn catalog(&self) -> &Catalog {
        self.of(self.lang())
    }

    /// The words in `lang`, whichever is in use.
    pub fn of(&self, lang: Lang) -> &Catalog {
        self.tables[lang.index()].get_or_init(|| Catalog::load(lang, self.files))
    }

    /// Use `lang` from now on: the next word asked for is in it.
    pub fn set(&self, lang: Lang) {
        self.current.store(lang.index() as u8, Ordering::Release);
    }

    /// `key`'s words in every language, each language's list `|`-separated:
    /// what text is matched against when it may be in any language (what a
    /// member wrote, a page's words), whichever one the screen is in.
    /// Read once per key and kept.
    pub fn every(&'static self, key: &str) -> &'static [&'static str] {
        let mut lists = self.lists.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(list) = lists.get(key) {
            return list;
        }
        let mut out: Vec<&'static str> = Vec::new();
        for lang in Lang::ALL {
            for word in self.of(lang).own(key).unwrap_or_default().split('|').filter(|w| !w.is_empty()) {
                if !out.contains(&word) {
                    out.push(word);
                }
            }
        }
        let list: &'static [&'static str] = Vec::leak(out);
        lists.insert(key.to_owned(), list);
        list
    }
}

/// equipe core's own words, in every language.
pub const FILES: &[(&str, &str)] = &[
    ("ko", include_str!("../locales/ko.json")),
    ("en", include_str!("../locales/en.json")),
    ("ja", include_str!("../locales/ja.json")),
    ("zh-Hans", include_str!("../locales/zh-Hans.json")),
    ("zh-Hant", include_str!("../locales/zh-Hant.json")),
    ("es", include_str!("../locales/es.json")),
    ("fr", include_str!("../locales/fr.json")),
    ("de", include_str!("../locales/de.json")),
];

static CORE: Words = Words::new(FILES, || Lang::En);

/// core's `key` words in every language (`Words::every`).
pub fn every(key: &str) -> &'static [&'static str] {
    CORE.every(key)
}

/// equipe core's words in `lang`: what the daemon says to the person.
pub fn core(lang: Lang) -> &'static Catalog {
    CORE.of(lang)
}

/// The protocol's names as a screen shows them, in a language. The wire
/// keeps its own (`in_progress`, `read_only`); these are only for people.
pub mod label {
    use super::{Lang, core};
    use equipe_protocol::{DesktopState, MemberRef, Mode, Priority, TicketKind, TicketStatus};

    /// `read-only`, `ask me`, `auto`, `full access`.
    pub fn mode(lang: Lang, mode: Mode) -> &'static str {
        core(lang).get(match mode {
            Mode::ReadOnly => "mode.read_only",
            Mode::Ask => "mode.ask",
            Mode::Auto => "mode.auto",
            Mode::Full => "mode.full",
        })
    }

    /// What a mode means, short enough for a menu line.
    pub fn mode_short(lang: Lang, mode: Mode) -> &'static str {
        core(lang).get(match mode {
            Mode::ReadOnly => "mode.read_only.short",
            Mode::Ask => "mode.ask.short",
            Mode::Auto => "mode.auto.short",
            Mode::Full => "mode.full.short",
        })
    }

    /// What a mode means, in full.
    pub fn mode_summary(lang: Lang, mode: Mode) -> &'static str {
        core(lang).get(match mode {
            Mode::ReadOnly => "mode.read_only.summary",
            Mode::Ask => "mode.ask.summary",
            Mode::Auto => "mode.auto.summary",
            Mode::Full => "mode.full.summary",
        })
    }

    /// `todo`, `in progress`, `review`…
    pub fn status(lang: Lang, status: TicketStatus) -> &'static str {
        core(lang).get(match status {
            TicketStatus::Proposed => "status.proposed",
            TicketStatus::Todo => "status.todo",
            TicketStatus::InProgress => "status.in_progress",
            TicketStatus::Waiting => "status.waiting",
            TicketStatus::Review => "status.review",
            TicketStatus::Done => "status.done",
            TicketStatus::Cancelled => "status.cancelled",
        })
    }

    /// `urgent`, `high`, `normal`, `low`.
    pub fn priority(lang: Lang, priority: Priority) -> &'static str {
        core(lang).get(match priority {
            Priority::Urgent => "priority.urgent",
            Priority::High => "priority.high",
            Priority::Normal => "priority.normal",
            Priority::Low => "priority.low",
        })
    }

    /// `code`, `task`, `action`.
    pub fn kind(lang: Lang, kind: TicketKind) -> &'static str {
        core(lang).get(match kind {
            TicketKind::Code => "kind.code",
            TicketKind::Task => "kind.task",
            TicketKind::Action => "kind.action",
        })
    }

    /// Who has the Mac's screen, in one line for a screen following squad
    /// `here` — [`DesktopState::summary`] in `lang`.
    pub fn desktop(lang: Lang, state: &DesktopState, here: Option<&str>) -> Option<String> {
        let words = core(lang);
        if state.stopped {
            return Some(words.get("desktop.stopped").to_owned());
        }
        let name = |m: &MemberRef| if here == Some(m.squad.as_str()) { m.member.clone() } else { m.to_string() };
        let holder = state.holder.as_ref().map(|m| words.fill("desktop.on_screen", &[("name", name(m))]));
        let waiting = (!state.queue.is_empty()).then(|| words.fill("desktop.waiting", &[("count", state.queue.len().to_string())]));
        let parts: Vec<String> = holder.into_iter().chain(waiting).collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// `fine`, `low`, `nearly full`, `full`.
    pub fn disk_level(lang: Lang, level: crate::disk::DiskLevel) -> &'static str {
        use crate::disk::DiskLevel as L;
        core(lang).get(match level {
            L::Ok => "disk.level.ok",
            L::Low => "disk.level.low",
            L::Critical => "disk.level.critical",
            L::Full => "disk.level.full",
        })
    }

    /// The one line a screen shows while the disk is short: `디스크 여유 4.2 GB — 곧
    /// 빌드가 실패할 수 있어요`. `None` while it's fine.
    pub fn disk_line(lang: Lang, status: &crate::disk::DiskStatus) -> Option<String> {
        use crate::disk::DiskLevel as L;
        let key = match status.level {
            L::Ok => return None,
            L::Low => "disk.line.low",
            L::Critical => "disk.line.critical",
            L::Full => "disk.line.full",
        };
        Some(core(lang).fill(key, &[("free", crate::disk::size(status.free))]))
    }

    /// Where an item is: `members' worktrees`, `the Mac's caches`…
    pub fn disk_group(lang: Lang, group: crate::disk::DiskGroup) -> &'static str {
        use crate::disk::DiskGroup as G;
        core(lang).get(match group {
            G::Worktrees => "disk.group.worktrees",
            G::Builds => "disk.group.builds",
            G::Projects => "disk.group.projects",
            G::Equipe => "disk.group.equipe",
            G::Caches => "disk.group.caches",
        })
    }

    /// What an item is: `build output`, `worktree`, `log`…
    pub fn disk_kind(lang: Lang, kind: crate::disk::DiskKind) -> &'static str {
        use crate::disk::DiskKind as K;
        core(lang).get(match kind {
            K::WorktreeBuild | K::ProjectBuild => "disk.kind.build_output",
            K::Worktree => "disk.kind.worktree",
            K::BuildCache => "disk.kind.build_cache",
            K::Log => "disk.kind.log",
            K::Browser => "disk.kind.browser",
            K::Journals => "disk.kind.journals",
            K::Uploads => "disk.kind.uploads",
            K::DerivedData => "disk.kind.derived_data",
            K::Simulators => "disk.kind.simulators",
            K::LibraryCache => "disk.kind.library_cache",
            K::Cargo => "disk.kind.cargo",
            K::Npm => "disk.kind.npm",
            K::Pnpm => "disk.kind.pnpm",
            K::Yarn => "disk.kind.yarn",
            K::Docker => "disk.kind.docker",
        })
    }

    /// Why an item may go, or stays: `done · in main`, `changes not committed`…
    pub fn disk_why(lang: Lang, why: crate::disk::DiskWhy) -> &'static str {
        use crate::disk::DiskWhy as W;
        core(lang).get(match why {
            W::Merged => "disk.why.merged",
            W::Cancelled => "disk.why.cancelled",
            W::NoTicket => "disk.why.no_ticket",
            W::OpenTicket => "disk.why.open_ticket",
            W::NotMerged => "disk.why.not_merged",
            W::Uncommitted => "disk.why.uncommitted",
            W::InUse => "disk.why.in_use",
            W::Stale => "disk.why.stale",
            W::Rebuildable => "disk.why.rebuildable",
            W::OldLog => "disk.why.old_log",
            W::Needed => "disk.why.needed",
            W::NotEquipes => "disk.why.not_equipes",
        })
    }
}

/// The ticket a line of news says waits for the person (`news.waiting`, in
/// any language: a line kept from before the language changed still reads).
pub fn waiting_ticket(text: &str) -> Option<crate::protocol::TicketId> {
    let rest = text.trim().strip_prefix("T-")?;
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let id = rest[..digits].parse().ok()?;
    let after = &rest[digits..];
    Lang::ALL
        .into_iter()
        .any(|lang| {
            let template = core(lang).get("news.waiting");
            let Some(tail) = template.strip_prefix("T-{id}") else { return false };
            let literal = tail.split('{').next().unwrap_or_default();
            !literal.trim().is_empty() && after.starts_with(literal)
        })
        .then_some(id)
}

/// A line of a conversation as the person reads it: the lines the transcript
/// writes itself (`Stopped.`, a ticket's branch…) in `lang`; any other line
/// — equipe's notices, already in the person's language — as it is.
pub fn note(lang: Lang, text: &str) -> std::borrow::Cow<'_, str> {
    use equipe_protocol::transcript::notes;
    use std::borrow::Cow;
    let words = core(lang);
    let fixed = match text {
        notes::STOPPED => Some("note.stopped"),
        notes::ENDED_EARLY => Some("note.ended_early"),
        notes::NOTHING_CHANGED => Some("note.nothing_changed"),
        _ => None,
    };
    if let Some(key) = fixed {
        return Cow::Borrowed(words.get(key));
    }
    if let Some(branch) = text.strip_prefix(notes::WORKING_ON).and_then(|rest| rest.strip_suffix(notes::WORKING_REST)) {
        return Cow::Owned(words.fill("note.working_on", &[("branch", branch.to_owned())]));
    }
    if text.starts_with("── T-")
        && let Some((head, branch)) = text.rsplit_once(notes::ON_BRANCH)
    {
        return Cow::Owned(words.fill("note.ticket_on_branch", &[("ticket", head.to_owned()), ("branch", branch.to_owned())]));
    }
    Cow::Borrowed(text)
}

/// Declare a front end's words, next to its `FILES` (each language's
/// `locales/<code>.json`, `include_str!`-ed): `catalog()`, the words in the
/// language in use — in tests always `$test`, what its tests read — `lang()`,
/// and `set`, to switch while it runs. `front` says how its language is found
/// (a [`Front`]; a terminal's by default). Then `t!("board.accept")` is the
/// word; `t!("chat.sent", name = who)` fills it in; `t!("board.tickets",
/// count = n)` picks the plural form for `n`.
#[macro_export]
macro_rules! words {
    (test = $test:expr) => {
        $crate::words!(test = $test, front = $crate::i18n::Front::Terminal);
    };
    (test = $test:expr, front = $front:expr) => {
        static WORDS: $crate::i18n::Words =
            $crate::i18n::Words::new(FILES, || if cfg!(test) { $test } else { $crate::i18n::Lang::resolve($front) });

        /// The words in the language in use.
        pub fn catalog() -> &'static $crate::i18n::Catalog {
            WORDS.catalog()
        }

        /// The language in use.
        #[allow(dead_code)]
        pub fn lang() -> $crate::i18n::Lang {
            WORDS.lang()
        }

        /// Use `lang` from now on: every word asked for after is in it.
        #[allow(dead_code)]
        pub fn set(lang: $crate::i18n::Lang) {
            WORDS.set(lang)
        }

        /// `key`'s words in every language (`Words::every`).
        #[allow(dead_code)]
        pub fn every(key: &str) -> &'static [&'static str] {
            WORDS.every(key)
        }

        /// The words in `lang`, whichever is in use.
        #[allow(dead_code)]
        pub fn of(lang: $crate::i18n::Lang) -> &'static $crate::i18n::Catalog {
            WORDS.of(lang)
        }
    };
}

/// The word for a key, from the calling crate's `i18n::catalog()` — `crate`,
/// not `$crate`: each front end has words of its own.
#[macro_export]
#[allow(clippy::crate_in_macro_def)]
macro_rules! t {
    ($key:literal) => {
        crate::i18n::catalog().get($key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        crate::i18n::catalog().fill($key, &[$((stringify!($name), ($value).to_string())),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locale_tag_names_its_language_whatever_its_shape() {
        assert_eq!(Lang::parse("ko_KR.UTF-8"), Some(Lang::Ko));
        assert_eq!(Lang::parse("en-US"), Some(Lang::En));
        assert_eq!(Lang::parse("zh-Hant-TW"), Some(Lang::ZhHant));
        assert_eq!(Lang::parse("zh_HK"), Some(Lang::ZhHant));
        assert_eq!(Lang::parse("zh-MO"), Some(Lang::ZhHant));
        assert_eq!(Lang::parse("zh_TW.UTF-8"), Some(Lang::ZhHant));
        assert_eq!(Lang::parse("zh-Hans-CN"), Some(Lang::ZhHans));
        assert_eq!(Lang::parse("zh-SG"), Some(Lang::ZhHans));
        assert_eq!(Lang::parse("zh"), Some(Lang::ZhHans));
        assert_eq!(Lang::parse("es-419"), Some(Lang::Es));
        assert_eq!(Lang::parse("de_DE@euro"), Some(Lang::De));
        assert_eq!(Lang::parse("pt-BR"), None);
        assert!(Lang::ALL.iter().all(|l| Lang::parse(l.code()) == Some(*l)));
        assert!(Lang::ALL.iter().enumerate().all(|(i, l)| l.index() == i));
    }

    fn env_of(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| vars.iter().find(|(n, _)| *n == name).map(|(_, v)| (*v).to_owned())
    }

    fn system(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|t| (*t).to_owned()).collect()
    }

    #[test]
    fn the_language_is_the_override_then_the_pick_then_language_then_the_locale_then_the_system_then_english() {
        let all = env_of(&[("EQUIPE_LANG", "de"), ("LANGUAGE", "fr:en"), ("LC_ALL", "es_ES.UTF-8"), ("LANG", "ja_JP.UTF-8")]);
        let mac = system(&["zh-Hant-TW"]);
        let term = Front::Terminal;
        assert_eq!(resolve(&all, Some(Lang::Ko), term, &mac), Lang::De, "EQUIPE_LANG first");
        let picked = env_of(&[("LANGUAGE", "fr:en"), ("LC_ALL", "es_ES.UTF-8"), ("LANG", "ja_JP.UTF-8")]);
        assert_eq!(resolve(&picked, Some(Lang::Ko), term, &mac), Lang::Ko, "then the person's pick");
        assert_eq!(resolve(&picked, None, term, &mac), Lang::Fr, "then LANGUAGE");
        let locale = env_of(&[("LC_MESSAGES", "es_ES.UTF-8"), ("LANG", "ja_JP.UTF-8")]);
        assert_eq!(resolve(&locale, None, term, &mac), Lang::Es, "then LC_ALL, LC_MESSAGES, LANG");
        let none = env_of(&[]);
        assert_eq!(resolve(&none, None, term, &mac), Lang::ZhHant, "then the system's");
        assert_eq!(resolve(&none, None, term, &[]), Lang::En, "else English");
        assert_eq!(resolve(&env_of(&[("EQUIPE_LANG", "klingon")]), None, term, &mac), Lang::ZhHant, "an override equipe can't speak is no override");
    }

    #[test]
    fn language_is_a_list_and_counts_only_where_the_locale_names_a_language() {
        let mac = system(&["ko-KR"]);
        let term = Front::Terminal;
        assert_eq!(auto(&env_of(&[("LANGUAGE", "pt_BR:it:de"), ("LANG", "en_US.UTF-8")]), term, &mac), Lang::De, "the first it speaks");
        assert_eq!(auto(&env_of(&[("LANGUAGE", "fr"), ("LC_ALL", "C")]), term, &mac), Lang::Ko, "the C locale turns LANGUAGE off, as gettext does");
        assert_eq!(auto(&env_of(&[("LANGUAGE", "fr")]), term, &mac), Lang::Ko, "so does no locale at all");
        assert_eq!(auto(&env_of(&[("LANGUAGE", "pt"), ("LANG", "ja_JP")]), term, &mac), Lang::Ja, "nothing it speaks: the locale");
        assert_eq!(auto(&env_of(&[("LC_ALL", "POSIX"), ("LANG", "fr_FR.UTF-8")]), term, &mac), Lang::Fr, "C and POSIX are skipped");
        assert_eq!(auto(&env_of(&[("LC_ALL", "C.UTF-8")]), term, &mac), Lang::Ko);
        assert_eq!(auto(&env_of(&[("LANG", "pt_BR.UTF-8")]), term, &mac), Lang::Ko, "a locale it doesn't speak: the system's");
    }

    #[test]
    fn an_app_walks_the_system_list_past_languages_it_doesnt_speak() {
        let locale = env_of(&[("LANG", "fr_FR.UTF-8"), ("LANGUAGE", "de")]);
        assert_eq!(auto(&locale, Front::App, &system(&["pt-BR", "ko-KR", "en-US"])), Lang::Ko, "an app has no locale of its own");
        assert_eq!(auto(&locale, Front::App, &system(&["zh-Hant-HK", "en"])), Lang::ZhHant);
        assert_eq!(auto(&locale, Front::App, &system(&["zh-Hans-CN"])), Lang::ZhHans);
        assert_eq!(auto(&locale, Front::App, &system(&["it-IT", "pt-PT"])), Lang::En);
        assert_eq!(resolve(&env_of(&[("EQUIPE_LANG", "ja")]), None, Front::App, &system(&["ko"])), Lang::Ja);
    }

    #[test]
    fn the_pick_is_kept_and_system_forgets_it() {
        let home = std::env::temp_dir().join(format!("equipe-lang-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(saved(&home), None, "nothing picked yet");
        crate::squads::remember_last(&home, "acme").unwrap();
        save(&home, Some(Lang::ZhHant)).unwrap();
        assert_eq!(saved(&home), Some(Lang::ZhHant));
        let text = std::fs::read_to_string(home.join("state.json")).unwrap();
        assert!(text.contains(r#""lang": "zh-Hant""#), "{text}");
        assert_eq!(crate::squads::last_squad(&home).as_deref(), Some("acme"), "the rest of the state stays");
        save(&home, None).unwrap();
        assert_eq!(saved(&home), None);
        let text = std::fs::read_to_string(home.join("state.json")).unwrap();
        assert!(!text.contains("lang"), "System is kept as no pick at all: {text}");
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_word_not_translated_yet_shows_in_english_and_values_fill_in() {
        let files = [("ko", r#"{"hi": "안녕 {name}"}"#), ("en", r#"{"hi": "Hi {name}", "bye": "Bye"}"#)];
        let ko = Catalog::load(Lang::Ko, &files);
        assert_eq!(ko.fill("hi", &[("name", "Abel".into())]), "안녕 Abel");
        assert_eq!(ko.get("bye"), "Bye");
        assert_eq!(ko.get("nope"), "nope");
        assert_eq!(Catalog::load(Lang::Ja, &files).get("bye"), "Bye");
    }

    #[test]
    fn plural_forms_follow_each_languages_rule() {
        assert_eq!([0, 1, 2].map(|n| Lang::En.plural(n)), ["other", "one", "other"]);
        assert_eq!([0, 1, 2].map(|n| Lang::De.plural(n)), ["other", "one", "other"]);
        assert_eq!([0, 1, 2].map(|n| Lang::Es.plural(n)), ["other", "one", "other"]);
        assert_eq!([0, 1, 2].map(|n| Lang::Fr.plural(n)), ["one", "one", "other"]);
        for lang in [Lang::Ko, Lang::Ja, Lang::ZhHans, Lang::ZhHant] {
            assert_eq!([0, 1, 2].map(|n| lang.plural(n)), ["other"; 3]);
        }
        let files = [
            ("en", r#"{"t.one": "{count} ticket", "t.other": "{count} tickets", "plain": "{count} things"}"#),
            ("fr", r#"{"t.one": "{count} ticket", "t.other": "{count} tickets"}"#),
            ("ko", r#"{"t.other": "티켓 {count}개"}"#),
            ("de", r#"{}"#),
        ];
        let count = |lang, n: u64| Catalog::load(lang, &files).fill("t", &[("count", n.to_string())]);
        assert_eq!([count(Lang::En, 1), count(Lang::En, 0), count(Lang::En, 3)], ["1 ticket", "0 tickets", "3 tickets"]);
        assert_eq!([count(Lang::Fr, 0), count(Lang::Fr, 2)], ["0 ticket", "2 tickets"]);
        assert_eq!([count(Lang::Ko, 1), count(Lang::Ko, 5)], ["티켓 1개", "티켓 5개"]);
        assert_eq!(count(Lang::De, 1), "1 ticket", "untranslated: English's form for the number");
        let en = Catalog::load(Lang::En, &files);
        assert_eq!(en.fill("plain", &[("count", "1".into())]), "1 things", "a word without forms is the word");
        assert_eq!(en.fill("t", &[("count", "many".into())]), "t", "no number, no form");
    }

    #[test]
    fn translations_that_lack_a_word_or_take_other_values_are_found() {
        let mut files: Vec<(&str, &str)> = Lang::ALL.iter().map(|l| (l.code(), r#"{"hi": "x {name}"}"#)).collect();
        assert!(problems(&files).is_empty());
        files[2] = ("ja", r#"{"hi": "x {who}", "extra": "y"}"#);
        let found = problems(&files);
        assert!(found.iter().any(|p| p.starts_with("ja: hi takes")), "{found:?}");
        assert!(found.iter().any(|p| p == "ja: extra isn't in English"), "{found:?}");
        assert_eq!(placeholders("{a} and {b}, {a} — {not a name}"), ["a", "b"]);
    }

    #[test]
    fn a_plural_word_needs_the_forms_each_language_has() {
        let one_other = r#"{"t.one": "{count} t", "t.other": "{count} ts"}"#;
        let other = r#"{"t.other": "{count} t"}"#;
        let files: Vec<(&str, &str)> =
            Lang::ALL.iter().map(|l| (l.code(), if l.plural_forms().len() == 1 { other } else { one_other })).collect();
        assert!(problems(&files).is_empty(), "{:?}", problems(&files));
        let mut lacking = files.clone();
        lacking[Lang::Fr.index()] = ("fr", other);
        lacking[Lang::Ko.index()] = ("ko", r#"{"t.one": "{count} t"}"#);
        let found = problems(&lacking);
        assert_eq!(found, ["fr: missing t.one", "ko: missing t.other"]);
        let mut english = files.clone();
        english[Lang::En.index()] = ("en", r#"{"t.one": "a t"}"#);
        assert!(problems(&english).contains(&"en: missing t.other".to_owned()), "{:?}", problems(&english));
    }

    #[test]
    fn words_switch_language_while_running() {
        static FILES: &[(&str, &str)] = &[("ko", r#"{"hi": "안녕"}"#), ("en", r#"{"hi": "Hi"}"#), ("ja", r#"{"hi": "やあ"}"#)];
        let words = Words::new(FILES, || Lang::Ko);
        assert_eq!((words.lang(), words.catalog().get("hi")), (Lang::Ko, "안녕"));
        words.set(Lang::Ja);
        assert_eq!((words.lang(), words.catalog().get("hi")), (Lang::Ja, "やあ"));
        assert_eq!(words.of(Lang::En).get("hi"), "Hi", "any language, whichever is in use");
    }

    #[test]
    fn core_has_every_word_in_every_language() {
        let found = problems(FILES);
        assert!(found.is_empty(), "{}", found.join("\n"));
        assert_eq!(label::mode(Lang::En, equipe_protocol::Mode::Ask), equipe_protocol::Mode::Ask.label());
        assert_eq!(label::status(Lang::En, equipe_protocol::TicketStatus::InProgress), "in progress");
        for mode in equipe_protocol::Mode::ALL {
            assert_eq!(label::mode(Lang::En, mode), mode.label());
            assert_eq!(label::mode_short(Lang::En, mode), mode.short());
            assert_eq!(label::mode_summary(Lang::En, mode), mode.summary());
        }
        assert_eq!(label::kind(Lang::De, equipe_protocol::TicketKind::Task), "Aufgabe");
    }

    #[test]
    fn the_transcripts_own_lines_read_in_the_persons_language_and_the_rest_as_they_are() {
        use equipe_protocol::transcript::notes;
        assert_eq!(note(Lang::En, notes::STOPPED), notes::STOPPED);
        assert_eq!(note(Lang::Ko, notes::STOPPED), "멈췄습니다.");
        let working = format!("{}equipe/try-1{}", notes::WORKING_ON, notes::WORKING_REST);
        assert_eq!(note(Lang::En, &working), working, "English reads as it always did");
        assert!(note(Lang::Fr, &working).contains("equipe/try-1"));
        let ticket = format!("── T-4 · a — b{}equipe/t-4", notes::ON_BRANCH);
        assert_eq!(note(Lang::En, &ticket), ticket);
        assert_eq!(note(Lang::Ko, &ticket), "── T-4 · a — b — 브랜치 equipe/t-4");
        assert_eq!(note(Lang::Ko, "✓ 허용함: ls"), "✓ 허용함: ls", "a notice is in the person's language already");
    }

    #[test]
    fn a_waiting_line_names_its_ticket_in_any_language() {
        for lang in Lang::ALL {
            let line = core(lang).fill("news.waiting", &[("id", "117".into()), ("name", "ios".into()), ("title", "앱스토어 제출".into())]);
            assert_eq!(waiting_ticket(&line), Some(117), "{lang:?}: {line}");
        }
        assert_eq!(waiting_ticket("T-4 · Research"), None);
        assert_eq!(waiting_ticket("You accepted T-58 · 헤더."), None);
    }

    #[test]
    fn who_has_the_screen_reads_as_before_in_english_and_counts_in_each_language() {
        use equipe_protocol::{DesktopState, MemberRef};
        let who = |squad: &str, member: &str| MemberRef { squad: squad.into(), member: member.into() };
        let state = DesktopState { holder: Some(who("acme", "qa")), queue: vec![who("beta", "pm")], stopped: false };
        assert_eq!(label::desktop(Lang::En, &state, Some("acme")), state.summary(Some("acme")));
        assert_eq!(label::desktop(Lang::Ko, &state, Some("acme")).as_deref(), Some("qa이(가) 화면 사용 중 · 1명 대기"));
        let two = DesktopState { queue: vec![who("a", "x"), who("b", "y")], ..DesktopState::default() };
        assert_eq!(label::desktop(Lang::De, &two, None).as_deref(), Some("2 warten"));
        assert_eq!(label::desktop(Lang::En, &DesktopState::default(), None), None);
        let stopped = DesktopState { stopped: true, ..DesktopState::default() };
        assert_eq!(label::desktop(Lang::En, &stopped, None), stopped.summary(None));
    }
}
