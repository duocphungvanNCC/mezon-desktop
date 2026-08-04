use crate::schemas;

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub write: bool,
}

pub const TOOL_SPECS: &[ToolSpec] = &[
    ToolSpec {
        name: "get_app_info",
        description: "\
Return Mezon desktop app metadata: version, platform, MCP server status, and auth summary.

Use on startup to confirm the app is reachable and the user is signed in.

Parameters: none.",
        write: false,
    },
    ToolSpec {
        name: "ping",
        description: "\
Health check through the Mezon control plane.

Returns { ok: true } when the desktop app is running and responding.

Parameters: none.",
        write: false,
    },
    ToolSpec {
        name: "get_connection_status",
        description: "\
Return realtime socket connection status for the signed-in session.

Use before sending messages or listing live data.

Parameters: none.",
        write: false,
    },
    ToolSpec {
        name: "list_clans",
        description: "\
List clans the signed-in user belongs to.

Each item includes ids and display names. Use clan_id from here for channel/message tools.

Parameters: none.",
        write: false,
    },
    ToolSpec {
        name: "list_channels",
        description: "\
List channels inside a clan.

Returns channel ids, names, types, and parent relationships. Pair with open_channel or list_messages.

Parameters:
- clan_id (required): clan snowflake id.",
        write: false,
    },
    ToolSpec {
        name: "list_dm_channels",
        description: "\
List direct message channels for the signed-in user.

Use channel_id with open_dm or send_message (clan_id=0).

Parameters: none.",
        write: false,
    },
    ToolSpec {
        name: "list_friends",
        description: "\
List friends of the signed-in user.

Parameters: none.",
        write: false,
    },
    ToolSpec {
        name: "get_account",
        description: "\
Return the signed-in account profile (user id, username, display name, email, etc.).

Parameters: none.",
        write: false,
    },
    ToolSpec {
        name: "list_channel_members",
        description: "\
List members in a clan or a specific channel.

When channel_id is omitted, returns up to 100 clan members. When channel_id is set, returns channel participants.

Parameters:
- clan_id (required)
- channel_id (optional)
- channel_type (optional, default 0)",
        write: false,
    },
    ToolSpec {
        name: "list_threads",
        description: "\
List threads in a channel.

Parameters:
- clan_id (required)
- channel_id (required)",
        write: false,
    },
    ToolSpec {
        name: "list_pinned_messages",
        description: "\
List pinned messages in a channel.

Parameters:
- clan_id (required)
- channel_id (required)",
        write: false,
    },
    ToolSpec {
        name: "list_messages",
        description: "\
Fetch messages from a channel with pagination.

Use message_id as an anchor (0 for latest) and direction to page older/newer messages.

Parameters:
- clan_id (required; use 0 for DMs)
- channel_id (required)
- message_id (optional, default 0)
- direction (optional: 0 around, 1 older, 2 newer)
- limit (optional, default 50)",
        write: false,
    },
    ToolSpec {
        name: "get_message",
        description: "\
Fetch one message with full detail: text, attachments, embeds, and interactive components (buttons, dropdowns).

Use before click_message_button or select_message_option.

Parameters:
- clan_id (required)
- channel_id (required)
- message_id (required)",
        write: false,
    },
    ToolSpec {
        name: "search_messages",
        description: "\
Search message content across accessible channels.

Parameters:
- query (required): search text
- size (optional, default 20): max results",
        write: false,
    },
    ToolSpec {
        name: "get_current_context",
        description: "\
Return the UI route and parsed context for the active screen.

Includes route, auth state, user_id, clan_id, and channel_id when viewing chat. Call this first to discover where the user is.

Parameters: none.",
        write: false,
    },
    ToolSpec {
        name: "get_scroll_state",
        description: "\
Report what the open channel's message list currently holds.

Returns loaded_count, has_more_top, has_more_bottom, and the active clan/channel ids.
`list_messages` reads history straight from the API without moving the UI, so use this
to learn whether the on-screen list still has older or newer pages to pull in, and pair
it with load_more_messages before capture_chat if you need those rows rendered.

Parameters: none.",
        write: false,
    },
    ToolSpec {
        name: "load_more_messages",
        description: "\
Pull the next page of history into the open channel's message list.

This is the deterministic equivalent of the user scrolling to the top or bottom edge:
it drives the same store fetch, so the rows become visible to capture_chat. Returns
started=false when the list already holds that end of the history.

Check has_more_top / has_more_bottom from get_scroll_state first, and call repeatedly
to walk further back.

Parameters:
- direction (optional): \"older\" (default) or \"newer\".",
        write: true,
    },
    ToolSpec {
        name: "list_emojis",
        description: "\
List custom emoji available to the signed-in user.

Emoji in a message body are spans, not text: sending the literal \":shortname:\" through
send_message renders as plain characters. Look the emoji up here, then pass its
shortname and emoji_id in send_message's `emojis` array so it renders.

Parameters:
- clan_id (optional): restrict to one clan's emoji.
- query (optional): case-insensitive substring match on shortname.
- limit (optional): max entries to return. Default 100.",
        write: false,
    },
    ToolSpec {
        name: "pin_message",
        description: "\
Pin a message to the channel.

The pin shows up for everyone in the channel; list_pinned_messages reads them back.

Parameters:
- clan_id (required): clan snowflake id. Use 0 for direct messages.
- channel_id (required): channel snowflake id.
- message_id (required): message to pin.",
        write: true,
    },
    ToolSpec {
        name: "unpin_message",
        description: "\
Remove a pinned message from the channel.

Call list_pinned_messages first: `pin_id` is the entry's own id, which is not the
message id.

Parameters:
- clan_id (required): clan snowflake id. Use 0 for direct messages.
- channel_id (required): channel snowflake id.
- message_id (required): the pinned message's id.
- pin_id (required): the pin entry id from list_pinned_messages.",
        write: true,
    },
    ToolSpec {
        name: "create_poll",
        description: "\
Post a poll to the channel.

Returns the created poll so its id can be passed to vote_poll.

Parameters:
- clan_id (required): clan snowflake id. Use 0 for direct messages.
- channel_id (required): channel snowflake id.
- question (required): poll question.
- answers (required): array of 2 or more answer strings.
- expire_hours (optional): hours until the poll closes. Default 24.
- poll_type (optional): 0 = single choice (default), 1 = multiple choice.",
        write: true,
    },
    ToolSpec {
        name: "vote_poll",
        description: "\
Cast a vote on an existing poll.

Parameters:
- poll_id (required): poll snowflake id.
- message_id (required): id of the message carrying the poll.
- channel_id (required): channel snowflake id.
- answers (required): array of zero-based answer indices. Single-choice polls take one.",
        write: true,
    },
    ToolSpec {
        name: "get_settings",
        description: "\
Read app settings: theme, language, zoom, notifications, voice state, and related flags.

Parameters: none.",
        write: false,
    },
    ToolSpec {
        name: "get_voice_status",
        description: "\
Return voice call status derived from settings: in_call, channel label, mic/camera enabled.

Parameters: none.",
        write: false,
    },
    ToolSpec {
        name: "list_stickers",
        description: "\
List stickers available to the signed-in user.

Parameters: none.",
        write: false,
    },
    ToolSpec {
        name: "get_sticker",
        description: "\
Look up one sticker by id or shortname.

Parameters (provide one):
- id
- name or shortname",
        write: false,
    },
    ToolSpec {
        name: "get_image",
        description: "\
Download an image and return base64 bytes.

Provide either url directly, or clan_id + channel_id + message_id (+ optional attachment_index) to resolve a message attachment.

Parameters:
- url (optional)
- clan_id, channel_id, message_id, attachment_index (optional)
- attachment_url (optional shortcut)",
        write: false,
    },
    ToolSpec {
        name: "capture_window",
        description: "\
Capture a PNG screenshot of the entire Mezon main window using OS screen capture (scap).

Requires Screen Recording permission (macOS). The window must be visible on screen. Does not use GPUI render-to-texture.

Returns: { format: \"png\", width, height, region: \"window\", source: \"scap\", data_base64 }.

Tip: decode data_base64 to a file, then send_image with path.

Parameters: none.",
        write: false,
    },
    ToolSpec {
        name: "capture_chat",
        description: "\
Capture a PNG screenshot of the chat panel only (excludes the left sidebar) using OS screen capture (scap).

Requires Screen Recording permission (macOS). The window must be visible. Cropping uses the fixed sidebar width and current display scale factor.

Returns: { format: \"png\", width, height, region: \"chat\", source: \"scap\", data_base64 }.

Workflow: get_current_context → capture_chat → write PNG to disk → send_image.

Parameters: none.",
        write: false,
    },
    ToolSpec {
        name: "navigate",
        description: "\
Navigate the in-app router to a path.

Path must start with / and must not be an external URL. Examples: /settings/advanced, /chat/clans/{clan_id}/channels/{channel_id}.

Parameters:
- path (required)",
        write: true,
    },
    ToolSpec {
        name: "open_channel",
        description: "\
Open a clan channel in the UI.

Equivalent to navigating to /chat/clans/{clan_id}/channels/{channel_id}.

Parameters:
- clan_id (required)
- channel_id (required)",
        write: true,
    },
    ToolSpec {
        name: "open_dm",
        description: "\
Open a direct message channel in the UI.

Parameters:
- channel_id (required)
- channel_type (optional, default 3)",
        write: true,
    },
    ToolSpec {
        name: "open_settings",
        description: "\
Open an app settings page.

Parameters:
- page (optional, default advanced). Examples: advanced, language, appearance, account.",
        write: true,
    },
    ToolSpec {
        name: "go_back",
        description: "\
Navigate back in the in-app history stack.

Parameters: none.",
        write: true,
    },
    ToolSpec {
        name: "go_forward",
        description: "\
Navigate forward in the in-app history stack.

Parameters: none.",
        write: true,
    },
    ToolSpec {
        name: "show_window",
        description: "\
Bring the main Mezon window to the foreground.

Useful before capture_chat/capture_window so the window is visible.

Parameters: none.",
        write: true,
    },
    ToolSpec {
        name: "send_message",
        description: "\
Send a plain-text message to a channel.

For direct messages, use clan_id=0 and the DM channel_id. The app auto-joins the channel when needed.

Parameters:
- clan_id (required)
- channel_id (required)
- content (required)",
        write: true,
    },
    ToolSpec {
        name: "reply_to_message",
        description: "\
Reply to an existing message in a channel.

Parameters:
- clan_id (required)
- channel_id (required)
- message_id (required): parent message
- content (required)",
        write: true,
    },
    ToolSpec {
        name: "react_to_message",
        description: "\
Add or remove an emoji reaction on a message.

Parameters:
- clan_id (required)
- channel_id (required)
- message_id (required)
- emoji (required)
- remove (optional boolean)
- message_sender_id (optional)
- topic_id (optional): discussion topic id when reacting inside a topic",
        write: true,
    },
    ToolSpec {
        name: "click_message_button",
        description: "\
Click an interactive button attached to a message.

Fetch the message first with get_message to read button ids/labels.

Parameters:
- clan_id, channel_id, message_id (required)
- button_id or button_label (one required)
- sender_id, user_id, extra_data (optional)",
        write: true,
    },
    ToolSpec {
        name: "select_message_option",
        description: "\
Select value(s) on a dropdown component before submitting a message button.

Parameters:
- clan_id, channel_id, message_id, select_id (required)
- values (optional string array)",
        write: true,
    },
    ToolSpec {
        name: "edit_message",
        description: "\
Edit the text content of an existing message.

Parameters:
- clan_id (required)
- channel_id (required)
- message_id (required)
- content (required)
- topic_id (optional): discussion topic id when editing inside a topic
- is_update_msg_topic (optional boolean)",
        write: true,
    },
    ToolSpec {
        name: "delete_message",
        description: "\
Delete a message from a channel.

Parameters:
- clan_id (required)
- channel_id (required)
- message_id (required)
- topic_id (optional): discussion topic id when deleting inside a topic",
        write: true,
    },
    ToolSpec {
        name: "mark_as_read",
        description: "\
Mark a channel as read for the signed-in user.

Parameters:
- clan_id (required)
- channel_id (required)",
        write: true,
    },
    ToolSpec {
        name: "send_image",
        description: "\
Send an image to a channel from a local file path or remote URL.

For captures from capture_chat, write data_base64 to a .png file and pass path.

Parameters:
- clan_id (required)
- channel_id (required)
- path or url (one required)
- content (optional caption)",
        write: true,
    },
    ToolSpec {
        name: "send_sticker",
        description: "\
Send a sticker to a channel by URL or shortname.

Parameters:
- clan_id (required)
- channel_id (required)
- url or name/shortname (one required)",
        write: true,
    },
    ToolSpec {
        name: "set_setting",
        description: "\
Update an allowlisted app setting.

Allowed keys: theme, language, zoom_factor, notifications_enabled, activity_tracking.

Parameters:
- key (required)
- value (required; string, number, or boolean depending on key)",
        write: true,
    },
    ToolSpec {
        name: "set_cli_enabled",
        description: "\
Install or remove the mezon CLI shim in the user PATH.

Parameters:
- enabled (required boolean)",
        write: true,
    },
    ToolSpec {
        name: "logout",
        description: "\
Sign out of the current Mezon session.

Parameters: none.",
        write: true,
    },
    ToolSpec {
        name: "refresh",
        description: "\
Refresh clans, direct messages, and message lists in the UI.

Parameters: none.",
        write: true,
    },
    ToolSpec {
        name: "quit_app",
        description: "\
Quit the Mezon desktop application.

Parameters: none.",
        write: true,
    },
];

pub fn list_tools_json(read_only: bool) -> serde_json::Value {
    let tools: Vec<_> = TOOL_SPECS
        .iter()
        .filter(|tool| !read_only || !tool.write)
        .map(|tool| {
            serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "write": tool.write,
                "inputSchema": schemas::input_schema(tool.name),
            })
        })
        .collect();
    serde_json::json!({ "tools": tools })
}

pub fn is_write_tool(name: &str) -> bool {
    TOOL_SPECS
        .iter()
        .find(|tool| tool.name == name)
        .is_some_and(|tool| tool.write)
}

#[cfg(test)]
mod tests {
    use super::{TOOL_SPECS, list_tools_json};
    use crate::schemas;

    #[test]
    fn every_tool_has_description_and_schema() {
        for tool in TOOL_SPECS {
            assert!(!tool.description.is_empty(), "{}", tool.name);
            let schema = schemas::input_schema(tool.name);
            assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        }
    }

    #[test]
    fn list_tools_json_includes_input_schema() {
        let value = list_tools_json(false);
        let tools = value
            .get("tools")
            .and_then(|v| v.as_array())
            .expect("tools array");
        assert_eq!(tools.len(), TOOL_SPECS.len());
        assert!(tools[0].get("inputSchema").is_some());
    }
}
