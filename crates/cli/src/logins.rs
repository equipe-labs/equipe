//! `equipe logins stats`: why the person was asked to sign in in members'
//! browsers, from the squads' journals (`equipe_core::logins`) — per site
//! per week by cause, how long a sign-in lasted before the site asked
//! again, the same site asked for by different members within a day, and
//! the share of sign-ins equipe finished by itself. Read-only: the journals
//! are opened without writing, and nothing but hosts, names and causes is
//! in them.

use clap::{Args, Subcommand};
use equipe_core::logins::{self, Cause, Stats};
use equipe_core::t;
use serde_json::json;

#[derive(Subcommand)]
pub enum LoginsAction {
    /// How often you were asked to sign in for members, where, and why
    Stats(StatsArgs),
}

/// `equipe credential aws <profile>`: what an AWS CLI profile's
/// `credential_process` runs (login autopilot set it up): the profile's key
/// from the secret store, as the AWS CLI reads it. Refused to a member's
/// command calling it directly — only the AWS CLI itself gets it there.
pub fn credential(kind: &str, name: &str) -> Result<(), String> {
    if kind != "aws" {
        return Err(t!("credential.unknown", kind = kind));
    }
    let member = equipe_core::guard::run_by_member(std::env::var_os(equipe_core::guard::MEMBER_ENV));
    if member && !equipe_core::autopilot::cli_logins::parent_is_aws() {
        return Err(t!("credential.refused").to_owned());
    }
    let home = equipe_providers::credentials::equipe_home();
    let vault = equipe_core::vault::Vault::open(&home, equipe_core::vault::system_store());
    let login = vault.cli.iter().find(|c| c.kind == equipe_core::protocol::vault::CliKind::AwsKeys && c.name == name).ok_or_else(|| t!("credential.missing", name = name))?;
    let secret = vault.secret(&equipe_core::vault::account::cli(login.id))?;
    println!("{}", equipe_core::autopilot::cli_logins::aws_process_output(&login.public, &secret));
    Ok(())
}

#[derive(Args)]
pub struct StatsArgs {
    /// One squad (default: every squad — they share one browser)
    #[arg(long)]
    pub squad: Option<String>,
    /// From when: 7d, 24h, 2w, or a day (2026-11-01)
    #[arg(long, default_value = "7d")]
    pub since: String,
    /// The measures and every row, as JSON
    #[arg(long)]
    pub json: bool,
}

pub fn command(action: LoginsAction) -> Result<(), String> {
    match action {
        LoginsAction::Stats(args) => stats(args),
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn stats(a: StatsArgs) -> Result<(), String> {
    let home = equipe_providers::credentials::equipe_home();
    let since = equipe_core::design::qa_stats::since(&a.since, now()).ok_or_else(|| t!("logins.bad_since", since = a.since))?.max(0) as u64;
    let squads: Vec<String> = match a.squad {
        Some(squad) => vec![squad],
        None => equipe_core::squads::list(&home).into_iter().map(|s| s.name).collect(),
    };
    let mut rows = Vec::new();
    for squad in &squads {
        let db = equipe_core::journal::squad_dir(&home, squad).join("squad.db");
        if !db.is_file() {
            continue;
        }
        for mut row in logins::read(&db, since)? {
            row.squad = squad.clone();
            rows.push(row);
        }
    }
    let stats = logins::summarize(&rows);
    if a.json {
        println!("{}", serde_json::to_string_pretty(&json!({ "since": since, "squads": squads, "stats": stats, "rows": rows })).unwrap_or_default());
    } else {
        println!("{}", text(&a.since, &stats));
    }
    Ok(())
}

/// A cause in the person's words.
fn cause_word(cause: Cause) -> &'static str {
    match cause {
        Cause::SessionExpired => t!("logins.cause_session_expired"),
        Cause::NoLoginKept => t!("logins.cause_no_login_kept"),
        Cause::Captcha => t!("logins.cause_captcha"),
        Cause::TwoFactor => t!("logins.cause_two_factor"),
        Cause::Passkey => t!("logins.cause_passkey"),
        Cause::DeviceCode => t!("logins.cause_device_code"),
        Cause::Consent => t!("logins.cause_consent"),
        Cause::Other => t!("logins.cause_other"),
    }
}

/// The report, as printed.
pub(crate) fn text(since: &str, s: &Stats) -> String {
    if s.asked == 0 && s.signed_in == 0 {
        return t!("logins.none", since = since);
    }
    let causes = |by: &std::collections::BTreeMap<Cause, u64>| by.iter().map(|(c, n)| format!("{} {n}", cause_word(*c))).collect::<Vec<_>>().join(", ");
    let mut out = vec![t!("logins.heading", since = since, count = s.asked)];
    if !s.by_cause.is_empty() {
        out.push(t!("logins.by_cause", causes = causes(&s.by_cause)));
    }
    if !s.site_weeks.is_empty() {
        out.push(t!("logins.by_site").to_owned());
        for w in &s.site_weeks {
            out.push(t!("logins.site_week", site = w.site, week = equipe_core::squad_memory::date(w.week), count = w.total, causes = causes(&w.causes)));
        }
    }
    if !s.lasts.is_empty() {
        out.push(t!("logins.lasts").to_owned());
        for (site, median, n) in &s.lasts {
            out.push(t!("logins.last_row", site = site, span = logins::span(*median), n = n));
        }
    }
    let repeats: Vec<String> = s.repeats.iter().map(|(site, n)| format!("{site} {n}")).collect();
    out.push(if repeats.is_empty() { t!("logins.repeats_none").to_owned() } else { t!("logins.repeats", count = s.repeats_total, sites = repeats.join(", ")) });
    out.push(match s.by_equipe_percent() {
        Some(percent) => t!("logins.signed_in", count = s.signed_in, equipe = s.signed_in_by_equipe, percent = percent),
        None => t!("logins.signed_in_none").to_owned(),
    });
    if !s.resolved.is_empty() {
        let steps: Vec<String> = s.resolved.iter().map(|(step, n)| format!("{} {n}", step.replace('_', " "))).collect();
        out.push(t!("logins.autopilot", steps = steps.join(", "), minutes = s.minutes_saved));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::logins::{BY_EQUIPE, BY_PERSON, Entry, Kind};

    #[test]
    fn the_report_names_sites_causes_repeats_and_equipes_share() {
        assert!(text("7d", &Stats::default()).contains("7d"));
        let t0 = 1_794_000_000;
        let rows = [
            Entry::new(t0, "github.com", "qa-2", Kind::Handoff, Some(Cause::SessionExpired), None),
            Entry::new(t0 + 60, "github.com", "backend", Kind::Handoff, Some(Cause::SessionExpired), None),
            Entry::new(t0 + 120, "github.com", "backend", Kind::SignedIn, None, Some(BY_PERSON)),
            Entry::new(t0 + 7_200, "github.com", "frontend", Kind::SignedIn, None, Some(BY_EQUIPE)),
        ];
        let report = text("7d", &logins::summarize(&rows));
        assert!(report.contains("github.com"), "{report}");
        assert!(report.contains("session expired 2"), "{report}");
        assert!(report.contains("1 (github.com 1)") || report.contains("github.com 1"), "{report}");
        assert!(report.contains("50%"), "{report}");
        assert!(report.contains("relogin 1"), "equipe's own steps: {report}");
    }
}
