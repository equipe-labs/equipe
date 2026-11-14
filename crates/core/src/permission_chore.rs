//! Permission chores: a member asking the person, in steps, to change macOS
//! privacy permissions that equipe's own desktop tools already hold — or can
//! ask for with one click.
//!
//! Members work apps through equipe's desktop tools (`mcp__desktop__…`), which
//! run as equipe itself and keep Accessibility and Screen Recording. A
//! member that reaches for shell `screencapture` or `osascript` UI scripting
//! runs as the command-line binary instead, finds that identity without the
//! permission, and used to hand the person a four-step chore: add the binary
//! under Privacy & Security, restart, handle Office's file-access dialog.
//!
//! [`judge`] looks at what `block_on_person` was given:
//!
//! - nothing about permissions: left as it is;
//! - a permission chore while the desktop already has the permission:
//!   refused with one line to the member, the person is never shown it;
//! - a permission chore while one is really missing: every such step is
//!   replaced by one card ([`crate::protocol::asks::permission_ask`]), whose
//!   one button runs the person-initiated flow that asks macOS and opens System
//!   Settings at the pane, and which says which Mac when that isn't the one
//!   in front of them.
//!
//! A chore is found by its structure first (a System Settings privacy URL, a
//! TCC service name, `tccutil`, the equipe binary's path) and by words second:
//! the panes' names in each language live in the locale files
//! (`match.privacy_*`), never here.

use equipe_protocol::DesktopReadiness;
use equipe_protocol::asks::{Permission, permission_ask, permission_kinds};

/// What an ask is, as far as permissions go.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chore {
    /// The permissions it names (none named: it is about "the permissions").
    pub kinds: Vec<Permission>,
    /// It also names a pane equipe has no one-click flow for (Full Disk
    /// Access, Input Monitoring, Automation…): left to the person as written.
    pub other: bool,
    /// An Office "Grant File Access" dialog: a click equipe's desktop tools
    /// make themselves, never the person's.
    pub dialog: bool,
    /// It is about a privacy pane or the permission itself, not only the dialog.
    pub pane: bool,
}

fn mentions(lower: &str, key: &str) -> bool {
    crate::i18n::every(key).iter().any(|w| lower.contains(w))
}

/// Whether the text points at the equipe binary itself: the file the person
/// was told to add by hand.
fn names_the_binary(lower: &str) -> bool {
    lower.contains("/bin/equipe") || lower.contains("equipe.app") || lower.contains(".local/bin")
}

/// Whether the text is the "restart equipe" that follows permission steps.
fn followup(text: &str) -> bool {
    mentions(&text.to_lowercase(), "match.privacy_followup_words")
}

/// The permissions whose panes (or TCC names) the lowercased text names.
fn pane_kinds(lower: &str) -> Vec<Permission> {
    let mut kinds = Vec::new();
    if mentions(lower, "match.privacy_screen_words") || lower.contains("privacy_screencapture") || lower.contains("ktccservicescreencapture") {
        kinds.push(Permission::ScreenRecording);
    }
    if mentions(lower, "match.privacy_accessibility_words") || lower.contains("privacy_accessibility") || lower.contains("ktccserviceaccessibility") {
        kinds.push(Permission::Accessibility);
    }
    kinds
}

/// What the text asks of the person about permissions, or `None` when it
/// asks nothing of the kind.
pub fn classify(text: &str) -> Option<Chore> {
    let lower = text.to_lowercase();
    let structural = ["x-apple.systempreferences", "com.apple.preference.security", "tccutil", "ktccservice", "privacy_screencapture", "privacy_accessibility", "privacy_all"]
        .iter()
        .any(|s| lower.contains(s));
    let kinds = pane_kinds(&lower);
    let other = mentions(&lower, "match.privacy_other_words");
    let settings = mentions(&lower, "match.privacy_settings_words");
    let dialog = mentions(&lower, "match.office_file_access_words");
    let chore = structural || (settings && (!kinds.is_empty() || other)) || (names_the_binary(&lower) && !kinds.is_empty());
    if !chore && !dialog {
        return None;
    }
    // A dialog alone is no permission; with a chore it goes with it.
    if !chore {
        return Some(Chore { kinds: Vec::new(), other: false, dialog: true, pane: false });
    }
    Some(Chore { kinds, other, dialog, pane: true })
}

/// Whether equipe's desktop tools hold `kind` on the daemon's Mac now.
pub fn holds(ready: &DesktopReadiness, kind: Permission) -> bool {
    ready.helper
        && match kind {
            Permission::Accessibility => ready.accessibility == Some(true),
            Permission::ScreenRecording => ready.screen == Some(true),
        }
}

