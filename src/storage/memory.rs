use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use super::{IncidentStore, LockGuard};
use crate::domain::incident::{IncidentRecord, IncidentStatus};
use crate::error::{AppError, Result};

struct DedupEntry {
    expires_at: Instant,
}

struct LockEntry {
    token: String,
    expires_at: Instant,
}

#[derive(Default)]
struct MemoryStoreInner {
    incidents: HashMap<String, IncidentRecord>,
    dedup: HashMap<String, DedupEntry>,
    locks: HashMap<String, LockEntry>,
}

/// In-memory implementation of IncidentStore for testing.
#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<MemoryStoreInner>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock_inner(&self) -> Result<std::sync::MutexGuard<'_, MemoryStoreInner>> {
        self.inner
            .lock()
            .map_err(|_| AppError::Internal("lock poisoned".into()))
    }
}

#[async_trait]
impl IncidentStore for MemoryStore {
    async fn get(&self, id: &str) -> Result<Option<IncidentRecord>> {
        let inner = self.lock_inner()?;
        Ok(inner.incidents.get(id).cloned())
    }

    async fn upsert(&self, rec: &IncidentRecord) -> Result<()> {
        let mut inner = self.lock_inner()?;
        inner.incidents.insert(rec.id.clone(), rec.clone());
        Ok(())
    }

    async fn set_message_id(&self, id: &str, chat_id: i64, message_id: i32) -> Result<()> {
        let mut inner = self.lock_inner()?;
        if let Some(rec) = inner.incidents.get_mut(id) {
            rec.chat_id = Some(chat_id);
            rec.message_id = Some(message_id);
        }
        Ok(())
    }

    async fn set_status(
        &self,
        id: &str,
        status: IncidentStatus,
        actor: Option<&str>,
        at: &str,
    ) -> Result<()> {
        let mut inner = self.lock_inner()?;
        if let Some(rec) = inner.incidents.get_mut(id) {
            rec.status = status;
            match status {
                IncidentStatus::Acknowledged => {
                    rec.acknowledged_at = Some(at.to_string());
                    rec.acknowledged_by = actor.map(str::to_string);
                }
                IncidentStatus::Resolved => {
                    rec.resolved_at = Some(at.to_string());
                    rec.resolved_by = actor.map(str::to_string);
                }
                IncidentStatus::Started => {}
            }
        }
        Ok(())
    }

    async fn mark_event_once(&self, id: &str, event: &str, ttl: Duration) -> Result<bool> {
        let mut inner = self.lock_inner()?;
        let key = format!("{id}:{event}");
        let now = Instant::now();

        if inner
            .dedup
            .get(&key)
            .is_some_and(|entry| entry.expires_at > now)
        {
            return Ok(false);
        }

        inner.dedup.insert(
            key,
            DedupEntry {
                expires_at: now + ttl,
            },
        );
        Ok(true)
    }

    async fn try_lock(&self, id: &str, ttl: Duration) -> Result<Option<LockGuard>> {
        let mut inner = self.lock_inner()?;
        let now = Instant::now();

        if let Some(entry) = inner.locks.get(id)
            && entry.expires_at > now
        {
            let _held_token = &entry.token;
            return Ok(None);
        }

        let token = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos().to_string())
            .map_err(|err| AppError::Internal(format!("system time before unix epoch: {err}")))?;

        inner.locks.insert(
            id.to_string(),
            LockEntry {
                token: token.clone(),
                expires_at: now + ttl,
            },
        );

