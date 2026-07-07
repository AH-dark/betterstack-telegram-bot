use crate::domain::incident::Trigger;
use serde::Deserialize;

/// Top-level Better Stack outgoing webhook payload.
#[derive(Debug, Deserialize)]
pub struct WebhookPayload {
    pub event: String,
    pub data: IncidentData,
    /// Ignored because relationships are not used in v1.
    #[serde(default)]
    pub relationships: serde_json::Value,
}

impl WebhookPayload {
    /// Extract the Trigger from the event field.
    ///
    /// Returns None for unknown events so the caller can acknowledge and skip.
    pub fn trigger(&self) -> Option<Trigger> {
        Trigger::from_event(&self.event)
    }
}

#[derive(Debug, Deserialize)]
pub struct IncidentData {
    pub id: String,
    #[serde(rename = "type")]
    pub data_type: String,
    pub attributes: IncidentAttributes,
}

/// Incident attributes from Better Stack webhook.
///
/// Optional fields use defaults so missing or null values parse successfully.
#[derive(Debug, Deserialize, Default)]
pub struct IncidentAttributes {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub cause: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub acknowledged_at: Option<String>,
    #[serde(default)]
    pub acknowledged_by: Option<String>,
    #[serde(default)]
    pub resolved_at: Option<String>,
    #[serde(default)]
    pub resolved_by: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> WebhookPayload {
        serde_json::from_str(json).expect("should parse")
    }

    const INCIDENT_STARTED: &str = r#"{
        "event": "incident_started",
        "data": {
            "id": "12345",
            "type": "incident",
            "attributes": {
                "name": "Homepage down",
                "url": "https://example.com",
                "cause": "Status 500",
                "started_at": "2026-01-01T00:00:00Z",
                "acknowledged_at": null,
                "resolved_at": null
            }
        },
        "relationships": { "monitor": { "data": { "id": "4", "type": "monitor" } } }
    }"#;

    const INCIDENT_ACKNOWLEDGED: &str = r#"{
        "event": "incident_acknowledged",
        "data": {
            "id": "12345",
            "type": "incident",
            "attributes": {
                "name": "Homepage down",
                "url": "https://example.com",
                "cause": "Status 500",
                "started_at": "2026-01-01T00:00:00Z",
                "acknowledged_at": "2026-01-01T00:05:00Z",
                "acknowledged_by": "alice",
                "resolved_at": null
            }
        },
        "relationships": {}
    }"#;

    const INCIDENT_RESOLVED: &str = r#"{
        "event": "incident_resolved",
        "data": {
            "id": "12345",
            "type": "incident",
            "attributes": {
                "name": "Homepage down",
                "url": "https://example.com",
                "cause": "Status 500",
                "started_at": "2026-01-01T00:00:00Z",
                "acknowledged_at": "2026-01-01T00:05:00Z",
                "resolved_at": "2026-01-01T00:10:00Z",
                "resolved_by": "bob"
            }
        },
        "relationships": {}
    }"#;

    const INCIDENT_REOPENED: &str = r#"{
        "event": "incident_reopened",
        "data": {
            "id": "12345",
            "type": "incident",
            "attributes": {
                "name": "Homepage down",
                "url": "https://example.com",
                "cause": "Status 500",
                "started_at": "2026-01-01T00:15:00Z"
            }
        },
        "relationships": {}
    }"#;

    const COMMENT_EVENT: &str = r#"{
        "event": "comment",
        "data": {
            "id": "12345",
            "type": "incident",
            "attributes": {
                "name": "Homepage down"
            }
        },
        "relationships": {}
    }"#;

    #[test]
    fn parses_incident_started() {
        let payload = parse(INCIDENT_STARTED);

        assert_eq!(payload.event, "incident_started");
        assert_eq!(payload.data.id, "12345");
        assert_eq!(payload.data.data_type, "incident");
        assert_eq!(payload.data.attributes.name, "Homepage down");
        assert_eq!(payload.data.attributes.url, "https://example.com");
        assert_eq!(payload.data.attributes.cause, "Status 500");
        assert_eq!(
            payload.data.attributes.started_at.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
        assert!(payload.data.attributes.acknowledged_at.is_none());
        assert!(payload.trigger().is_some());
    }

    #[test]
    fn parses_incident_acknowledged() {
        let payload = parse(INCIDENT_ACKNOWLEDGED);

        assert_eq!(payload.event, "incident_acknowledged");
        assert_eq!(
            payload.data.attributes.acknowledged_at.as_deref(),
            Some("2026-01-01T00:05:00Z")
        );
        assert_eq!(
            payload.data.attributes.acknowledged_by.as_deref(),
            Some("alice")
        );
        assert!(payload.trigger().is_some());
    }

    #[test]
    fn parses_incident_resolved() {
        let payload = parse(INCIDENT_RESOLVED);

        assert_eq!(payload.event, "incident_resolved");
        assert_eq!(
            payload.data.attributes.resolved_at.as_deref(),
            Some("2026-01-01T00:10:00Z")
        );
        assert_eq!(payload.data.attributes.resolved_by.as_deref(), Some("bob"));
        assert!(payload.trigger().is_some());
    }

    #[test]
    fn parses_incident_reopened() {
        let payload = parse(INCIDENT_REOPENED);

        assert_eq!(payload.event, "incident_reopened");
        assert!(payload.trigger().is_some());
    }

    #[test]
    fn comment_event_parses_but_trigger_is_none() {
        let payload = parse(COMMENT_EVENT);

        assert_eq!(payload.event, "comment");
        assert!(
            payload.trigger().is_none(),
            "comment event should return None trigger"
        );
    }

    #[test]
    fn unknown_event_parses_without_error() {
        let json = r#"{
            "event": "some_future_event",
            "data": { "id": "1", "type": "incident", "attributes": {} },
            "relationships": {}
        }"#;
        let payload = parse(json);

        assert_eq!(payload.event, "some_future_event");
        assert!(payload.trigger().is_none());
    }

    #[test]
    fn nullable_fields_tolerated() {
        let json = r#"{
            "event": "incident_started",
            "data": {
                "id": "999",
                "type": "incident",
                "attributes": {
                    "name": "Test",
                    "url": "",
                    "cause": "",
                    "started_at": null,
                    "acknowledged_at": null,
                    "resolved_at": null
                }
            },
            "relationships": {}
        }"#;
        let payload = parse(json);

        assert!(payload.data.attributes.started_at.is_none());
        assert!(payload.data.attributes.acknowledged_at.is_none());
    }

    #[test]
    fn extra_attributes_fields_tolerated() {
        let json = r#"{
            "event": "incident_started",
            "data": {
                "id": "1",
                "type": "incident",
                "attributes": {
                    "name": "Test",
                    "url": "https://example.com",
                    "cause": "test",
                    "some_future_field": "value",
                    "another_field": 42
                }
            },
            "relationships": {}
        }"#;
        let payload = parse(json);

        assert_eq!(payload.data.attributes.name, "Test");
    }
}
