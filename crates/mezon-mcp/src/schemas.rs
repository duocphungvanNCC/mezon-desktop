use serde_json::{Map, Value, json};
use std::sync::Arc;

fn object(properties: Value, required: &[&str]) -> Map<String, Value> {
    serde_json::from_value(json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    }))
    .unwrap_or_default()
}

fn empty() -> Map<String, Value> {
    object(json!({}), &[])
}

fn id(desc: &str) -> Value {
    json!({
        "description": desc,
        "oneOf": [
            { "type": "integer" },
            { "type": "string", "pattern": "^[0-9]+$" }
        ]
    })
}

fn string(desc: &str) -> Value {
    json!({ "type": "string", "description": desc })
}

fn bool(desc: &str) -> Value {
    json!({ "type": "boolean", "description": desc })
}

fn integer(desc: &str, default: Option<i64>) -> Value {
    let mut value = json!({ "type": "integer", "description": desc });
    if let Some(default) = default {
        value["default"] = json!(default);
    }
    value
}

fn string_array(desc: &str) -> Value {
    json!({
        "type": "array",
        "description": desc,
        "items": { "type": "string" }
    })
}

fn clan_channel() -> Map<String, Value> {
    object(
        json!({
            "clan_id": id("Clan snowflake id. Use 0 for direct messages."),
            "channel_id": id("Channel snowflake id."),
        }),
        &["clan_id", "channel_id"],
    )
}

fn clan_channel_message() -> Map<String, Value> {
    object(
        json!({
            "clan_id": id("Clan snowflake id. Use 0 for direct messages."),
            "channel_id": id("Channel snowflake id."),
            "message_id": id("Target message snowflake id."),
        }),
        &["clan_id", "channel_id", "message_id"],
    )
}

