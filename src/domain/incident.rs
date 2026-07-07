use serde::{Deserialize, Serialize};

/// Current status of an incident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IncidentStatus {
    #[default]
    Started,
    Acknowledged,
    Resolved,
}

impl std::fmt::Display for IncidentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IncidentStatus::Started => write!(f, "STARTED"),
            IncidentStatus::Acknowledged => write!(f, "ACKNOWLEDGED"),
            IncidentStatus::Resolved => write!(f, "RESOLVED"),
        }
    }
}

/// Trigger that drives the state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    /// Better Stack webhook: incident_started
    IncidentStarted,
    /// Better Stack webhook: incident_acknowledged OR button click
    Acknowledged,
    /// Better Stack webhook: incident_resolved OR button click
    Resolved,
    /// Better Stack webhook: incident_reopened
    Reopened,
}

impl Trigger {
    /// Parse from Better Stack webhook `event` field.
    pub fn from_event(event: &str) -> Option<Self> {
        match event {
            "incident_started" => Some(Trigger::IncidentStarted),
            "incident_acknowledged" => Some(Trigger::Acknowledged),
            "incident_resolved" => Some(Trigger::Resolved),
            "incident_reopened" => Some(Trigger::Reopened),
            _ => None,
        }
    }
}

/// Persistent record of an incident stored in Redis.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IncidentRecord {
    pub id: String,
    pub name: String,
    pub url: String,
    pub cause: String,
    pub status: IncidentStatus,
    /// Telegram chat ID where the message was sent.
    pub chat_id: Option<i64>,
    /// Telegram message ID for editing.
    pub message_id: Option<i32>,
    pub started_at: Option<String>,
    pub acknowledged_at: Option<String>,
    pub acknowledged_by: Option<String>,
    pub resolved_at: Option<String>,
    pub resolved_by: Option<String>,
}

impl IncidentRecord {
    pub fn new(id: impl Into<String>) -> Self {
        IncidentRecord {
            id: id.into(),
            ..Default::default()
        }
    }
}

/// Pure state machine: given current status and a trigger, return (new_status, changed).
///
/// Rules:
/// - Monotonic-ish with reopen escape hatch
/// - Never regress Resolved -> Acknowledged via late ack
/// - Only incident_reopened moves out of Resolved
pub fn next(current: IncidentStatus, trigger: &Trigger) -> (IncidentStatus, bool) {
    match (current, trigger) {
        (IncidentStatus::Started, Trigger::IncidentStarted) => (IncidentStatus::Started, false),
        (IncidentStatus::Started, Trigger::Acknowledged) => (IncidentStatus::Acknowledged, true),
        (IncidentStatus::Started, Trigger::Resolved) => (IncidentStatus::Resolved, true),
        (IncidentStatus::Started, Trigger::Reopened) => (IncidentStatus::Started, false),

        (IncidentStatus::Acknowledged, Trigger::IncidentStarted) => {
            (IncidentStatus::Acknowledged, false)
        }
        (IncidentStatus::Acknowledged, Trigger::Acknowledged) => {
            (IncidentStatus::Acknowledged, false)
        }
        (IncidentStatus::Acknowledged, Trigger::Resolved) => (IncidentStatus::Resolved, true),
        (IncidentStatus::Acknowledged, Trigger::Reopened) => (IncidentStatus::Started, true),

        (IncidentStatus::Resolved, Trigger::IncidentStarted) => (IncidentStatus::Resolved, false),
        (IncidentStatus::Resolved, Trigger::Acknowledged) => (IncidentStatus::Resolved, false),
        (IncidentStatus::Resolved, Trigger::Resolved) => (IncidentStatus::Resolved, false),
        (IncidentStatus::Resolved, Trigger::Reopened) => (IncidentStatus::Started, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_next(
        current: IncidentStatus,
        trigger: Trigger,
        expected_status: IncidentStatus,
        expected_changed: bool,
    ) {
        let (status, changed) = next(current, &trigger);
        assert_eq!(
            status, expected_status,
            "status mismatch for {current:?} + {trigger:?}"
        );
        assert_eq!(
            changed, expected_changed,
            "changed mismatch for {current:?} + {trigger:?}"
        );
    }

    #[test]
    fn started_plus_started_is_idempotent() {
        assert_next(
            IncidentStatus::Started,
            Trigger::IncidentStarted,
            IncidentStatus::Started,
            false,
        );
    }

    #[test]
    fn started_plus_ack_becomes_acknowledged() {
        assert_next(
            IncidentStatus::Started,
            Trigger::Acknowledged,
            IncidentStatus::Acknowledged,
            true,
        );
    }

    #[test]
    fn started_plus_resolve_becomes_resolved() {
        assert_next(
            IncidentStatus::Started,
            Trigger::Resolved,
            IncidentStatus::Resolved,
            true,
        );
    }

    #[test]
    fn started_plus_reopen_stays_started() {
        assert_next(
            IncidentStatus::Started,
            Trigger::Reopened,
            IncidentStatus::Started,
            false,
        );
    }

    #[test]
    fn acknowledged_plus_started_stays_acknowledged() {
        assert_next(
            IncidentStatus::Acknowledged,
            Trigger::IncidentStarted,
            IncidentStatus::Acknowledged,
            false,
        );
    }

    #[test]
    fn acknowledged_plus_ack_is_idempotent() {
        assert_next(
            IncidentStatus::Acknowledged,
            Trigger::Acknowledged,
            IncidentStatus::Acknowledged,
            false,
        );
    }

    #[test]
    fn acknowledged_plus_resolve_becomes_resolved() {
        assert_next(
            IncidentStatus::Acknowledged,
            Trigger::Resolved,
            IncidentStatus::Resolved,
            true,
        );
    }

    #[test]
    fn acknowledged_plus_reopen_becomes_started() {
        assert_next(
            IncidentStatus::Acknowledged,
            Trigger::Reopened,
            IncidentStatus::Started,
            true,
        );
    }

    #[test]
    fn resolved_plus_started_stays_resolved() {
        assert_next(
            IncidentStatus::Resolved,
            Trigger::IncidentStarted,
            IncidentStatus::Resolved,
            false,
        );
    }

    #[test]
    fn resolved_plus_ack_does_not_regress() {
        assert_next(
            IncidentStatus::Resolved,
            Trigger::Acknowledged,
            IncidentStatus::Resolved,
            false,
        );
    }

    #[test]
    fn resolved_plus_resolve_is_idempotent() {
        assert_next(
            IncidentStatus::Resolved,
            Trigger::Resolved,
            IncidentStatus::Resolved,
            false,
        );
    }

    #[test]
    fn resolved_plus_reopen_becomes_started() {
        assert_next(
            IncidentStatus::Resolved,
            Trigger::Reopened,
            IncidentStatus::Started,
            true,
        );
    }

    #[test]
    fn trigger_from_event_parses_known_events() {
        assert_eq!(
            Trigger::from_event("incident_started"),
            Some(Trigger::IncidentStarted)
        );
        assert_eq!(
            Trigger::from_event("incident_acknowledged"),
            Some(Trigger::Acknowledged)
        );
        assert_eq!(
            Trigger::from_event("incident_resolved"),
            Some(Trigger::Resolved)
        );
        assert_eq!(
            Trigger::from_event("incident_reopened"),
            Some(Trigger::Reopened)
        );
    }

    #[test]
    fn trigger_from_event_returns_none_for_unknown() {
        assert_eq!(Trigger::from_event("comment"), None);
        assert_eq!(Trigger::from_event("unknown_event"), None);
        assert_eq!(Trigger::from_event(""), None);
    }
}
