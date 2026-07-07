use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use betterstack_telegram_bot::betterstack::client::BetterStackClient;
use betterstack_telegram_bot::betterstack::webhook::WebhookAppState;
use betterstack_telegram_bot::domain::incident::{IncidentRecord, IncidentStatus};
use betterstack_telegram_bot::server::build_router;
use betterstack_telegram_bot::storage::IncidentStore;
use betterstack_telegram_bot::storage::memory::MemoryStore;
use betterstack_telegram_bot::telegram::handler::callback_handler;
use betterstack_telegram_bot::telegram::{FakeNotifier, Notifier, NotifierCall};
use teloxide::prelude::*;
use teloxide::requests::RequesterExt;
use teloxide::types::{CallbackQuery, CallbackQueryId, User, UserId};
use tower::ServiceExt;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SECRET: &str = "betterstack-secret";
const STARTED_FIXTURE: &str = include_str!("fixtures/incident_started.json");

fn build_test_app(secret: &str, notifier: Arc<FakeNotifier>) -> axum::Router {
    let store = Arc::new(MemoryStore::new()) as Arc<dyn IncidentStore>;
    build_test_app_with_store(secret, store, notifier)
}

fn build_test_app_with_store(
    secret: &str,
    store: Arc<dyn IncidentStore>,
    notifier: Arc<FakeNotifier>,
) -> axum::Router {
    let state = WebhookAppState {
        store,
        notifier: notifier as Arc<dyn Notifier>,
        webhook_secret: secret.to_string(),
    };
    build_router(axum::Router::new(), state, "/webhooks/betterstack")
}

fn webhook_request(secret: Option<&str>, body: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/webhooks/betterstack")
        .header("Content-Type", "application/json");

    if let Some(secret) = secret {
        builder = builder.header("X-Webhook-Secret", secret);
    }

    builder
        .body(Body::from(body.to_string()))
        .expect("request should build")
}

fn make_callback_query(data: &str) -> CallbackQuery {
    CallbackQuery {
        id: CallbackQueryId("test-query-id".to_string()),
        from: User {
            id: UserId(999),
            is_bot: false,
            first_name: "Alice".to_string(),
            last_name: None,
            username: Some("alice".to_string()),
            language_code: None,
            is_premium: false,
            added_to_attachment_menu: false,
        },
        message: None,
        inline_message_id: None,
        chat_instance: String::new(),
        data: Some(data.to_string()),
        game_short_name: None,
    }
}

async fn mount_answer_callback_stub(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path_regex("(?i)^/bot[^/]+/answercallbackquery$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": true
        })))
        .mount(server)
        .await;
}

fn test_bot(server: &MockServer) -> teloxide::adaptors::Throttle<Bot> {
    Bot::new("TESTTOKEN")
        .set_api_url(reqwest::Url::parse(&server.uri()).expect("valid Telegram mock URL"))
        .throttle(Default::default())
}

fn betterstack_client(server: &MockServer) -> BetterStackClient {
    BetterStackClient::new(
        reqwest::Url::parse(&server.uri()).expect("valid Better Stack mock URL"),
        "betterstack-token".to_string(),
    )
}

async fn prepopulate_started(store: &MemoryStore) {
    let mut rec = IncidentRecord::new("12345");
    rec.name = "Homepage down".to_string();
    rec.url = "https://example.com".to_string();
    rec.cause = "Status 500".to_string();
    rec.status = IncidentStatus::Started;
    rec.chat_id = Some(-100123);
    rec.message_id = Some(42);
    store.upsert(&rec).await.expect("record should upsert");
}

#[tokio::test]
async fn webhook_started_sends_incident() {
    let notifier = Arc::new(FakeNotifier::new(-100123, 42));
    let app = build_test_app(SECRET, notifier.clone());

    let resp = app
        .oneshot(webhook_request(Some(SECRET), STARTED_FIXTURE))
        .await
        .expect("handler should respond");

    assert_eq!(resp.status(), StatusCode::OK);
    let calls = notifier.recorded_calls();
    assert_eq!(calls.len(), 1);
    match &calls[0] {
        NotifierCall::Send {
            incident_id,
            text,
            button_count,
        } => {
            assert_eq!(incident_id, "12345");
            assert!(text.contains("Homepage down"));
            assert_eq!(*button_count, 2);
        }
        call => panic!("expected Send call, got {call:?}"),
    }
}

#[tokio::test]
async fn webhook_bad_secret_is_rejected_without_notifier_call() {
    let notifier = Arc::new(FakeNotifier::new(-100123, 42));
    let app = build_test_app(SECRET, notifier.clone());

    let resp = app
        .oneshot(webhook_request(Some("wrong-secret"), STARTED_FIXTURE))
        .await
        .expect("handler should respond");

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(notifier.recorded_calls().is_empty());
}

#[tokio::test]
async fn webhook_started_is_idempotent() {
    let store = Arc::new(MemoryStore::new()) as Arc<dyn IncidentStore>;
    let notifier = Arc::new(FakeNotifier::new(-100123, 42));
    let app = build_test_app_with_store(SECRET, store, notifier.clone());

    let first = app
        .clone()
        .oneshot(webhook_request(Some(SECRET), STARTED_FIXTURE))
        .await
        .expect("first webhook should respond");
    let second = app
        .oneshot(webhook_request(Some(SECRET), STARTED_FIXTURE))
        .await
        .expect("second webhook should respond");

    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    let send_count = notifier
        .recorded_calls()
        .iter()
        .filter(|call| matches!(call, NotifierCall::Send { .. }))
        .count();
    assert_eq!(send_count, 1);
}

