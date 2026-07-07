use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;

use super::{IncidentStore, LockGuard};
use crate::domain::incident::{IncidentRecord, IncidentStatus};
use crate::error::{AppError, Result};

const LOCK_RELEASE_SCRIPT: &str = r#"
if redis.call("get", KEYS[1]) == ARGV[1] then
    return redis.call("del", KEYS[1])
else
    return 0
end
"#;

#[derive(Clone)]
pub struct RedisStore {
    conn: ConnectionManager,
    prefix: Arc<String>,
    incident_ttl: Duration,
}

impl RedisStore {
    pub async fn new(redis_url: &str, prefix: String, incident_ttl: Duration) -> Result<Self> {
        let client = redis::Client::open(redis_url).map_err(AppError::Redis)?;
        let conn = ConnectionManager::new(client)
            .await
            .map_err(AppError::Redis)?;

        Ok(Self {
            conn,
            prefix: Arc::new(prefix),
            incident_ttl,
        })
    }

    fn incident_key(&self, id: &str) -> String {
        format!("{}:incident:{}", self.prefix, id)
    }

    fn dedup_key(&self, id: &str, event: &str) -> String {
        format!("{}:dedup:{}:{}", self.prefix, id, event)
    }

    fn lock_key(&self, id: &str) -> String {
        format!("{}:lock:{}", self.prefix, id)
    }
}

fn record_to_fields(rec: &IncidentRecord) -> Result<Vec<(String, String)>> {
    let mut fields = vec![
        ("id".to_string(), rec.id.clone()),
        ("name".to_string(), rec.name.clone()),
        ("url".to_string(), rec.url.clone()),
        ("cause".to_string(), rec.cause.clone()),
        (
            "status".to_string(),
            serde_json::to_string(&rec.status)
                .map_err(|err| AppError::Internal(err.to_string()))?,
        ),
    ];

    push_optional_field(&mut fields, "chat_id", rec.chat_id);
    push_optional_field(&mut fields, "message_id", rec.message_id);
    push_optional_string(&mut fields, "started_at", rec.started_at.as_deref());
    push_optional_string(
        &mut fields,
        "acknowledged_at",
        rec.acknowledged_at.as_deref(),
    );
    push_optional_string(
        &mut fields,
        "acknowledged_by",
        rec.acknowledged_by.as_deref(),
    );
    push_optional_string(&mut fields, "resolved_at", rec.resolved_at.as_deref());
    push_optional_string(&mut fields, "resolved_by", rec.resolved_by.as_deref());

    Ok(fields)
}

fn push_optional_field<T: ToString>(
    fields: &mut Vec<(String, String)>,
    name: &str,
    value: Option<T>,
) {
    if let Some(value) = value {
        fields.push((name.to_string(), value.to_string()));
    }
}

fn push_optional_string(fields: &mut Vec<(String, String)>, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        fields.push((name.to_string(), value.to_string()));
    }
}

fn fields_to_record(fields: HashMap<String, String>) -> Result<IncidentRecord> {
    let status = fields
        .get("status")
        .map(|status| serde_json::from_str(status))
        .transpose()
        .map_err(|err| AppError::Internal(err.to_string()))?
        .unwrap_or_default();

    Ok(IncidentRecord {
        id: fields.get("id").cloned().unwrap_or_default(),
        name: fields.get("name").cloned().unwrap_or_default(),
        url: fields.get("url").cloned().unwrap_or_default(),
        cause: fields.get("cause").cloned().unwrap_or_default(),
        status,
        chat_id: parse_optional_field(&fields, "chat_id")?,
        message_id: parse_optional_field(&fields, "message_id")?,
        started_at: fields.get("started_at").cloned(),
        acknowledged_at: fields.get("acknowledged_at").cloned(),
        acknowledged_by: fields.get("acknowledged_by").cloned(),
        resolved_at: fields.get("resolved_at").cloned(),
        resolved_by: fields.get("resolved_by").cloned(),
    })
}

fn parse_optional_field<T>(fields: &HashMap<String, String>, name: &str) -> Result<Option<T>>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    fields
        .get(name)
        .map(|value| {
            value
                .parse()
                .map_err(|err| AppError::Internal(format!("invalid {name} field: {err}")))
        })
        .transpose()
}

fn ttl_secs(ttl: Duration) -> u64 {
    ttl.as_secs().max(1)
}

fn lock_token() -> Result<String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().to_string())
        .map_err(|err| AppError::Internal(format!("system time before unix epoch: {err}")))
}

#[async_trait]
impl IncidentStore for RedisStore {
    async fn get(&self, id: &str) -> Result<Option<IncidentRecord>> {
        let key = self.incident_key(id);
        let mut conn = self.conn.clone();
        let fields: HashMap<String, String> = conn.hgetall(key).await.map_err(AppError::Redis)?;

        if fields.is_empty() {
            return Ok(None);
        }

        fields_to_record(fields).map(Some)
    }

    async fn upsert(&self, rec: &IncidentRecord) -> Result<()> {
        let key = self.incident_key(&rec.id);
        let fields = record_to_fields(rec)?;
        let mut conn = self.conn.clone();

        redis::pipe()
            .atomic()
            .hset_multiple(&key, &fields)
            .expire(&key, ttl_secs(self.incident_ttl) as i64)
            .query_async::<()>(&mut conn)
            .await
            .map_err(AppError::Redis)?;

        Ok(())
    }

