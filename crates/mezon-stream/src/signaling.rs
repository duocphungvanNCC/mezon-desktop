use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct OutboundMessage {
    #[serde(rename = "Key")]
    pub key: String,
    #[serde(rename = "ClanId", skip_serializing_if = "Option::is_none")]
    pub clan_id: Option<String>,
    #[serde(rename = "ChannelId", skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(rename = "UserId", skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(rename = "Value", skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

impl OutboundMessage {
    pub fn get_channels() -> Self {
        Self {
            key: "get_channels".into(),
            clan_id: None,
            channel_id: None,
            user_id: None,
            value: None,
        }
    }

    pub fn session_subscriber(
        clan_id: &str,
        channel_id: &str,
        user_id: &str,
        offer: Value,
    ) -> Self {
        Self {
            key: "session_subscriber".into(),
            clan_id: Some(clan_id.to_string()),
            channel_id: Some(channel_id.to_string()),
            user_id: Some(user_id.to_string()),
            value: Some(offer),
        }
    }

    pub fn connect_subscriber(
        clan_id: &str,
        channel_id: &str,
        user_id: &str,
        stream_id: &str,
    ) -> Self {
        Self {
            key: "connect_subscriber".into(),
            clan_id: Some(clan_id.to_string()),
            channel_id: Some(channel_id.to_string()),
            user_id: Some(user_id.to_string()),
            value: Some(serde_json::json!({ "ChannelId": stream_id })),
        }
    }

    pub fn ice_candidate(candidate: Value) -> Self {
        Self {
            key: "ice_candidate".into(),
            clan_id: None,
            channel_id: None,
            user_id: None,
            value: Some(candidate),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct InboundMessage {
    #[serde(rename = "Key")]
    pub key: String,
    #[serde(rename = "Value")]
    pub value: Option<Value>,
}

pub fn ws_connect_url(base: &str, username: &str, token: &str) -> anyhow::Result<url::Url> {
    let base = base.trim_end_matches('/');
    let mut url = url::Url::parse(&format!("{base}/ws"))?;
    url.query_pairs_mut()
        .append_pair("username", username)
        .append_pair("token", token);
    Ok(url)
}

pub fn parse_channels(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect(),
        Value::String(s) => vec![s.clone()],
        _ => Vec::new(),
    }
}

pub fn parse_sdp_answer(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Object(map) => map.get("sdp").and_then(|v| v.as_str()).map(str::to_string),
        _ => None,
    }
}

pub fn parse_ice_candidate(value: &Value) -> Option<(String, i32, String)> {
    let map = value.as_object()?;
    let candidate = map.get("candidate")?.as_str()?.to_string();
    if candidate.is_empty() {
        return None;
    }
    let sdp_mid = map
        .get("sdpMid")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sdp_mline_index = map
        .get("sdpMLineIndex")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;
    Some((sdp_mid, sdp_mline_index, candidate))
}
