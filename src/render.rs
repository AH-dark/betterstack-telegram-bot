use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use teloxide::utils::html;

use crate::domain::callback::{encode, Action};
use crate::domain::incident::{IncidentRecord, IncidentStatus};

/// Status emoji for display.
fn status_emoji(status: IncidentStatus) -> &'static str {
    match status {
        IncidentStatus::Started => "🚨",
        IncidentStatus::Acknowledged => "👀",
        IncidentStatus::Resolved => "✅",
    }
}

/// Render an incident record into (HTML message text, inline keyboard).
///
/// This is a pure function: no I/O, no side effects.
pub fn render(rec: &IncidentRecord) -> (String, InlineKeyboardMarkup) {
    let text = build_text(rec);
    let keyboard = build_keyboard(rec);
    (text, keyboard)
}

fn build_text(rec: &IncidentRecord) -> String {
    let emoji = status_emoji(rec.status);
    let status_label = rec.status.to_string();

    let mut lines = vec![
        format!("{} <b>{}</b>", emoji, html::escape(&rec.name)),
        format!("Status: {} {}", emoji, html::escape(&status_label)),
        format!("Cause: {}", html::escape(&rec.cause)),
        format!(
            "Monitor: <a href=\"{}\">{}</a>",
            html::escape(&rec.url),
            html::escape(&rec.url)
        ),
    ];

    if let Some(ref started_at) = rec.started_at {
        lines.push(format!("Started: {}", html::escape(started_at)));
    }

    if let Some(ref ack_at) = rec.acknowledged_at {
        let by = rec.acknowledged_by.as_deref().unwrap_or("unknown");
        lines.push(format!(
            "Acknowledged: {} by {}",
            html::escape(ack_at),
            html::escape(by)
        ));
    }

    if let Some(ref res_at) = rec.resolved_at {
        let by = rec.resolved_by.as_deref().unwrap_or("unknown");
        lines.push(format!(
            "Resolved: {} by {}",
            html::escape(res_at),
            html::escape(by)
        ));
    }

    lines.join("\n")
}