pub fn input_schema(name: &str) -> Arc<Map<String, Value>> {
    match name {
        "get_app_info"
        | "ping"
        | "get_connection_status"
        | "list_clans"
        | "list_dm_channels"
        | "list_friends"
        | "get_account"
        | "get_current_context"
        | "get_settings"
        | "get_voice_status"
        | "list_stickers"
        | "capture_window"
        | "capture_chat"
        | "go_back"
        | "go_forward"
        | "show_window"
        | "logout"
        | "refresh"
        | "quit_app" => Arc::new(empty()),
        "list_channels" => Arc::new(object(
            json!({ "clan_id": id("Clan snowflake id to list channels for.") }),
            &["clan_id"],
        )),
        "list_channel_members" => Arc::new(object(
            json!({
                "clan_id": id("Clan snowflake id."),
                "channel_id": id("Optional channel id. When omitted, returns all clan members."),
                "channel_type": integer("Channel type when channel_id is set. Default 0.", Some(0)),
            }),
            &["clan_id"],
        )),
        "list_threads" | "list_pinned_messages" | "mark_as_read" => Arc::new(clan_channel()),
        "list_messages" => Arc::new(object(
            json!({
                "clan_id": id("Clan snowflake id. Use 0 for direct messages."),
                "channel_id": id("Channel snowflake id."),
                "message_id": id("Anchor message id. Use 0 to start from latest."),
                "direction": integer("0 = around anchor, 1 = older, 2 = newer.", Some(0)),
                "limit": integer("Max messages to return (default 50).", Some(50)),
            }),
            &["clan_id", "channel_id"],
        )),
        "get_message" => Arc::new(clan_channel_message()),
        "search_messages" => Arc::new(object(
            json!({
                "query": string("Search text."),
                "size": integer("Max hits to return (default 20).", Some(20)),
            }),
            &["query"],
        )),
        "get_sticker" => Arc::new(object(
            json!({
                "id": id("Sticker id."),
                "name": string("Sticker shortname (case-insensitive)."),
                "shortname": string("Alias for name."),
            }),
            &[],
        )),
        "get_image" => Arc::new(object(
            json!({
                "url": string("Direct image URL to download."),
                "clan_id": id("Clan id when resolving a message attachment."),
                "channel_id": id("Channel id when resolving a message attachment."),
                "message_id": id("Message id when resolving a message attachment."),
                "attachment_url": string("Attachment URL (skips message lookup)."),
                "attachment_index": integer("Zero-based attachment index on the message.", Some(0)),
            }),
            &[],
        )),
        "navigate" => Arc::new(object(
            json!({
                "path": string("In-app route starting with /. Example: /chat/clans/{clan_id}/channels/{channel_id}"),
            }),
            &["path"],
        )),
        "open_channel" => Arc::new(clan_channel()),
        "open_dm" => Arc::new(object(
            json!({
                "channel_id": id("Direct message channel id."),
                "channel_type": integer("DM channel type (default 3).", Some(3)),
            }),
            &["channel_id"],
        )),
        "open_settings" => Arc::new(object(
            json!({
                "page": string("Settings page slug. Default advanced. Examples: advanced, language, appearance, account."),
            }),
            &[],
        )),
        "send_message" => Arc::new(object(
            json!({
                "clan_id": id("Clan snowflake id. Use 0 for the current direct message."),
                "channel_id": id("Channel snowflake id."),
                "content": string("Plain-text message body."),
            }),
            &["clan_id", "channel_id", "content"],
        )),
        "reply_to_message" => Arc::new(object(
            json!({
                "clan_id": id("Clan snowflake id. Use 0 for direct messages."),
                "channel_id": id("Channel snowflake id."),
                "message_id": id("Parent message id to reply to."),
                "content": string("Reply text."),
            }),
            &["clan_id", "channel_id", "message_id", "content"],
        )),
        "react_to_message" => Arc::new(object(
            json!({
                "clan_id": id("Clan snowflake id."),
                "channel_id": id("Channel snowflake id."),
                "message_id": id("Message to react to."),
                "emoji": string("Emoji character or shortcode."),
                "remove": bool("When true, removes the reaction instead of adding it."),
                "message_sender_id": id("Optional message author id (resolved automatically when omitted)."),
                "topic_id": id("Optional topic id when reacting inside a discussion topic."),
            }),
            &["clan_id", "channel_id", "message_id", "emoji"],
        )),
        "click_message_button" => Arc::new(object(
            json!({
                "clan_id": id("Clan snowflake id."),
                "channel_id": id("Channel snowflake id."),
                "message_id": id("Message containing the button."),
                "button_id": string("Component id from get_message."),
                "button_label": string("Visible button label (used when button_id is omitted)."),
                "sender_id": id("Message sender id (defaults to message sender)."),
                "user_id": id("Clicking user id (defaults to signed-in user)."),
                "extra_data": string("JSON string passed to the button handler. Default {}."),
            }),
            &["clan_id", "channel_id", "message_id"],
        )),
        "select_message_option" => Arc::new(object(
            json!({
                "clan_id": id("Clan snowflake id."),
                "channel_id": id("Channel snowflake id."),
                "message_id": id("Message containing the dropdown."),
                "select_id": string("Dropdown component id from get_message."),
                "values": string_array("Selected option values before clicking a submit button."),
            }),
            &["clan_id", "channel_id", "message_id", "select_id"],
        )),
        "edit_message" => Arc::new(object(
            json!({
                "clan_id": id("Clan snowflake id."),
                "channel_id": id("Channel snowflake id."),
                "message_id": id("Message to edit."),
                "content": string("New message text."),
                "topic_id": id("Optional topic id when editing inside a discussion topic."),
                "is_update_msg_topic": bool("When true, sends the edit as a topic message update."),
            }),
            &["clan_id", "channel_id", "message_id", "content"],
        )),
        "delete_message" => Arc::new(object(
            json!({
                "clan_id": id("Clan snowflake id. Use 0 for direct messages."),
                "channel_id": id("Channel snowflake id."),
                "message_id": id("Target message snowflake id."),
                "topic_id": id("Optional topic id when deleting inside a discussion topic."),
            }),
            &["clan_id", "channel_id", "message_id"],
        )),
        "send_image" => Arc::new(object(
            json!({
                "clan_id": id("Clan snowflake id."),
                "channel_id": id("Channel snowflake id."),
                "content": string("Optional caption text."),
                "path": string("Local filesystem path to an image file."),
                "url": string("Remote image URL (alternative to path)."),
            }),
            &["clan_id", "channel_id"],
        )),
        "send_sticker" => Arc::new(object(
            json!({
                "clan_id": id("Clan snowflake id."),
                "channel_id": id("Channel snowflake id."),
                "url": string("Sticker image URL."),
                "name": string("Sticker shortname (alternative to url)."),
                "shortname": string("Alias for name."),
            }),
            &["clan_id", "channel_id"],
        )),
        "set_setting" => Arc::new(object(
            json!({
                "key": string("Allowlisted key: theme, language, zoom_factor, notifications_enabled, activity_tracking."),
                "value": json!({
                    "description": "Setting value. Type depends on key (string, number, or boolean)."
                }),
            }),
            &["key", "value"],
        )),
        "set_cli_enabled" => Arc::new(object(
            json!({
                "enabled": bool("When true, installs the mezon CLI shim into PATH."),
            }),
            &["enabled"],
        )),
        _ => Arc::new(empty()),
    }
}
