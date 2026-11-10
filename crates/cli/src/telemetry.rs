//! `equipe telemetry on|off|status`: the person's yes or no to anonymous
//! usage events (docs/TELEMETRY.md). It writes the file the daemon reads
//! before every send, so it holds at once, the daemon running or not.

use std::path::Path;

use equipe_core::protocol::telemetry::{Consent, TelemetryStatus};
use equipe_core::t;

const CONTRACT: &str = "https://github.com/equipe-labs/equipe/blob/main/docs/TELEMETRY.md";

pub fn command(home: &Path, word: Option<&str>) -> Result<Vec<String>, String> {
    let status = match word.map(str::trim) {
        None | Some("status") => equipe_core::telemetry::status(home),
        Some("on") => equipe_core::telemetry::set(home, true)?,
        Some("off") => equipe_core::telemetry::set(home, false)?,
        Some(_) => return Err(t!("telemetry.usage").into()),
    };
    Ok(lines(&status))
}

fn lines(status: &TelemetryStatus) -> Vec<String> {
    let mut lines = vec![match status.consent {
        Consent::On => t!("telemetry.on").into(),
        Consent::Off => t!("telemetry.off").into(),
        Consent::Unset => t!("telemetry.unset").into(),
    }];
    if status.consent == Consent::On
        && let Some(var) = &status.held_by
    {
        lines.push(t!("telemetry.held", var = var));
    }
    lines.push(t!("telemetry.what", url = CONTRACT));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_off_and_status_change_and_show_the_file() {
        let home = std::env::temp_dir().join(format!("equipe-cli-telemetry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(equipe_core::telemetry::status(&home).consent, Consent::Unset);
        command(&home, Some("on")).unwrap();
        assert_eq!(equipe_core::telemetry::status(&home).consent, Consent::On);
        command(&home, Some("off")).unwrap();
        assert_eq!(command(&home, None).unwrap().len(), 2);
        assert_eq!(equipe_core::telemetry::status(&home).consent, Consent::Off);
        assert!(command(&home, Some("maybe")).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }
}
