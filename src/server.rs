use axum::http::StatusCode;
use axum::{
    routing::{get, post},
    Router,
};
use teloxide::prelude::*;
use tower_http::trace::TraceLayer;

use crate::betterstack::webhook::{betterstack_webhook_handler, WebhookAppState};
use crate::config::Config;
use crate::error::{AppError, Result};

/// Health check handler.
async fn healthz() -> StatusCode {
    StatusCode::OK
}

/// Register the Telegram webhook with the Bot API.
pub async fn register_webhook(bot: &Bot, config: &Config) -> Result<()> {
    let webhook_url = config
        .public_url
        .join(config.telegram_webhook_path.trim_start_matches('/'))
        .map_err(|e| AppError::Config(e.to_string()))?;

    let mut set_webhook = bot.set_webhook(webhook_url).allowed_updates(vec![
        teloxide::types::AllowedUpdate::CallbackQuery,
        teloxide::types::AllowedUpdate::Message,
    ]);

    if let Some(secret) = &config.telegram_secret_token {
        set_webhook = set_webhook.secret_token(secret.expose().to_string());
    }

    set_webhook
        .await
        .map_err(|e| AppError::Telegram(e.to_string()))?;

    tracing::info!("Telegram webhook registered");
    Ok(())
}

/// Build the combined axum Router.
pub fn build_router(
    teloxide_router: Router,
    webhook_state: WebhookAppState,
    betterstack_webhook_path: &str,
) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route(betterstack_webhook_path, post(betterstack_webhook_handler))
        .with_state(webhook_state)
        .merge(teloxide_router)
        .layer(TraceLayer::new_for_http())
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::Body;
    use axum::http::{Method, Request};
    use axum::routing::post;
    use serde_json::json;
    use std::sync::Arc;
    use tower::ServiceExt;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::config::{LogFormat, Secret};
    use crate::storage::memory::MemoryStore;
    use crate::storage::IncidentStore;
    use crate::telegram::{FakeNotifier, Notifier};

    fn test_config(api_base: &str) -> Config {
        Config {
            bot_token: Secret("test-token".to_string()),
            chat_id: -100123,
            public_url: "https://bot.example.com/"
                .parse()
                .expect("valid public URL"),
            telegram_webhook_path: "/telegram/webhook".to_string(),
            telegram_secret_token: Some(Secret("telegram_secret".to_string())),
            betterstack_webhook_path: "/webhooks/betterstack".to_string(),
            betterstack_webhook_secret: Secret("betterstack-secret".to_string()),
            betterstack_api_token: Secret("betterstack-token".to_string()),
            betterstack_api_base: api_base.parse().expect("valid API base URL"),
            redis_url: "redis://localhost:6379/0".to_string(),
            redis_key_prefix: "bstg".to_string(),
            incident_ttl_secs: 2_592_000,
            bind: "127.0.0.1:0".parse().expect("valid bind address"),
            log_format: LogFormat::Json,
            log_level: "info".to_string(),
        }
    }

    fn webhook_state() -> WebhookAppState {
        WebhookAppState {
            store: Arc::new(MemoryStore::new()) as Arc<dyn IncidentStore>,
            notifier: Arc::new(FakeNotifier::new(-100123, 42)) as Arc<dyn Notifier>,
            webhook_secret: "betterstack-secret".to_string(),
        }
    }

    #[tokio::test]
    async fn healthz_returns_200() {
        let teloxide_router = Router::new();
        let app = build_router(teloxide_router, webhook_state(), "/webhooks/betterstack");

        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("handler should respond");

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn build_router_reaches_both_webhook_routes() {
        async fn telegram_handler() -> StatusCode {
            StatusCode::OK
        }

        let teloxide_router = Router::new().route("/telegram/webhook", post(telegram_handler));
        let app = build_router(teloxide_router, webhook_state(), "/webhooks/betterstack");

        let telegram_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/telegram/webhook")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("telegram route should respond");
        assert_eq!(telegram_resp.status(), StatusCode::OK);

        let betterstack_resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/webhooks/betterstack")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("betterstack route should respond");
        assert_eq!(betterstack_resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn register_webhook_sends_correct_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "result": true
            })))
            .mount(&server)
            .await;

        let bot = Bot::new("test-token")
            .set_api_url(reqwest::Url::parse(&server.uri()).expect("valid mock server URL"));
        let config = test_config(&server.uri());

        register_webhook(&bot, &config)
            .await
            .expect("webhook registration should succeed");

        let requests = server
            .received_requests()
            .await
            .expect("request recording should be enabled");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url.path(), "/bottest-token/SetWebhook");

        let body = String::from_utf8_lossy(&requests[0].body);
        assert!(body.contains("name=\"url\""));
        assert!(body.contains("https://bot.example.com/telegram/webhook"));
        assert!(body.contains("name=\"allowed_updates\""));
        assert!(body.contains(r#"["callback_query","message"]"#));
        assert!(body.contains("name=\"secret_token\""));
        assert!(body.contains("telegram_secret"));
    }
}