fn names(kinds: &[Permission]) -> String {
    kinds
        .iter()
        .map(|k| match k {
            Permission::Accessibility => "Accessibility",
            Permission::ScreenRecording => "Screen Recording",
        })
        .collect::<Vec<_>>()
        .join(" and ")
}

/// The one line a member is told when it hands the person a permission chore
/// that equipe's desktop tools have no need of.
pub fn refusal(held: &[Permission]) -> String {
    format!(
        "Not put to the person: Desktop control already has {} — use mcp__desktop__screenshot / snapshot / act (they run as equipe, \
which holds the permission) instead of shell screencapture / osascript, which run as the command-line binary and never get it; \
the person is never asked to grant permissions to that binary, and Office's \"Grant File Access\" dialog is a click `act` makes. \
If the screen is locked, wait for it to unlock.",
        names(held)
    )
}

/// What `block_on_person` does with its asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judged {
    /// What the person is shown: the asks that aren't permission chores, then
    /// one permission card when a permission is really missing.
    pub asks: Vec<String>,
    /// What to tell the member when its steps were turned into the card.
    pub told: Option<String>,
}

/// Judge what a member put down for the person (`needs`, and the permissions
/// it asked for by name in `named`) against what the daemon's desktop holds
/// (`ready`: `None` before the first look).
pub fn judge(needs: &[String], named: &[Permission], ready: Option<&DesktopReadiness>) -> Result<Judged, String> {
    let chores: Vec<Option<Chore>> = needs.iter().map(|n| classify(n)).collect();
    // A card the member asked for by name, or one already in its list.
    let carded: Vec<Permission> = needs.iter().filter_map(|n| permission_kinds(n)).flatten().collect();
    // A pane equipe has no flow for (Full Disk Access…) stays the person's, as written.
    let ours = |c: &Option<Chore>| c.as_ref().is_some_and(|c| !c.other || !c.kinds.is_empty() || c.dialog);
    // A real permission chore, not only the file-access dialog that follows one.
    let real = |c: &Option<Chore>| c.as_ref().is_some_and(|c| c.pane && (!c.other || !c.kinds.is_empty()));
    let of_ours = chores.iter().filter(|c| ours(c)).count();
    if of_ours == 0 && named.is_empty() && carded.is_empty() {
        return Ok(Judged { asks: needs.to_vec(), told: None });
    }
    // Not a desktop that has these permissions at all (another OS): nothing to compare.
    if ready.is_some_and(|r| !r.supported) {
        return Ok(Judged { asks: needs.to_vec(), told: None });
    }
    // What goes with permission steps: "do the same under Accessibility" and
    // "restart equipe" are not chores alone, only beside one.
    let with_steps = chores.iter().any(real);
    let weak = |n: &str| with_steps && classify(n).is_none() && permission_kinds(n).is_none() && (followup(n) || !pane_kinds(&n.to_lowercase()).is_empty());
    let mut kinds: Vec<Permission> = named.iter().chain(&carded).copied().collect();
    for chore in chores.iter().flatten() {
        kinds.extend(chore.kinds.iter().copied());
    }
    for n in needs.iter().filter(|n| weak(n)) {
        kinds.extend(pane_kinds(&n.to_lowercase()));
    }
    if kinds.is_empty() {
        // "The permissions", unspecified: both. The dialog alone needs only the clicks.
        kinds = if chores.iter().any(real) { Permission::ALL.to_vec() } else { vec![Permission::Accessibility] };
    }
    kinds.sort();
    kinds.dedup();
    let missing: Vec<Permission> = kinds.iter().copied().filter(|k| !ready.is_some_and(|r| holds(r, *k))).collect();
    if missing.is_empty() {
        return Err(refusal(&kinds));
    }
    let goes = |n: &str, c: &Option<Chore>| ours(c) || permission_kinds(n).is_some() || weak(n);
    let mut asks: Vec<String> = needs.iter().zip(&chores).filter(|(n, c)| !goes(n, c)).map(|(n, _)| n.clone()).collect();
    asks.push(permission_ask(&missing));
    let replaced = needs.iter().zip(&chores).filter(|(n, c)| goes(n, c) && permission_kinds(n).is_none()).count();
    let told = (replaced > 0).then(|| {
        format!(
            "Your {replaced} permission step(s) were not shown as steps: the person gets one card with one button that asks macOS and opens \
System Settings at the {} pane. Don't write steps for permissions, and never ask them to add the equipe binary by hand. \
Once granted, equipe's desktop tools work by themselves (including Office's \"Grant File Access\" dialog, which you click with `act`).",
            names(&missing)
        )
    });
    Ok(Judged { asks, told })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(accessibility: bool, screen: bool) -> DesktopReadiness {
        DesktopReadiness { supported: true, helper: true, engine: true, accessibility: Some(accessibility), screen: Some(screen), ..Default::default() }
    }

    /// The four steps a designer handed the person.
    fn chore_steps() -> Vec<String> {
        vec![
            "Open System Settings › Privacy & Security › Screen & System Audio Recording, press + and add `/Users/me/.local/bin/equipe` (⌘⇧G in Finder).".into(),
            "Do the same under Accessibility.".into(),
            "Restart equipe.".into(),
            "Handle Excel's 'Grant File Access' dialog.".into(),
        ]
    }

    #[test]
    fn what_is_and_is_not_a_permission_chore() {
        let screen = classify("Open System Settings › Privacy & Security › Screen Recording and turn it on").unwrap();
        assert_eq!(screen.kinds, vec![Permission::ScreenRecording]);
        let url = classify("open x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility").unwrap();
        assert_eq!(url.kinds, vec![Permission::Accessibility]);
        assert_eq!(classify("run tccutil reset All").unwrap().kinds, vec![]);
        assert!(classify("add /Users/me/.local/bin/equipe under Accessibility").is_some());
        let fda = classify("System Settings › Privacy & Security › Full Disk Access: add Terminal").unwrap();
        assert!(fda.other && fda.kinds.is_empty());
        assert!(classify("Handle the 'Grant File Access' dialog in Excel").unwrap().dialog);
        // Talking about accessibility is not asking for the permission.
        for plain in ["Check the accessibility tree of the checkout page", "Issue a GitHub token", "Which plan do we take?", "Restart the dev server"] {
            assert_eq!(classify(plain), None, "{plain}");
        }
        // In the person's language, too.
        assert_eq!(classify("시스템 설정 › 개인정보 보호 및 보안 › 화면 기록에서 equipe를 켜 주세요").unwrap().kinds, vec![Permission::ScreenRecording]);
        assert_eq!(classify("Systemeinstellungen › Datenschutz & Sicherheit › Bedienungshilfen").unwrap().kinds, vec![Permission::Accessibility]);
    }

    #[test]
    fn a_chore_the_desktop_has_no_need_of_is_refused_in_one_line_and_never_shown() {
        let refused = judge(&chore_steps(), &[], Some(&ready(true, true))).unwrap_err();
        assert!(refused.contains("Desktop control already has Accessibility and Screen Recording"), "{refused}");
        assert!(refused.contains("mcp__desktop__screenshot") && refused.contains("screencapture") && refused.contains("unlock"), "{refused}");
        assert!(!refused.contains('\n'), "one line");
        // Only the permission the steps name counts.
        let only_screen = ["System Settings › Privacy & Security › Screen Recording: turn equipe on".to_owned()];
        assert!(judge(&only_screen, &[], Some(&ready(false, true))).unwrap_err().contains("already has Screen Recording"));
        // Named by the member the structured way: same.
        assert!(judge(&[], &[Permission::Accessibility], Some(&ready(true, false))).is_err());
    }

    #[test]
    fn a_missing_permission_is_one_card_in_place_of_the_steps() {
        let mut needs = chore_steps();
        needs.push("Send me the Figma file name.".into());
        let judged = judge(&needs, &[], Some(&ready(true, false))).unwrap();
        assert_eq!(judged.asks, vec!["Send me the Figma file name.".to_owned(), permission_ask(&[Permission::ScreenRecording])]);
        assert!(judged.told.unwrap().contains("one card"));
        // Both missing, named by the member: one card for both.
        let judged = judge(&[], &[Permission::ScreenRecording, Permission::Accessibility], Some(&ready(false, false))).unwrap();
        assert_eq!(judged.asks, vec![permission_ask(&Permission::ALL)]);
        // A card already in the list isn't doubled.
        let judged = judge(&[permission_ask(&[Permission::ScreenRecording])], &[], Some(&ready(true, false))).unwrap();
        assert_eq!(judged.asks, vec![permission_ask(&[Permission::ScreenRecording])]);
        // Before the first look at the desktop: the card, with what the steps name.
        let judged = judge(&chore_steps(), &[], None).unwrap();
        assert_eq!(judged.asks, vec![permission_ask(&Permission::ALL)]);
    }

    #[test]
    fn other_asks_and_other_systems_go_through_untouched() {
        let plain = vec!["Issue a GitHub token.".to_owned()];
        assert_eq!(judge(&plain, &[], Some(&ready(false, false))).unwrap(), Judged { asks: plain.clone(), told: None });
        let fda = vec!["System Settings › Privacy & Security › Full Disk Access: add Terminal".to_owned()];
        assert_eq!(judge(&fda, &[], Some(&ready(false, false))).unwrap().asks, fda);
        let linux = DesktopReadiness { supported: false, ..Default::default() };
        assert_eq!(judge(&chore_steps(), &[], Some(&linux)).unwrap().asks, chore_steps());
    }
}