#[tokio::test]
async fn callback_acknowledge_syncs_betterstack_store_and_message() {
    let betterstack_mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v3/incidents/12345/acknowledge"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&betterstack_mock)
        .await;
    let telegram_mock = MockServer::start().await;
    mount_answer_callback_stub(&telegram_mock).await;

    let store = Arc::new(MemoryStore::new());
    prepopulate_started(&store).await;
    let notifier = Arc::new(FakeNotifier::new(-100123, 42));

    callback_handler(
        test_bot(&telegram_mock),
        make_callback_query("ack:12345"),
        store.clone() as Arc<dyn IncidentStore>,
        notifier.clone() as Arc<dyn Notifier>,
        betterstack_client(&betterstack_mock),
    )
    .await
    .expect("callback should succeed");

    let betterstack_requests = betterstack_mock
        .received_requests()
        .await
        .expect("request recording should be enabled");
    assert_eq!(betterstack_requests.len(), 1);
    assert_eq!(
        betterstack_requests[0]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer betterstack-token")
    );
    let telegram_requests = telegram_mock
        .received_requests()
        .await
        .expect("request recording should be enabled");
    assert_eq!(telegram_requests.len(), 1);
    let body = String::from_utf8_lossy(&betterstack_requests[0].body);
    assert!(body.contains("acknowledged_by"));
    assert!(body.contains("alice"));

    let rec = store
        .get("12345")
        .await
        .expect("store read should succeed")
        .expect("record should exist");
    assert_eq!(rec.status, IncidentStatus::Acknowledged);

    let calls = notifier.recorded_calls();
    assert_eq!(calls.len(), 1);
    match &calls[0] {
        NotifierCall::Edit {
            chat_id,
            message_id,
            incident_id,
            button_count,
            ..
        } => {
            assert_eq!(*chat_id, -100123);
            assert_eq!(*message_id, 42);
            assert_eq!(incident_id, "12345");
            assert_eq!(*button_count, 1);
        }
        call => panic!("expected Edit call, got {call:?}"),
    }
}

#[tokio::test]
async fn callback_acknowledge_409_is_idempotent_success() {
    let betterstack_mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v3/incidents/12345/acknowledge"))
        .respond_with(ResponseTemplate::new(409))
        .mount(&betterstack_mock)
        .await;
    let telegram_mock = MockServer::start().await;
    mount_answer_callback_stub(&telegram_mock).await;

    let store = Arc::new(MemoryStore::new());
    prepopulate_started(&store).await;
    let notifier = Arc::new(FakeNotifier::new(-100123, 42));

    callback_handler(
        test_bot(&telegram_mock),
        make_callback_query("ack:12345"),
        store.clone() as Arc<dyn IncidentStore>,
        notifier.clone() as Arc<dyn Notifier>,
        betterstack_client(&betterstack_mock),
    )
    .await
    .expect("callback should succeed");

    let betterstack_requests = betterstack_mock
        .received_requests()
        .await
        .expect("request recording should be enabled");
    assert_eq!(betterstack_requests.len(), 1);
    let rec = store
        .get("12345")
        .await
        .expect("store read should succeed")
        .expect("record should exist");
    assert_eq!(rec.status, IncidentStatus::Acknowledged);
    assert!(matches!(
        notifier.recorded_calls().as_slice(),
        [NotifierCall::Edit {
            button_count: 1,
            ..
        }]
    ));
}

#[tokio::test]
async fn malformed_callback_is_answered_without_betterstack_or_edit() {
    let betterstack_mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v3/incidents/12345/acknowledge"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&betterstack_mock)
        .await;
    let telegram_mock = MockServer::start().await;
    mount_answer_callback_stub(&telegram_mock).await;

    let store = Arc::new(MemoryStore::new());
    let notifier = Arc::new(FakeNotifier::new(-100123, 42));

    callback_handler(
        test_bot(&telegram_mock),
        make_callback_query("garbage"),
        store as Arc<dyn IncidentStore>,
        notifier.clone() as Arc<dyn Notifier>,
        betterstack_client(&betterstack_mock),
    )
    .await
    .expect("malformed callback should still succeed");

    let telegram_requests = telegram_mock
        .received_requests()
        .await
        .expect("request recording should be enabled");
    assert_eq!(telegram_requests.len(), 1);
    let betterstack_requests = betterstack_mock
        .received_requests()
        .await
        .expect("request recording should be enabled");
    assert!(betterstack_requests.is_empty());
    assert!(notifier.recorded_calls().is_empty());
}

#[tokio::test]
async fn callback_api_failure_answers_once_without_editing() {
    let betterstack_mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v3/incidents/12345/acknowledge"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&betterstack_mock)
        .await;
    let telegram_mock = MockServer::start().await;
    mount_answer_callback_stub(&telegram_mock).await;

    let store = Arc::new(MemoryStore::new());
    prepopulate_started(&store).await;
    let notifier = Arc::new(FakeNotifier::new(-100123, 42));

    callback_handler(
        test_bot(&telegram_mock),
        make_callback_query("ack:12345"),
        store.clone() as Arc<dyn IncidentStore>,
        notifier.clone() as Arc<dyn Notifier>,
        betterstack_client(&betterstack_mock),
    )
    .await
    .expect("callback should succeed");

    let telegram_requests = telegram_mock
        .received_requests()
        .await
        .expect("request recording should be enabled");
    assert_eq!(telegram_requests.len(), 1);
    assert!(notifier.recorded_calls().is_empty());
    let rec = store
        .get("12345")
        .await
        .expect("store read should succeed")
        .expect("record should exist");
    assert_eq!(rec.status, IncidentStatus::Started);
}
