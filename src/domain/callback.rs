/// Action encoded in Telegram callback data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Acknowledge,
    Resolve,
}

impl Action {
    fn as_str(&self) -> &'static str {
        match self {
            Action::Acknowledge => "ack",
            Action::Resolve => "res",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "ack" => Some(Action::Acknowledge),
            "res" => Some(Action::Resolve),
            _ => None,
        }
    }
}

/// Maximum byte length of Telegram callback_data.
pub const MAX_CALLBACK_DATA_BYTES: usize = 64;

/// Encode an action and incident ID into callback_data.
///
/// Returns `None` if the resulting string would exceed 64 bytes.
pub fn encode(action: Action, incident_id: &str) -> Option<String> {
    let data = format!("{}:{}", action.as_str(), incident_id);
    if data.len() > MAX_CALLBACK_DATA_BYTES {
        tracing::warn!(
            len = data.len(),
            max = MAX_CALLBACK_DATA_BYTES,
            "callback_data exceeds 64-byte limit; incident_id too long"
        );
        return None;
    }
    Some(data)
}

/// Decode callback_data into (Action, incident_id).
///
/// Returns `None` for any malformed input.
pub fn decode(data: &str) -> Option<(Action, String)> {
    if data.len() > MAX_CALLBACK_DATA_BYTES {
        return None;
    }
    let (action_str, incident_id) = data.split_once(':')?;
    let action = Action::from_str(action_str)?;
    if incident_id.is_empty() {
        return None;
    }
    Some((action, incident_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_ack_round_trips() {
        let data = encode(Action::Acknowledge, "12345").unwrap();
        assert_eq!(data, "ack:12345");
        let (action, id) = decode(&data).unwrap();
        assert_eq!(action, Action::Acknowledge);
        assert_eq!(id, "12345");
    }

    #[test]
    fn encode_res_round_trips() {
        let data = encode(Action::Resolve, "99999").unwrap();
        assert_eq!(data, "res:99999");
        let (action, id) = decode(&data).unwrap();
        assert_eq!(action, Action::Resolve);
        assert_eq!(id, "99999");
    }

    #[test]
    fn decode_malformed_no_colon_returns_none() {
        assert!(decode("ack12345").is_none());
    }

    #[test]
    fn decode_unknown_action_returns_none() {
        assert!(decode("unknown:12345").is_none());
    }

    #[test]
    fn decode_empty_incident_id_returns_none() {
        assert!(decode("ack:").is_none());
    }

    #[test]
    fn decode_empty_string_returns_none() {
        assert!(decode("").is_none());
    }

    #[test]
    fn decode_garbage_returns_none() {
        assert!(decode("garbage").is_none());
        assert!(decode(":").is_none());
        assert!(decode("::").is_none());
    }

    #[test]
    fn encode_returns_none_when_too_long() {
        // "ack:" is 4 bytes, so 61 chars of id = 65 bytes total.
        let long_id = "x".repeat(61);
        assert!(encode(Action::Acknowledge, &long_id).is_none());
    }

    #[test]
    fn encode_accepts_exactly_64_bytes() {
        // "ack:" is 4 bytes, so 60 chars of id = 64 bytes total.
        let id_60 = "x".repeat(60);
        let data = encode(Action::Acknowledge, &id_60).unwrap();
        assert_eq!(data.len(), 64);
        let (action, id) = decode(&data).unwrap();
        assert_eq!(action, Action::Acknowledge);
        assert_eq!(id, id_60);
    }

    #[test]
    fn decode_rejects_over_64_bytes() {
        let long_data = "ack:".to_string() + &"x".repeat(61);
        assert_eq!(long_data.len(), 65);
        assert!(decode(&long_data).is_none());
    }
}
