//! Anonymous usage events, sent only when the person said yes
//! (docs/TELEMETRY.md): whether they did, and the switch a screen flips —
//! `telemetry` (request) → [`TelemetryStatus`], `telemetry_set` (request)
//! [`SetTelemetry`] → [`TelemetryStatus`].

use serde::{Deserialize, Serialize};

/// What the person said about sending usage events.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Consent {
    /// Not asked yet: nothing is sent. A screen asks on its first run.
    #[default]
    Unset,
    On,
    Off,
}

/// Where telemetry stands on the daemon's machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetryStatus {
    pub consent: Consent,
    /// Events go out now: the person said yes and nothing holds them back.
    pub sending: bool,
    /// What holds them back although the person said yes: `EQUIPE_TELEMETRY`
    /// or `CI`, the environment the daemon runs in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_by: Option<String>,
}

/// The person's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetTelemetry {
    pub on: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_is_a_plain_word_on_the_wire() {
        assert_eq!(serde_json::to_string(&Consent::Unset).unwrap(), "\"unset\"");
        let status = TelemetryStatus { consent: Consent::On, sending: true, held_by: None };
        assert_eq!(serde_json::to_value(&status).unwrap(), serde_json::json!({ "consent": "on", "sending": true }));
    }
}
