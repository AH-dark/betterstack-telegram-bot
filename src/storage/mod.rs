use std::time::Duration;

use async_trait::async_trait;

use crate::domain::incident::{IncidentRecord, IncidentStatus};
use crate::error::Result;

pub mod memory;

/// Guard returned by try_lock; dropping it releases the lock.
pub struct LockGuard {
    pub incident_id: String,
    pub token: String,
}

#[async_trait]
pub trait IncidentStore: Send + Sync {
    /// Get an incident record by ID.
    async fn get(&self, id: &str) -> Result<Option<IncidentRecord>>;

    /// Insert or update an incident record.
    async fn upsert(&self, rec: &IncidentRecord) -> Result<()>;

    /// Set the Telegram message coordinates after sending.
    async fn set_message_id(&self, id: &str, chat_id: i64, message_id: i32) -> Result<()>;

    /// Update the status (and optional actor/timestamp) of an incident.
    async fn set_status(
        &self,
        id: &str,
        status: IncidentStatus,
        actor: Option<&str>,
        at: &str,
    ) -> Result<()>;

    /// Returns true if this (id, event) was NOT seen before (i.e. we should process it).
    /// Uses SET NX EX semantics: first call returns true, subsequent calls return false.
    async fn mark_event_once(&self, id: &str, event: &str, ttl: Duration) -> Result<bool>;

    /// Try to acquire a short per-incident lock.
    /// Returns Some(LockGuard) if acquired, None if already locked.
    async fn try_lock(&self, id: &str, ttl: Duration) -> Result<Option<LockGuard>>;
}
