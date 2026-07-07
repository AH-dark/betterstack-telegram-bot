pub mod dispatch;
pub mod handler;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use teloxide::prelude::*;
use teloxide::types::{ChatId, MessageId, ParseMode};

use crate::domain::incident::IncidentRecord;
use crate::error::{AppError, Result};
use crate::render;

/// Abstraction over Telegram send/edit operations.
/// Allows swapping the real teloxide impl for a recording fake in tests.
#[async_trait]
pub trait Notifier: Send + Sync {
    /// Send a new incident message to the configured chat.
    /// Returns (chat_id, message_id) of the sent message.
    async fn send_incident(&self, rec: &IncidentRecord) -> Result<(i64, i32)>;

    /// Edit an existing incident message in place.
    async fn edit_incident(
        &self,
        chat_id: i64,
        message_id: i32,
        rec: &IncidentRecord,
    ) -> Result<()>;
}

/// Real Telegram notifier backed by teloxide Bot.
pub struct TeloxideNotifier {
    bot: Bot,
    chat_id: i64,
}

impl TeloxideNotifier {
    pub fn new(bot: Bot, chat_id: i64) -> Self {
        Self { bot, chat_id }
    }
}

#[async_trait]
impl Notifier for TeloxideNotifier {
    async fn send_incident(&self, rec: &IncidentRecord) -> Result<(i64, i32)> {
        let (text, keyboard) = render::render(rec);
        let msg = self
            .bot
            .send_message(ChatId(self.chat_id), text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await
            .map_err(|e| AppError::Telegram(e.to_string()))?;

        Ok((self.chat_id, msg.id.0))
    }

    async fn edit_incident(
        &self,
        chat_id: i64,
        message_id: i32,
        rec: &IncidentRecord,
    ) -> Result<()> {
        let (text, keyboard) = render::render(rec);
        let result = self
            .bot
            .edit_message_text(ChatId(chat_id), MessageId(message_id), text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await;

        match result {
            Ok(_) => Ok(()),
            Err(e) => {
                let err_str = e.to_string();
                if is_message_not_modified(&err_str) {
                    tracing::debug!(error = %e, "ignoring unchanged Telegram edit");
                    Ok(())
                } else if is_message_not_found(&err_str) {
                    // v1 skips resend for missing edits; warn so operators can see stale coordinates.
                    tracing::warn!(error = %e, "Telegram message to edit was not found");
                    Ok(())
                } else {
                    Err(AppError::Telegram(err_str))
                }
            }
        }
    }
}

fn is_message_not_modified(error: &str) -> bool {
    error.contains("message is not modified")
}

fn is_message_not_found(error: &str) -> bool {
    error.contains("message to edit not found") || error.contains("MESSAGE_ID_INVALID")
}

/// Recording fake Notifier for tests.
/// Records all calls for later assertion.
#[derive(Debug, Clone)]
pub enum NotifierCall {
    Send {
        incident_id: String,
        text: String,
        button_count: usize,
    },
    Edit {
        chat_id: i64,
        message_id: i32,
        incident_id: String,
        text: String,
        button_count: usize,
    },
}

#[derive(Default, Clone)]
pub struct FakeNotifier {
    pub calls: Arc<Mutex<Vec<NotifierCall>>>,
    /// Fixed (chat_id, message_id) to return from send_incident.
    pub send_returns: (i64, i32),
}

impl FakeNotifier {
    pub fn new(chat_id: i64, message_id: i32) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            send_returns: (chat_id, message_id),
        }
    }

    pub fn recorded_calls(&self) -> Vec<NotifierCall> {
        self.calls
            .lock()
            .expect("fake notifier mutex poisoned")
            .clone()
    }
}

#[async_trait]
impl Notifier for FakeNotifier {
    async fn send_incident(&self, rec: &IncidentRecord) -> Result<(i64, i32)> {
        let (text, keyboard) = render::render(rec);
        let button_count = keyboard.inline_keyboard.iter().map(|row| row.len()).sum();
        self.calls
            .lock()
            .expect("fake notifier mutex poisoned")
            .push(NotifierCall::Send {
                incident_id: rec.id.clone(),
                text,
                button_count,
            });

        Ok(self.send_returns)
    }

    async fn edit_incident(
        &self,
        chat_id: i64,
        message_id: i32,
        rec: &IncidentRecord,
    ) -> Result<()> {
        let (text, keyboard) = render::render(rec);
        let button_count = keyboard.inline_keyboard.iter().map(|row| row.len()).sum();
        self.calls
            .lock()
            .expect("fake notifier mutex poisoned")
            .push(NotifierCall::Edit {
                chat_id,
                message_id,
                incident_id: rec.id.clone(),
                text,
                button_count,
            });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::incident::{IncidentRecord, IncidentStatus};

    fn make_record(id: &str, status: IncidentStatus) -> IncidentRecord {
        IncidentRecord {
            id: id.to_string(),
            name: "Test Incident".to_string(),
            url: "https://example.com".to_string(),
            cause: "Test cause".to_string(),
            status,
            started_at: Some("2026-01-01T00:00:00Z".to_string()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn fake_notifier_records_send_call() {
        let notifier = FakeNotifier::new(-100123, 42);
        let rec = make_record("12345", IncidentStatus::Started);

        let (chat_id, msg_id) = notifier.send_incident(&rec).await.unwrap();

        assert_eq!(chat_id, -100123);
        assert_eq!(msg_id, 42);
        let calls = notifier.recorded_calls();
        assert_eq!(calls.len(), 1);
        if let NotifierCall::Send {
            incident_id,
            button_count,
            ..
        } = &calls[0]
        {
            assert_eq!(incident_id, "12345");
            assert_eq!(*button_count, 2, "started status should have 2 buttons");
        } else {
            panic!("expected Send call");
        }
    }

    #[tokio::test]
    async fn fake_notifier_records_edit_call() {
        let notifier = FakeNotifier::new(-100123, 42);
        let rec = make_record("12345", IncidentStatus::Acknowledged);

        notifier.edit_incident(-100123, 42, &rec).await.unwrap();

        let calls = notifier.recorded_calls();
        assert_eq!(calls.len(), 1);
        if let NotifierCall::Edit {
            chat_id,
            message_id,
            incident_id,
            button_count,
            ..
        } = &calls[0]
        {
            assert_eq!(*chat_id, -100123);
            assert_eq!(*message_id, 42);
            assert_eq!(incident_id, "12345");
            assert_eq!(*button_count, 1, "acknowledged status should have 1 button");
        } else {
            panic!("expected Edit call");
        }
    }

    #[tokio::test]
    async fn fake_notifier_records_multiple_calls() {
        let notifier = FakeNotifier::new(-100123, 42);
        let rec1 = make_record("111", IncidentStatus::Started);
        let rec2 = make_record("222", IncidentStatus::Resolved);

        notifier.send_incident(&rec1).await.unwrap();
        notifier.edit_incident(-100123, 42, &rec2).await.unwrap();

        let calls = notifier.recorded_calls();
        assert_eq!(calls.len(), 2);
    }
}