    async fn set_message_id(&self, id: &str, chat_id: i64, message_id: i32) -> Result<()> {
        let key = self.incident_key(id);
        let fields = [
            ("chat_id", chat_id.to_string()),
            ("message_id", message_id.to_string()),
        ];
        let mut conn = self.conn.clone();

        conn.hset_multiple(key, &fields)
            .await
            .map_err(AppError::Redis)
    }

    async fn set_status(
        &self,
        id: &str,
        status: IncidentStatus,
        actor: Option<&str>,
        at: &str,
    ) -> Result<()> {
        let key = self.incident_key(id);
        let mut fields = vec![(
            "status".to_string(),
            serde_json::to_string(&status).map_err(|err| AppError::Internal(err.to_string()))?,
        )];

        match status {
            IncidentStatus::Started => {}
            IncidentStatus::Acknowledged => {
                fields.push(("acknowledged_at".to_string(), at.to_string()));
                push_optional_string(&mut fields, "acknowledged_by", actor);
            }
            IncidentStatus::Resolved => {
                fields.push(("resolved_at".to_string(), at.to_string()));
                push_optional_string(&mut fields, "resolved_by", actor);
            }
        }

        let mut conn = self.conn.clone();
        conn.hset_multiple(key, &fields)
            .await
            .map_err(AppError::Redis)
    }

    async fn mark_event_once(&self, id: &str, event: &str, ttl: Duration) -> Result<bool> {
        let key = self.dedup_key(id, event);
        let mut conn = self.conn.clone();

        redis::cmd("SET")
            .arg(key)
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(ttl_secs(ttl))
            .query_async(&mut conn)
            .await
            .map_err(AppError::Redis)
    }

    async fn try_lock(&self, id: &str, ttl: Duration) -> Result<Option<LockGuard>> {
        let key = self.lock_key(id);
        let token = lock_token()?;
        let mut conn = self.conn.clone();
        let acquired: bool = redis::cmd("SET")
            .arg(key)
            .arg(&token)
            .arg("NX")
            .arg("EX")
            .arg(ttl_secs(ttl))
            .query_async(&mut conn)
            .await
            .map_err(AppError::Redis)?;

        Ok(acquired.then(|| LockGuard {
            incident_id: id.to_string(),
            token,
        }))
    }
}

impl RedisStore {
    pub async fn release_lock(&self, guard: &LockGuard) -> Result<bool> {
        let key = self.lock_key(&guard.incident_id);
        let mut conn = self.conn.clone();
        let deleted: i32 = redis::Script::new(LOCK_RELEASE_SCRIPT)
            .key(key)
            .arg(&guard.token)
            .invoke_async(&mut conn)
            .await
            .map_err(AppError::Redis)?;

        Ok(deleted == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REDIS_URL: &str = "redis://127.0.0.1:6379";

    fn make_record(id: &str) -> IncidentRecord {
        IncidentRecord {
            id: id.to_string(),
            name: "Test Incident".to_string(),
            url: "https://example.com/incidents/123".to_string(),
            cause: "Test cause".to_string(),
            status: IncidentStatus::Started,
            started_at: Some("2026-07-07T00:00:00Z".to_string()),
            ..Default::default()
        }
    }

    async fn test_store(test_name: &str) -> RedisStore {
        let prefix = format!("test:redis_store:{test_name}");
        let store = RedisStore::new(REDIS_URL, prefix.clone(), Duration::from_secs(60))
            .await
            .unwrap();
        let mut conn = store.conn.clone();
        for key in [
            format!("{prefix}:incident:incident-1"),
            format!("{prefix}:dedup:incident-1:incident_started"),
            format!("{prefix}:lock:incident-1"),
        ] {
            redis::cmd("DEL")
                .arg(key)
                .query_async::<()>(&mut conn)
                .await
                .unwrap();
        }
        store
    }

    #[tokio::test]
    #[ignore = "requires local Redis on 127.0.0.1:6379"]
    async fn upsert_get_dedup_and_lock_round_trip() {
        let store = test_store("round_trip").await;
        let rec = make_record("incident-1");

        store.upsert(&rec).await.unwrap();
        let got = store.get("incident-1").await.unwrap().unwrap();
        assert_eq!(got.id, rec.id);
        assert_eq!(got.name, rec.name);
        assert_eq!(got.url, rec.url);
        assert_eq!(got.cause, rec.cause);
        assert_eq!(got.status, rec.status);
        assert_eq!(got.started_at, rec.started_at);

        assert!(store
            .mark_event_once("incident-1", "incident_started", Duration::from_secs(60))
            .await
            .unwrap());
        assert!(!store
            .mark_event_once("incident-1", "incident_started", Duration::from_secs(60))
            .await
            .unwrap());

        let first = store
            .try_lock("incident-1", Duration::from_secs(60))
            .await
            .unwrap();
        assert!(first.is_some());

        let second = store
            .try_lock("incident-1", Duration::from_secs(60))
            .await
            .unwrap();
        assert!(second.is_none());

        assert!(store.release_lock(&first.unwrap()).await.unwrap());
        assert!(store
            .try_lock("incident-1", Duration::from_secs(60))
            .await
            .unwrap()
            .is_some());
    }
}