fn build_keyboard(rec: &IncidentRecord) -> InlineKeyboardMarkup {
    let mut buttons: Vec<InlineKeyboardButton> = Vec::new();

    match rec.status {
        IncidentStatus::Started => {
            if let Some(data) = encode(Action::Acknowledge, &rec.id) {
                buttons.push(InlineKeyboardButton::callback("✅ Acknowledge", data));
            }
            if let Some(data) = encode(Action::Resolve, &rec.id) {
                buttons.push(InlineKeyboardButton::callback("🔵 Resolve", data));
            }
        }
        IncidentStatus::Acknowledged => {
            if let Some(data) = encode(Action::Resolve, &rec.id) {
                buttons.push(InlineKeyboardButton::callback("🔵 Resolve", data));
            }
        }
        IncidentStatus::Resolved => {}
    }

    if buttons.is_empty() {
        InlineKeyboardMarkup::new(Vec::<Vec<InlineKeyboardButton>>::new())
    } else {
        InlineKeyboardMarkup::new(vec![buttons])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::incident::{IncidentRecord, IncidentStatus};

    fn make_started_record() -> IncidentRecord {
        IncidentRecord {
            id: "12345".to_string(),
            name: "Homepage down".to_string(),
            url: "https://example.com".to_string(),
            cause: "Status 500".to_string(),
            status: IncidentStatus::Started,
            started_at: Some("2026-01-01T00:00:00Z".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn render_started_has_two_buttons() {
        let rec = make_started_record();
        let (_, keyboard) = render(&rec);
        let rows = keyboard.inline_keyboard;
        assert_eq!(rows.len(), 1, "should have one row");
        assert_eq!(rows[0].len(), 2, "started status should have 2 buttons");
    }

    #[test]
    fn render_acknowledged_has_one_button() {
        let rec = IncidentRecord {
            status: IncidentStatus::Acknowledged,
            acknowledged_at: Some("2026-01-01T00:05:00Z".to_string()),
            acknowledged_by: Some("alice".to_string()),
            ..make_started_record()
        };
        let (_, keyboard) = render(&rec);
        let rows = keyboard.inline_keyboard;
        assert_eq!(rows.len(), 1, "should have one row");
        assert_eq!(rows[0].len(), 1, "acknowledged status should have 1 button");
    }

    #[test]
    fn render_resolved_has_no_buttons() {
        let rec = IncidentRecord {
            status: IncidentStatus::Resolved,
            resolved_at: Some("2026-01-01T00:10:00Z".to_string()),
            resolved_by: Some("bob".to_string()),
            ..make_started_record()
        };
        let (_, keyboard) = render(&rec);
        let rows = keyboard.inline_keyboard;
        assert!(
            rows.is_empty() || rows[0].is_empty(),
            "resolved status should have no buttons"
        );
    }

    #[test]
    fn render_escapes_html_in_name() {
        let rec = IncidentRecord {
            name: "<script>alert('xss')</script>".to_string(),
            ..make_started_record()
        };
        let (text, _) = render(&rec);
        assert!(!text.contains("<script>"), "HTML should be escaped");
        assert!(
            text.contains("&lt;script&gt;"),
            "HTML should be escaped to entities"
        );
    }

    #[test]
    fn render_escapes_html_in_cause() {
        let rec = IncidentRecord {
            cause: "Error: <b>critical</b> & failure".to_string(),
            ..make_started_record()
        };
        let (text, _) = render(&rec);
        assert!(
            !text.contains("<b>critical</b>"),
            "HTML in cause should be escaped"
        );
        assert!(
            text.contains("&lt;b&gt;critical&lt;/b&gt;"),
            "HTML should be escaped to entities"
        );
    }

    #[test]
    fn render_shows_acknowledged_line_when_set() {
        let rec = IncidentRecord {
            status: IncidentStatus::Acknowledged,
            acknowledged_at: Some("2026-01-01T00:05:00Z".to_string()),
            acknowledged_by: Some("alice".to_string()),
            ..make_started_record()
        };
        let (text, _) = render(&rec);
        assert!(
            text.contains("Acknowledged:"),
            "should show acknowledged line"
        );
        assert!(text.contains("alice"), "should show acknowledged_by");
    }

    #[test]
    fn render_shows_resolved_line_when_set() {
        let rec = IncidentRecord {
            status: IncidentStatus::Resolved,
            resolved_at: Some("2026-01-01T00:10:00Z".to_string()),
            resolved_by: Some("bob".to_string()),
            ..make_started_record()
        };
        let (text, _) = render(&rec);
        assert!(text.contains("Resolved:"), "should show resolved line");
        assert!(text.contains("bob"), "should show resolved_by");
    }

    #[test]
    fn render_does_not_show_optional_lines_when_absent() {
        let rec = make_started_record();
        let (text, _) = render(&rec);
        assert!(
            !text.contains("Acknowledged:"),
            "should not show acknowledged line when absent"
        );
        assert!(
            !text.contains("Resolved:"),
            "should not show resolved line when absent"
        );
    }

    #[test]
    fn render_button_callback_data_is_correct_format() {
        let rec = make_started_record();
        let (_, keyboard) = render(&rec);
        let row = &keyboard.inline_keyboard[0];

        let ack_data = row[0].kind.clone();
        if let teloxide::types::InlineKeyboardButtonKind::CallbackData(data) = ack_data {
            assert_eq!(data, "ack:12345");
        } else {
            panic!("expected callback data button");
        }

        let res_data = row[1].kind.clone();
        if let teloxide::types::InlineKeyboardButtonKind::CallbackData(data) = res_data {
            assert_eq!(data, "res:12345");
        } else {
            panic!("expected callback data button");
        }
    }
}