        Ok(Some(LockGuard {
            incident_id: id.to_string(),
            token,
        }))
    }

    async fn release_lock(&self, guard: &LockGuard) -> Result<()> {
        let mut inner = self.lock_inner()?;
        if inner
            .locks
            .get(&guard.incident_id)
            .is_some_and(|entry| entry.token == guard.token)
        {
            inner.locks.remove(&guard.incident_id);
        }

        Ok(())
    }

    async fn unmark_event(&self, id: &str, event: &str) -> Result<()> {
        let mut inner = self.lock_inner()?;
        inner.dedup.remove(&format!("{id}:{event}"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::incident::{IncidentRecord, IncidentStatus};

    fn make_record(id: &str) -> IncidentRecord {
        IncidentRecord {
            id: id.to_string(),
            name: "Test Incident".to_string(),
            url: "https://example.com".to_string(),
            cause: "Test cause".to_string(),
            status: IncidentStatus::Started,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn get_returns_none_for_missing() {
        let store = MemoryStore::new();
        assert!(store.get("nonexistent").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn upsert_and_get_round_trip() {
        let store = MemoryStore::new();
        let rec = make_record("123");
        store.upsert(&rec).await.unwrap();
        let got = store.get("123").await.unwrap().unwrap();
        assert_eq!(got.id, "123");
        assert_eq!(got.name, "Test Incident");
    }

    #[tokio::test]
    async fn set_message_id_updates_record() {
        let store = MemoryStore::new();
        let rec = make_record("456");
        store.upsert(&rec).await.unwrap();
        store.set_message_id("456", -100123, 42).await.unwrap();
        let got = store.get("456").await.unwrap().unwrap();
        assert_eq!(got.chat_id, Some(-100123));
        assert_eq!(got.message_id, Some(42));
    }

    #[tokio::test]
    async fn set_status_acknowledged_updates_fields() {
        let store = MemoryStore::new();
        let rec = make_record("789");
        store.upsert(&rec).await.unwrap();
        store
            .set_status(
                "789",
                IncidentStatus::Acknowledged,
                Some("alice"),
                "2026-01-01T00:00:00Z",
            )
            .await
            .unwrap();
        let got = store.get("789").await.unwrap().unwrap();
        assert_eq!(got.status, IncidentStatus::Acknowledged);
        assert_eq!(got.acknowledged_by, Some("alice".to_string()));
        assert_eq!(
            got.acknowledged_at,
            Some("2026-01-01T00:00:00Z".to_string())
        );
    }

    #[tokio::test]
    async fn set_status_resolved_updates_fields() {
        let store = MemoryStore::new();
        let rec = make_record("790");
        store.upsert(&rec).await.unwrap();
        store
            .set_status(
                "790",
                IncidentStatus::Resolved,
                Some("bob"),
                "2026-01-01T00:01:00Z",
            )
            .await
            .unwrap();
        let got = store.get("790").await.unwrap().unwrap();
        assert_eq!(got.status, IncidentStatus::Resolved);
        assert_eq!(got.resolved_by, Some("bob".to_string()));
        assert_eq!(got.resolved_at, Some("2026-01-01T00:01:00Z".to_string()));
    }

    #[tokio::test]
    async fn mark_event_once_returns_true_first_time() {
        let store = MemoryStore::new();
        let first = store
            .mark_event_once("inc1", "incident_started", Duration::from_secs(86400))
            .await
            .unwrap();
        assert!(first, "first call should return true");
    }

    #[tokio::test]
    async fn mark_event_once_returns_false_second_time() {
        let store = MemoryStore::new();
        let ttl = Duration::from_secs(86400);
        let first = store
            .mark_event_once("inc2", "incident_started", ttl)
            .await
            .unwrap();
        let second = store
            .mark_event_once("inc2", "incident_started", ttl)
            .await
            .unwrap();
        assert!(first, "first call should return true");
        assert!(!second, "second call should return false (dedup)");
    }

    #[tokio::test]
    async fn mark_event_once_different_events_are_independent() {
        let store = MemoryStore::new();
        let ttl = Duration::from_secs(86400);
        let started = store
            .mark_event_once("inc3", "incident_started", ttl)
            .await
            .unwrap();
        let resolved = store
            .mark_event_once("inc3", "incident_resolved", ttl)
            .await
            .unwrap();
        assert!(started);
        assert!(resolved, "different events should be independent");
    }

    #[tokio::test]
    async fn try_lock_acquires_when_free() {
        let store = MemoryStore::new();
        let guard = store
            .try_lock("inc4", Duration::from_secs(10))
            .await
            .unwrap();
        assert!(guard.is_some());
    }

    #[tokio::test]
    async fn try_lock_returns_none_when_held() {
        let store = MemoryStore::new();
        let ttl = Duration::from_secs(10);
        let _guard = store.try_lock("inc5", ttl).await.unwrap();
        let second = store.try_lock("inc5", ttl).await.unwrap();
        assert!(second.is_none(), "second lock attempt should fail");
    }

    #[tokio::test]
    async fn release_lock_allows_reacquire_when_token_matches() {
        let store = MemoryStore::new();
        let ttl = Duration::from_secs(10);
        let guard = store.try_lock("inc6", ttl).await.unwrap().unwrap();

        store.release_lock(&guard).await.unwrap();

        assert!(store.try_lock("inc6", ttl).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn release_lock_keeps_lock_when_token_differs() {
        let store = MemoryStore::new();
        let ttl = Duration::from_secs(10);
        let mut guard = store.try_lock("inc7", ttl).await.unwrap().unwrap();
        guard.token = "wrong-token".to_string();

        store.release_lock(&guard).await.unwrap();

        assert!(store.try_lock("inc7", ttl).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn unmark_event_allows_event_to_be_marked_again() {
        let store = MemoryStore::new();
        let ttl = Duration::from_secs(10);

        assert!(
            store
                .mark_event_once("inc8", "incident_started", ttl)
                .await
                .unwrap()
        );
        assert!(
            !store
                .mark_event_once("inc8", "incident_started", ttl)
                .await
                .unwrap()
        );

        store
            .unmark_event("inc8", "incident_started")
            .await
            .unwrap();

        assert!(
            store
                .mark_event_once("inc8", "incident_started", ttl)
                .await
                .unwrap()
        );
    }
}
