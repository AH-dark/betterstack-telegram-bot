use std::sync::Arc;

use teloxide::adaptors::Throttle;
use teloxide::prelude::*;
use teloxide::types::CallbackQuery;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use super::Notifier;
use crate::betterstack::client::BetterStackClient;
use crate::domain::callback::{decode, Action};
use crate::domain::incident::{self, Trigger};
use crate::storage::IncidentStore;

fn actor_name(query: &CallbackQuery) -> String {
    query
        .from
        .username
        .clone()
        .unwrap_or_else(|| query.from.first_name.clone())
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|err| {
            tracing::warn!(error = %err, "failed to format callback timestamp");
            "unknown".to_string()
        })
}

pub async fn callback_handler(
    bot: Throttle<Bot>,
    query: CallbackQuery,
    store: Arc<dyn IncidentStore>,
    notifier: Arc<dyn Notifier>,
    betterstack_client: BetterStackClient,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let query_id = query.id.clone();

    let (action, incident_id) = match query.data.as_deref().and_then(decode) {
        Some(decoded) => decoded,
        None => {
            tracing::warn!(data = ?query.data, "malformed callback data");
            let _ = bot
                .answer_callback_query(query_id)
                .text("Invalid button data. Please try again.")
                .show_alert(true)
                .await;
            return Ok(());
        }
    };

    let _ = bot.answer_callback_query(query_id.clone()).await;

    let actor = actor_name(&query);
    let api_result = match action {
        Action::Acknowledge => {
            betterstack_client
                .acknowledge(&incident_id, Some(&actor))
                .await
        }
        Action::Resolve => betterstack_client.resolve(&incident_id, Some(&actor)).await,
    };

    if let Err(err) = api_result {
        tracing::error!(error = %err, incident_id = %incident_id, "Better Stack API call failed");
        let _ = bot
            .answer_callback_query(query_id)
            .text("Failed to update incident. Please try again.")
            .show_alert(true)
            .await;
        return Ok(());
    }

    let current = match store.get(&incident_id).await {
        Ok(Some(record)) => record,
        Ok(None) => {
            tracing::warn!(incident_id = %incident_id, "incident not found in store for callback");
            return Ok(());
        }
        Err(err) => {
            tracing::error!(error = %err, incident_id = %incident_id, "store read failed in callback handler");
            return Ok(());
        }
    };

    let trigger = match action {
        Action::Acknowledge => Trigger::Acknowledged,
        Action::Resolve => Trigger::Resolved,
    };
    let (new_status, changed) = incident::next(current.status, &trigger);

    if changed {
        let now = now_rfc3339();
        if let Err(err) = store
            .set_status(&incident_id, new_status, Some(&actor), &now)
            .await
        {
            tracing::error!(error = %err, incident_id = %incident_id, "store status update failed in callback handler");
            return Ok(());
        }
    }

    let updated = match store.get(&incident_id).await {
        Ok(Some(record)) => record,
        Ok(None) => {
            tracing::warn!(incident_id = %incident_id, "incident disappeared after callback update");
            return Ok(());
        }
        Err(err) => {
            tracing::error!(error = %err, incident_id = %incident_id, "store read after update failed in callback handler");
            return Ok(());
        }
    };

    if let (Some(chat_id), Some(message_id)) = (updated.chat_id, updated.message_id) {
        if let Err(err) = notifier.edit_incident(chat_id, message_id, &updated).await {
            tracing::error!(error = %err, incident_id = %incident_id, "failed to edit Telegram message in callback handler");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use teloxide::types::{CallbackQueryId, User, UserId};

    fn make_user(username: Option<&str>, first_name: &str) -> User {
        User {
            id: UserId(12345),
            is_bot: false,
            first_name: first_name.to_string(),
            last_name: None,
            username: username.map(str::to_string),
            language_code: None,
            is_premium: false,
            added_to_attachment_menu: false,
        }
    }

    fn make_query(user: User) -> CallbackQuery {
        CallbackQuery {
            id: CallbackQueryId("test".to_string()),
            from: user,
            message: None,
            inline_message_id: None,
            chat_instance: String::new(),
            data: None,
            game_short_name: None,
        }
    }

    #[test]
    fn actor_name_uses_username_when_available() {
        let query = make_query(make_user(Some("alice"), "Alice"));

        assert_eq!(actor_name(&query), "alice");
    }

    #[test]
    fn actor_name_falls_back_to_first_name() {
        let query = make_query(make_user(None, "Bob"));

        assert_eq!(actor_name(&query), "Bob");
    }

    #[test]
    fn now_rfc3339_returns_formatted_timestamp() {
        let timestamp = now_rfc3339();

        assert!(
            OffsetDateTime::parse(&timestamp, &Rfc3339).is_ok(),
            "timestamp should parse as RFC3339: {timestamp}"
        );
    }
}
