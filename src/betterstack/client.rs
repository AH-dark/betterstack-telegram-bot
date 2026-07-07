use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::time::Duration;
use url::Url;

use crate::error::{AppError, Result};

/// Client for the Better Stack Uptime API.
#[derive(Clone)]
pub struct BetterStackClient {
    http: Client,
    base_url: Url,
    token: String,
}

impl BetterStackClient {
    pub fn new(base_url: Url, token: String) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");

        Self {
            http,
            base_url: normalize_base_url(base_url),
            token,
        }
    }

    /// Acknowledge an incident. 409 is treated as success (already acknowledged).
    pub async fn acknowledge(&self, incident_id: &str, by: Option<&str>) -> Result<()> {
        let url = self.incident_action_url(incident_id, "acknowledge")?;
        let body = by
            .map(|name| json!({ "acknowledged_by": name }))
            .unwrap_or_else(|| json!({}));

        self.post_with_retry(url, body).await
    }

    /// Resolve an incident. 409 is treated as success (already resolved).
    pub async fn resolve(&self, incident_id: &str, by: Option<&str>) -> Result<()> {
        let url = self.incident_action_url(incident_id, "resolve")?;
        let body = by
            .map(|name| json!({ "resolved_by": name }))
            .unwrap_or_else(|| json!({}));

        self.post_with_retry(url, body).await
    }

    fn incident_action_url(&self, incident_id: &str, action: &str) -> Result<Url> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|_| AppError::Internal("base url cannot be a base".into()))?
            .pop_if_empty()
            .extend(["api", "v3", "incidents", incident_id, action]);
        Ok(url)
    }

    async fn post_with_retry(&self, url: Url, body: Value) -> Result<()> {
        const MAX_RETRIES: u32 = 2;

        let mut attempt = 0;
        loop {
            let resp = self
                .http
                .post(url.clone())
                .bearer_auth(&self.token)
                .json(&body)
                .send()
                .await
                .map_err(AppError::Reqwest)?;

            let status = resp.status();

            if status.is_success() || status == StatusCode::CONFLICT {
                return Ok(());
            }

            if should_retry(status) && attempt < MAX_RETRIES {
                attempt += 1;
                tokio::time::sleep(Duration::from_secs(u64::from(attempt))).await;
                continue;
            }

            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(format!(
                "Better Stack API returned {status}: {body}"
            )));
        }
    }
}

fn normalize_base_url(mut base_url: Url) -> Url {
    if !base_url.path().ends_with('/') {
        let path = format!("{}/", base_url.path());
        base_url.set_path(&path);
    }

    base_url
}

fn should_retry(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn make_client(server: &MockServer) -> BetterStackClient {
        let base_url: Url = server.uri().parse().expect("valid mock server URL");
        BetterStackClient::new(base_url, "test-token".to_string())
    }

    #[tokio::test]
    async fn acknowledge_sends_correct_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/incidents/12345/acknowledge"))
            .and(header("Authorization", "Bearer test-token"))
            .and(body_json(json!({ "acknowledged_by": "alice" })))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = make_client(&server).await;
        client.acknowledge("12345", Some("alice")).await.unwrap();
    }

    #[tokio::test]
    async fn resolve_sends_correct_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/incidents/12345/resolve"))
            .and(header("Authorization", "Bearer test-token"))
            .and(body_json(json!({ "resolved_by": "bob" })))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = make_client(&server).await;
        client.resolve("12345", Some("bob")).await.unwrap();
    }

    #[tokio::test]
    async fn acknowledge_409_is_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/incidents/12345/acknowledge"))
            .and(body_json(json!({})))
            .respond_with(ResponseTemplate::new(409))
            .mount(&server)
            .await;

        let client = make_client(&server).await;
        client.acknowledge("12345", None).await.unwrap();
    }

    #[tokio::test]
    async fn resolve_409_is_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/incidents/12345/resolve"))
            .and(body_json(json!({})))
            .respond_with(ResponseTemplate::new(409))
            .mount(&server)
            .await;

        let client = make_client(&server).await;
        client.resolve("12345", None).await.unwrap();
    }

    #[tokio::test]
    async fn server_error_surfaces_as_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/incidents/12345/acknowledge"))
            .respond_with(ResponseTemplate::new(500).set_body_string("upstream failed"))
            .mount(&server)
            .await;

        let client = make_client(&server).await;
        let result = client.acknowledge("12345", None).await;

        assert!(result.is_err(), "500 should surface as error after retries");
        assert!(
            result
                .expect_err("expected Better Stack API error")
                .to_string()
                .contains("Better Stack API returned 500")
        );
    }

    #[test]
    fn new_normalizes_base_url_with_trailing_slash() {
        let base_url: Url = "https://uptime.example.test/api"
            .parse()
            .expect("valid URL");
        let client = BetterStackClient::new(base_url, "test-token".to_string());

        assert_eq!(client.base_url.as_str(), "https://uptime.example.test/api/");
    }

    #[test]
    fn incident_action_url_percent_encodes_incident_id_path_segment() {
        let base_url: Url = "https://uptime.example.test".parse().expect("valid URL");
        let client = BetterStackClient::new(base_url, "test-token".to_string());

        let url = client
            .incident_action_url("abc/def?x=1", "resolve")
            .expect("url should build");

        assert_eq!(
            url.as_str(),
            "https://uptime.example.test/api/v3/incidents/abc%2Fdef%3Fx=1/resolve"
        );
    }
}
