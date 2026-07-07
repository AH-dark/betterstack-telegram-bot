use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde::Deserialize;

use crate::domain::incident::{self, IncidentRecord, IncidentStatus, Trigger};
use crate::storage::IncidentStore;
use crate::telegram::Notifier;

#[derive(Clone)]
pub struct WebhookAppState {
    pub store: Arc<dyn IncidentStore>,
    pub notifier: Arc<dyn Notifier>,
    pub webhook_secret: String,
}

/// Constant-time string comparison to prevent timing attacks.
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }

    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

pub async fn betterstack_webhook_handler(
    State(state): State<WebhookAppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let provided_secret = headers
        .get("X-Webhook-Secret")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");

    if !constant_time_eq(provided_secret, &state.webhook_secret) {
        tracing::warn!("rejected webhook: invalid X-Webhook-Secret");
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let payload: WebhookPayload = match serde_json::from_slice(&body) {
        Ok(payload) => payload,
        Err(err) => {
            tracing::warn!(error = %err, "rejected webhook: malformed JSON");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    let trigger = match payload.trigger() {
        Some(trigger) => trigger,
        None => {
            tracing::warn!(event = %payload.event, "ignoring unknown event");
            return StatusCode::OK.into_response();
        }
    };

    let incident_id = &payload.data.id;
    let attrs = &payload.data.attributes;

    let is_new = match state
        .store
        .mark_event_once(incident_id, &payload.event, Duration::from_secs(86_400))
        .await
    {
        Ok(is_new) => is_new,
        Err(err) => {
            tracing::error!(error = %err, "store error in mark_event_once");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if !is_new {
        tracing::debug!(incident_id = %incident_id, event = %payload.event, "deduped webhook");
        return StatusCode::OK.into_response();
    }

    let mut rec = match state.store.get(incident_id).await {
        Ok(Some(rec)) => rec,
        Ok(None) => IncidentRecord {
            id: incident_id.clone(),
            name: attrs.name.clone(),
            url: attrs.url.clone(),
            cause: attrs.cause.clone(),
            status: IncidentStatus::Started,
            started_at: attrs.started_at.clone(),
            ..Default::default()
        },
        Err(err) => {
            tracing::error!(error = %err, "store error in get");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    rec.name = attrs.name.clone();
    rec.url = attrs.url.clone();
    rec.cause = attrs.cause.clone();
    if let Some(value) = &attrs.started_at {
        rec.started_at = Some(value.clone());
    }
    if let Some(value) = &attrs.acknowledged_at {
        rec.acknowledged_at = Some(value.clone());
    }
    if let Some(value) = &attrs.acknowledged_by {
        rec.acknowledged_by = Some(value.clone());
    }
    if let Some(value) = &attrs.resolved_at {
        rec.resolved_at = Some(value.clone());
    }
    if let Some(value) = &attrs.resolved_by {
        rec.resolved_by = Some(value.clone());
    }

    let (new_status, _changed) = incident::next(rec.status, &trigger);
    rec.status = new_status;

    if let Err(err) = state.store.upsert(&rec).await {
        tracing::error!(error = %err, "store error in upsert");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    match (rec.chat_id, rec.message_id) {
        (_, None) => match state.notifier.send_incident(&rec).await {
            Ok((chat_id, message_id)) => {
                if let Err(err) = state
                    .store
                    .set_message_id(incident_id, chat_id, message_id)
                    .await
                {
                    tracing::error!(error = %err, "failed to store message_id");
                }
            }
            Err(err) => {
                tracing::error!(error = %err, "failed to send Telegram message");
            }
        },
        (Some(chat_id), Some(message_id)) => {
            if let Err(err) = state
                .notifier
                .edit_incident(chat_id, message_id, &rec)
                .await
            {
                tracing::error!(error = %err, "failed to edit Telegram message");
            }
        }
        (None, Some(message_id)) => {
            tracing::error!(message_id, "stored incident is missing Telegram chat_id");
        }
    }

    StatusCode::OK.into_response()
}

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

#[cfg(test)]
mod handler_tests {
    use super::*;

    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::post;
    use axum::Router;
    use tower::ServiceExt;

    use crate::storage::memory::MemoryStore;
    use crate::telegram::{FakeNotifier, NotifierCall};

    fn make_app(secret: &str) -> Router {
        let store = Arc::new(MemoryStore::new()) as Arc<dyn IncidentStore>;
        let notifier = Arc::new(FakeNotifier::new(-100123, 42)) as Arc<dyn Notifier>;
        make_router(secret, store, notifier)
    }

    fn make_app_with_parts(
        secret: &str,
        store: Arc<MemoryStore>,
        notifier: Arc<FakeNotifier>,
    ) -> (Router, Arc<MemoryStore>, Arc<FakeNotifier>) {
        let app = make_router(
            secret,
            store.clone() as Arc<dyn IncidentStore>,
            notifier.clone() as Arc<dyn Notifier>,
        );
        (app, store, notifier)
    }

    fn make_router(
        secret: &str,
        store: Arc<dyn IncidentStore>,
        notifier: Arc<dyn Notifier>,
    ) -> Router {
        let state = WebhookAppState {
            store,
            notifier,
            webhook_secret: secret.to_string(),
        };

        Router::new()
            .route("/webhooks/betterstack", post(betterstack_webhook_handler))
            .with_state(state)
    }

    fn webhook_request(secret: Option<&str>, body: &'static str) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/webhooks/betterstack")
            .header("Content-Type", "application/json");

        if let Some(secret) = secret {
            builder = builder.header("X-Webhook-Secret", secret);
        }

        builder
            .body(Body::from(body))
            .expect("request should build")
    }

    const INCIDENT_STARTED_JSON: &str = r#"{
        "event": "incident_started",
        "data": {
            "id": "12345",
            "type": "incident",
            "attributes": {
                "name": "Homepage down",
                "url": "https://example.com",
                "cause": "Status 500",
                "started_at": "2026-01-01T00:00:00Z"
            }
        },
        "relationships": {}
    }"#;

    const INCIDENT_RESOLVED_JSON: &str = r#"{
        "event": "incident_resolved",
        "data": {
            "id": "12345",
            "type": "incident",
            "attributes": {
                "name": "Homepage down",
                "url": "https://example.com",
                "cause": "Status 200",
                "started_at": "2026-01-01T00:00:00Z",
                "resolved_at": "2026-01-01T00:10:00Z",
                "resolved_by": "bob"
            }
        },
        "relationships": {}
    }"#;

    #[tokio::test]
    async fn bad_secret_returns_401() {
        let app = make_app("correct-secret");

        let resp = app
            .oneshot(webhook_request(Some("wrong-secret"), INCIDENT_STARTED_JSON))
            .await
            .expect("handler should respond");

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn missing_secret_returns_401() {
        let app = make_app("correct-secret");

        let resp = app
            .oneshot(webhook_request(None, INCIDENT_STARTED_JSON))
            .await
            .expect("handler should respond");

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn malformed_json_returns_400() {
        let app = make_app("secret");

        let resp = app
            .oneshot(webhook_request(Some("secret"), "not valid json"))
            .await
            .expect("handler should respond");

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unknown_event_returns_200_no_op() {
        let store = Arc::new(MemoryStore::new());
        let notifier = Arc::new(FakeNotifier::new(-100123, 42));
        let (app, store, notifier) = make_app_with_parts("secret", store, notifier);
        let body = r#"{"event":"comment","data":{"id":"1","type":"incident","attributes":{}},"relationships":{}}"#;

        let resp = app
            .oneshot(webhook_request(Some("secret"), body))
            .await
            .expect("handler should respond");

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            store
                .get("1")
                .await
                .expect("store get should succeed")
                .is_none(),
            "unknown event should not create a record"
        );
        assert!(
            notifier.recorded_calls().is_empty(),
            "no Telegram calls for unknown event"
        );
    }

    #[tokio::test]
    async fn valid_started_creates_record_and_sends() {
        let store = Arc::new(MemoryStore::new());
        let notifier = Arc::new(FakeNotifier::new(-100123, 42));
        let (app, store, notifier) = make_app_with_parts("secret", store, notifier);

        let resp = app
            .oneshot(webhook_request(Some("secret"), INCIDENT_STARTED_JSON))
            .await
            .expect("handler should respond");

        assert_eq!(resp.status(), StatusCode::OK);
        let rec = store
            .get("12345")
            .await
            .expect("store get should succeed")
            .expect("record should exist");
        assert_eq!(rec.status, IncidentStatus::Started);
        assert_eq!(rec.message_id, Some(42));
        assert_eq!(rec.chat_id, Some(-100123));

        let calls = notifier.recorded_calls();
        assert_eq!(calls.len(), 1, "should have exactly one Telegram call");
        if let NotifierCall::Send {
            incident_id,
            button_count,
            ..
        } = &calls[0]
        {
            assert_eq!(incident_id, "12345");
            assert_eq!(*button_count, 2, "started status should have 2 buttons");
        } else {
            panic!("expected Send call, got {:?}", calls[0]);
        }
    }

    #[tokio::test]
    async fn valid_resolved_edits_existing_message() {
        let store = Arc::new(MemoryStore::new());
        let notifier = Arc::new(FakeNotifier::new(-100123, 42));
        let (app, store, notifier) = make_app_with_parts("secret", store, notifier);

        let started_resp = app
            .clone()
            .oneshot(webhook_request(Some("secret"), INCIDENT_STARTED_JSON))
            .await
            .expect("handler should respond");
        assert_eq!(started_resp.status(), StatusCode::OK);

        let resolved_resp = app
            .oneshot(webhook_request(Some("secret"), INCIDENT_RESOLVED_JSON))
            .await
            .expect("handler should respond");
        assert_eq!(resolved_resp.status(), StatusCode::OK);

        let rec = store
            .get("12345")
            .await
            .expect("store get should succeed")
            .expect("record should exist");
        assert_eq!(rec.status, IncidentStatus::Resolved);
        assert_eq!(rec.resolved_by.as_deref(), Some("bob"));

        let calls = notifier.recorded_calls();
        assert_eq!(calls.len(), 2, "should send once and edit once");
        assert!(matches!(calls[0], NotifierCall::Send { .. }));
        if let NotifierCall::Edit {
            chat_id,
            message_id,
            incident_id,
            button_count,
            ..
        } = &calls[1]
        {
            assert_eq!(*chat_id, -100123);
            assert_eq!(*message_id, 42);
            assert_eq!(incident_id, "12345");
            assert_eq!(*button_count, 0, "resolved status should have no buttons");
        } else {
            panic!("expected Edit call, got {:?}", calls[1]);
        }
    }
}
