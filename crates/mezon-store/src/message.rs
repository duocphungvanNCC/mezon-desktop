
use std::sync::Arc;

use gpui::SharedString;
use mezon_client::transport::{ApiMessageContent, ApiMessageReaction, ContentToken};

use crate::album_layout::AlbumLayout;
use crate::config::AppConfig;
use crate::ids::{MessageId, UserId};
use crate::message_time::{format_local_time_hhmm, local_datetime, local_day_key};

#[derive(Debug, Clone, Default)]
pub struct MessageAttachment {
    pub url: String,
    pub filename: String,
    pub filetype: String,
    pub width: u32,
    pub height: u32,
    pub thumbnail: String,
    pub duration: i32,
    pub proxied_src: SharedString,
    pub thumbnail_proxied: SharedString,
    pub display_width: f32,
    pub display_height: f32,
    pub tenor_mp4: Option<SharedString>,
}

impl MessageAttachment {
    pub fn is_image(&self) -> bool {
        self.filetype.starts_with("image/")
            || matches!(
                self.url
                    .split(['?', '#'])
                    .next()
                    .and_then(|u| u.rsplit('.').next())
                    .map(|ext| ext.to_ascii_lowercase())
                    .as_deref(),
                Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "avif")
            )
    }

    pub fn is_video(&self) -> bool {
        ((self.filetype.contains("video/mp4") || self.filetype.contains("video/quicktime"))
            && !self.url.contains("tenor.com"))
            || (self.filetype.starts_with("video") && !self.filetype.ends_with("vnd.dlna.mpeg-tts"))
    }

    pub fn is_unsupported_media(&self) -> bool {
        matches!(
            self.filetype.as_str(),
            "video/x-ms-wmv"
                | "video/wmv"
                | "video/avi"
                | "video/flv"
                | "video/mkv"
                | "video/rmvb"
                | "audio/wma"
                | "audio/ra"
                | "audio/atrac"
                | "image/tiff"
                | "image/bmp"
                | "image/psd"
        )
    }
}

pub fn tenor_mp4_url(gif_url: &str) -> Option<String> {
    let rest = gif_url.strip_prefix("https://media.tenor.com/")?;
    let (media_id, name) = rest.split_once('/')?;
    let name = name.strip_suffix(".gif")?;
    if media_id.len() != 16 || !media_id.is_ascii() || name.is_empty() {
        return None;
    }
    let content_id = &media_id[..11];
    Some(format!(
        "https://media.tenor.com/{content_id}AAAPo/{name}.mp4"
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageCode {
    Chat,
    ChatUpdate,
    ChatRemove,
    Typing,
    Indicator,
    Welcome,
    CreateThread,
    CreatePin,
    MessageBuzz,
    Topic,
    AuditLog,
    SendToken,
    Ephemeral,
    UpcomingEvent,
    UpdateEphemeralMsg,
    DeleteEphemeralMsg,
    ShareContact,
    Location,
    Poll,
    Unknown(i32),
}

impl MessageCode {
    pub fn from_raw(raw: i32) -> Self {
        match raw {
            0 => MessageCode::Chat,
            1 => MessageCode::ChatUpdate,
            2 => MessageCode::ChatRemove,
            3 => MessageCode::Typing,
            4 => MessageCode::Indicator,
            5 => MessageCode::Welcome,
            6 => MessageCode::CreateThread,
            7 => MessageCode::CreatePin,
            8 => MessageCode::MessageBuzz,
            9 => MessageCode::Topic,
            10 => MessageCode::AuditLog,
            11 => MessageCode::SendToken,
            12 => MessageCode::Ephemeral,
            13 => MessageCode::UpcomingEvent,
            14 => MessageCode::UpdateEphemeralMsg,
            15 => MessageCode::DeleteEphemeralMsg,
            16 => MessageCode::ShareContact,
            17 => MessageCode::Location,
            18 => MessageCode::Poll,
            other => MessageCode::Unknown(other),
        }
    }

    pub fn is_system(self) -> bool {
        matches!(
            self,
            MessageCode::Welcome
                | MessageCode::UpcomingEvent
                | MessageCode::CreateThread
                | MessageCode::CreatePin
                | MessageCode::AuditLog
        )
    }

    pub fn is_user_timeline(self) -> bool {
        !matches!(
            self,
            MessageCode::Indicator
                | MessageCode::Typing
                | MessageCode::ChatUpdate
                | MessageCode::ChatRemove
                | MessageCode::UpdateEphemeralMsg
                | MessageCode::DeleteEphemeralMsg
        ) && !self.is_system()
    }
}

/// A reply/reference shown above a message ("replying to …").
#[derive(Debug, Clone, Default)]
pub struct MessageReference {
    pub message_ref_id: MessageId,
    pub sender_id: UserId,
    pub sender_name: String,
    pub sender_avatar: String,
    pub content: String,
    pub has_attachment: bool,
}


#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReactionSender {
    pub sender_id: String,
    pub count: u32,
}


#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reaction {
    /// Aggregation key (emoji id when present, else shortname).
    pub key: String,
    pub emoji: SharedString,
    pub emoji_id: SharedString,
    pub emoji_proxied: SharedString,
    pub count: u32,
    pub count_label: SharedString,
    pub senders: Vec<ReactionSender>,
}

impl Reaction {
    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn has_sender(&self, sender_id: &str) -> bool {
        self.senders
            .iter()
            .any(|s| s.sender_id == sender_id && s.count > 0)
    }

    fn refresh(&mut self, cfg: Option<&AppConfig>) {
        self.count = self.senders.iter().map(|s| s.count).sum();
        self.count_label = format_reaction_count(self.count).into();
        self.emoji_proxied = cfg
            .map(|c| c.emoji_src(&self.emoji_id))
            .unwrap_or_default()
            .into();
    }
}

pub fn format_reaction_count(count: u32) -> String {
    if count < 1000 {
        return count.to_string();
    }
    const UNITS: [&str; 5] = ["", "K", "M", "G", "T"];
    let unit_index = (((count as f64).log10() / 3.0).floor() as usize).min(UNITS.len() - 1);
    let value = count as f64 / 1000f64.powi(unit_index as i32);
    format!("{:.1}{}", value, UNITS[unit_index])
}

pub fn reaction_key<'a>(emoji_id: &'a str, emoji: &'a str) -> &'a str {
    if !emoji_id.is_empty() && emoji_id != "0" {
        emoji_id
    } else {
        emoji
    }
}

fn upsert_sender(senders: &mut Vec<ReactionSender>, sender_id: &str, count: u32, set: bool) {
    match senders.iter_mut().find(|s| s.sender_id == sender_id) {
        Some(s) => {
            s.count = if set {
                count
            } else {
                s.count.saturating_add(count)
            };
        }
        None => senders.push(ReactionSender {
            sender_id: sender_id.to_string(),
            count,
        }),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MentionTarget {
    pub user_id: Option<String>,
    pub role_id: Option<String>,
}

/// An inline rich-text span produced from a message's content tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageSpan {
    Text(SharedString),
    Bold(SharedString),
    Code(SharedString),
    CodeBlock {
        language: Option<String>,
        text: SharedString,
    },
    Link {
        text: SharedString,
        url: String,
    },
    Mention {
        display: SharedString,
        user_id: Option<String>,
        role_id: Option<String>,
    },
    Hashtag {
        display: SharedString,
        channel_id: Option<String>,
    },
    Emoji {
        name: SharedString,
        emoji_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OgpPreview {
    pub url: String,
    pub title: SharedString,
    pub description: SharedString,
    pub image_proxied: SharedString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollLabelSegment {
    Text(SharedString),
    Emoji(SharedString),
}

#[derive(Debug, Clone)]
pub struct PollAnswerView {
    pub index: i32,
    pub label: SharedString,
    pub segments: Vec<PollLabelSegment>,
}

#[derive(Debug, Clone)]
pub struct PollData {
    pub poll_id: i64,
    pub question: SharedString,
    pub answers: Vec<PollAnswerView>,
    pub answer_counts: Vec<i32>,
    pub total_votes: i32,
    pub percentages: Vec<u8>,
    pub expire_at: Option<i64>,
    pub is_closed: bool,
    pub allow_multiple: bool,
}

impl PollData {
    pub fn is_expired(&self, now_secs: i64) -> bool {
        self.expire_at.is_some_and(|exp| exp > 0 && exp < now_secs)
    }
}

#[derive(Debug, Clone)]
pub struct PollVoter {
    pub user_id: UserId,
    pub display_name: SharedString,
    pub username: SharedString,
    pub avatar_proxied: SharedString,
}


#[derive(Debug, Clone)]
pub struct PollDetail {
    pub total_votes: i32,
    pub answer_counts: Vec<i32>,
    pub voters_by_answer: Vec<Vec<PollVoter>>,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: MessageId,
    /// Stable GPUI row key — kept at the optimistic temp id through ack reconcile.
    pub row_anchor_id: MessageId,
    pub content: String,
    pub sender_id: String,
    pub sender_user_id: Option<UserId>,
    pub sender_name: SharedString,
    pub avatar_url: SharedString,
    pub avatar_proxied: SharedString,
    pub create_time: i64,
    pub update_time: i64,
    pub day_label: String,
    pub time_hhmm: SharedString,
    pub local_date: Option<chrono::NaiveDate>,
    pub code: MessageCode,
    pub is_edited: bool,
    pub is_forwarded: bool,
    pub show_forwarded_label: bool,
    pub combined_with_prev: bool,
    pub highlights_viewer_direct: bool,
    pub ogp: Option<OgpPreview>,
    pub poll: Option<PollData>,
    pub spans: Vec<MessageSpan>,
    pub mention_targets: Vec<MentionTarget>,
    pub references: Vec<MessageReference>,
    pub reactions: Vec<Reaction>,
    pub attachments: Vec<MessageAttachment>,
    pub album_layout: Option<AlbumLayout>,
    pub viewer_media: Arc<[ViewerMedia]>,
}

#[derive(Debug, Clone)]
pub struct ViewerMedia {
    pub url: SharedString,
    pub filename: SharedString,
    pub viewer_src: SharedString,
}

pub const COMBINE_TIME_WINDOW: i64 = 600;

pub fn same_message_sender(a: &Message, b: &Message) -> bool {
    if let (Some(au), Some(bu)) = (resolved_sender_user_id(a), resolved_sender_user_id(b))
        && au == bu
    {
        return true;
    }
    let a_id = a.sender_id.as_str();
    let b_id = b.sender_id.as_str();
    !a_id.is_empty() && a_id != "0" && !b_id.is_empty() && b_id != "0" && a_id == b_id
}

fn resolved_sender_user_id(m: &Message) -> Option<UserId> {
    if let Some(uid) = m.sender_user_id.filter(|u| u.0 != 0) {
        return Some(uid);
    }
    if m.sender_id.is_empty() || m.sender_id == "0" {
        return None;
    }
    m.sender_id
        .parse::<i64>()
        .ok()
        .filter(|&id| id != 0)
        .map(UserId)
}

pub fn message_combined_with_prev(prev: Option<&Message>, msg: &Message) -> bool {
    if !msg.code.is_user_timeline() {
        return false;
    }
    let Some(prev) = prev else {
        return false;
    };
    if msg.create_time == 0 {
        return false;
    }
    let delta = msg.create_time - prev.create_time;
    same_message_sender(prev, msg) && delta < COMBINE_TIME_WINDOW
}

pub fn should_show_message_head(msg: &Message, is_combine: bool) -> bool {
    !msg.references.is_empty() || !is_combine
}

pub fn message_row_highlight(msg: &Message, viewer_id: Option<UserId>, role_ids: &[i64]) -> bool {
    viewer_highlight_direct(&msg.references, &msg.mention_targets, &msg.spans, viewer_id)
        || message_row_highlight_roles(msg, role_ids)
}

pub(crate) fn viewer_highlight_direct(
    references: &[MessageReference],
    mention_targets: &[MentionTarget],
    spans: &[MessageSpan],
    viewer_id: Option<UserId>,
) -> bool {
    let Some(viewer_id) = viewer_id else {
        return false;
    };
    if references
        .iter()
        .any(|reference| reference.sender_id == viewer_id)
    {
        return true;
    }
    if mention_targets.iter().any(|target| {
        target
            .user_id
            .as_deref()
            .is_some_and(|uid| mention_user_id_targets_viewer(uid, viewer_id))
    }) {
        return true;
    }
    spans.iter().any(|span| {
        matches!(span, MessageSpan::Mention { user_id, .. }
            if user_id
                .as_deref()
                .is_some_and(|uid| mention_user_id_targets_viewer(uid, viewer_id)))
    })
}

pub fn message_row_highlight_roles(msg: &Message, role_ids: &[i64]) -> bool {
    if role_ids.is_empty() {
        return false;
    }
    if msg.mention_targets.iter().any(|target| {
        target.role_id.as_deref().is_some_and(|rid| {
            rid.parse::<i64>()
                .ok()
                .is_some_and(|id| role_ids.contains(&id))
        })
    }) {
        return true;
    }
    msg.spans.iter().any(|span| {
        matches!(span, MessageSpan::Mention { role_id, .. }
            if role_id.as_deref().is_some_and(|r| !r.is_empty())
                && role_id
                    .as_deref()
                    .and_then(|r| r.parse::<i64>().ok())
                    .is_some_and(|id| role_ids.contains(&id)))
    })
}

fn mention_user_id_targets_viewer(uid: &str, viewer_id: UserId) -> bool {
    !uid.is_empty()
        && uid != "0"
        && (mezon_client::transport::is_here_user_id(uid)
            || uid.parse::<i64>().ok().map(UserId) == Some(viewer_id))
}

pub fn aggregate_reactions(raw: &[ApiMessageReaction], cfg: Option<&AppConfig>) -> Vec<Reaction> {
    let mut out: Vec<Reaction> = Vec::new();
    for r in raw {
        if r.action {
            continue;
        }
        let key = reaction_key(&r.emoji_id, &r.emoji);
        if key.is_empty() {
            continue;
        }
        let count = if r.count == 0 { 1 } else { r.count };
        let idx = match out.iter().position(|x| x.key == key) {
            Some(i) => i,
            None => {
                out.push(Reaction {
                    key: key.to_string(),
                    emoji: r.emoji.clone().into(),
                    emoji_id: r.emoji_id.clone().into(),
                    ..Default::default()
                });
                out.len() - 1
            }
        };
        upsert_sender(&mut out[idx].senders, &r.sender_id, count, true);
    }
    for r in out.iter_mut() {
        r.refresh(cfg);
    }
    out.retain(|r| r.count > 0);
    out
}

pub fn apply_reaction_event(
    reactions: &mut Vec<Reaction>,
    emoji_id: &str,
    emoji: &str,
    sender_id: &str,
    removed: bool,
    cfg: Option<&AppConfig>,
) {
    let key = reaction_key(emoji_id, emoji);
    if key.is_empty() {
        return;
    }
    if removed {
        if let Some(pos) = reactions.iter().position(|x| x.key == key) {
            reactions[pos].senders.retain(|s| s.sender_id != sender_id);
            reactions[pos].refresh(cfg);
            if reactions[pos].count == 0 {
                reactions.remove(pos);
            }
        }
    } else if let Some(rec) = reactions.iter_mut().find(|x| x.key == key) {
        upsert_sender(&mut rec.senders, sender_id, 1, false);
        rec.refresh(cfg);
    } else {
        let mut rec = Reaction {
            key: key.to_string(),
            emoji: emoji.into(),
            emoji_id: emoji_id.into(),
            senders: vec![ReactionSender {
                sender_id: sender_id.to_string(),
                count: 1,
            }],
            ..Default::default()
        };
        rec.refresh(cfg);
        reactions.push(rec);
    }
}

pub fn rollback_reaction(
    reactions: &mut Vec<Reaction>,
    emoji_id: &str,
    emoji: &str,
    sender_id: &str,
    was_remove: bool,
    cfg: Option<&AppConfig>,
) {
    if was_remove {
        apply_reaction_event(reactions, emoji_id, emoji, sender_id, false, cfg);
        return;
    }
    let key = reaction_key(emoji_id, emoji);
    let Some(pos) = reactions.iter().position(|x| x.key == key) else {
        return;
    };
    if let Some(s) = reactions[pos]
        .senders
        .iter_mut()
        .find(|s| s.sender_id == sender_id)
    {
        s.count = s.count.saturating_sub(1);
    }
    reactions[pos].senders.retain(|s| s.count > 0);
    reactions[pos].refresh(cfg);
    if reactions[pos].count == 0 {
        reactions.remove(pos);
    }
}

pub fn parse_spans(content: &ApiMessageContent) -> Vec<MessageSpan> {
    let text = &content.t;
    if text.is_empty() {
        return Vec::new();
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    let total = units.len() as i64;
    let slice = |s: i64, e: i64| -> String {
        let s = s.clamp(0, total) as usize;
        let e = e.clamp(0, total) as usize;
        if e <= s {
            return String::new();
        }
        String::from_utf16_lossy(&units[s..e])
    };

    #[derive(Clone, Copy)]
    enum Kind {
        Mention,
        Hashtag,
        Emoji,
        Markdown,
        Link,
    }
    let mut toks: Vec<(i64, i64, Kind, ContentToken)> = Vec::new();
    let collect =
        |list: &[ContentToken], kind: Kind, toks: &mut Vec<(i64, i64, Kind, ContentToken)>| {
            for t in list {
                let s = t.s.unwrap_or(0);
                let e = t.e.unwrap_or(0);
                if e > s {
                    toks.push((s, e, kind, t.clone()));
                }
            }
        };
    collect(&content.mentions, Kind::Mention, &mut toks);
    collect(&content.hg, Kind::Hashtag, &mut toks);
    collect(&content.ej, Kind::Emoji, &mut toks);
    collect(&content.mk, Kind::Markdown, &mut toks);
    collect(&content.lk, Kind::Link, &mut toks);
    toks.sort_by_key(|t| t.0);

    let mut spans: Vec<MessageSpan> = Vec::new();
    let mut last = 0i64;
    let mut prev_end = i64::MIN;
    for (s, e, kind, tok) in toks {
        if s < prev_end {
            continue;
        }
        if last < s {
            spans.push(MessageSpan::Text(slice(last, s).into()));
        }
        let inner = slice(s, e);
        match kind {
            Kind::Mention => {
                let is_user = tok
                    .user_id
                    .as_deref()
                    .is_some_and(|u| u != "0" && !u.is_empty())
                    || tok.username.is_some();
                if is_user {
                    spans.push(MessageSpan::Mention {
                        display: inner.into(),
                        user_id: tok.user_id.clone(),
                        role_id: None,
                    });
                } else if tok.role_id.as_deref().is_some_and(|r| !r.is_empty()) {
                    spans.push(MessageSpan::Mention {
                        display: inner.into(),
                        user_id: None,
                        role_id: tok.role_id.clone(),
                    });
                } else {
                    spans.push(MessageSpan::Text(inner.into()));
                }
            }
            Kind::Hashtag => spans.push(MessageSpan::Hashtag {
                display: inner.into(),
                channel_id: tok.channel_id.clone(),
            }),
            Kind::Emoji => spans.push(MessageSpan::Emoji {
                name: inner.into(),
                emoji_id: tok.emojiid.clone().unwrap_or_default(),
            }),
            Kind::Link => {
                let url = tok.url.clone().unwrap_or_else(|| inner.clone());
                spans.push(MessageSpan::Link {
                    text: inner.into(),
                    url,
                });
            }
            Kind::Markdown => {
                let ty = tok.kind.as_deref().unwrap_or("");
                match ty {
                    "b" => spans.push(MessageSpan::Bold(strip_marker(&inner, "**").into())),
                    "c" | "s" => spans.push(MessageSpan::Code(strip_marker(&inner, "`").into())),
                    "t" | "pre" => spans.push(MessageSpan::CodeBlock {
                        language: None,
                        text: strip_marker(&inner, "```").into(),
                    }),
                    "lk" | "lk_yt" | "lk_fb" | "lk_tt" => {
                        let url = tok.url.clone().unwrap_or_else(|| inner.clone());
                        spans.push(MessageSpan::Link {
                            text: inner.into(),
                            url,
                        });
                    }
                    "lk_ogp" => {}
                    _ => spans.push(MessageSpan::Text(inner.into())),
                }
            }
        }
        prev_end = e;
        last = e;
    }
    if last < total {
        spans.push(MessageSpan::Text(slice(last, total).into()));
    }
    spans
}

fn strip_marker(s: &str, marker: &str) -> String {
    let trimmed = s
        .strip_prefix(marker)
        .and_then(|r| r.strip_suffix(marker))
        .unwrap_or(s);
    trimmed.to_string()
}

pub fn recompute_message_grouping(messages: &mut [Message]) {
    for i in 0..messages.len() {
        let prev = if i > 0 { Some(&messages[i - 1]) } else { None };
        let combined = message_combined_with_prev(prev, &messages[i]);
        let show_forwarded_label = compute_show_forwarded_label(prev, &messages[i]);
        messages[i].combined_with_prev = combined;
        messages[i].show_forwarded_label = show_forwarded_label;
    }
}

fn compute_show_forwarded_label(prev: Option<&Message>, msg: &Message) -> bool {
    if !msg.is_forwarded {
        return false;
    }
    let Some(prev) = prev else {
        return true;
    };
    if !prev.is_forwarded {
        return true;
    }
    !same_message_sender(prev, msg) || (msg.create_time - prev.create_time).abs() > 600_000
}


pub fn message_sort_key(m: &Message) -> (u8, i64) {
    let not_first = u8::from(m.code != MessageCode::Indicator);
    (not_first, m.id.get())
}

pub fn sort_messages(messages: &mut [Message]) {
    messages.sort_by_key(message_sort_key);
}

impl Message {
    pub fn new(
        id: MessageId,
        content: impl Into<String>,
        sender_id: impl Into<String>,
        sender_name: impl Into<SharedString>,
        create_time: i64,
    ) -> Self {
        let content: String = content.into();
        let spans = if content.is_empty() {
            Vec::new()
        } else {
            vec![MessageSpan::Text(content.as_str().into())]
        };
        let sender_id: String = sender_id.into();
        let sender_user_id = sender_id.parse::<i64>().ok().map(UserId);
        Self {
            id,
            row_anchor_id: id,
            content,
            sender_id,
            sender_user_id,
            sender_name: sender_name.into(),
            avatar_url: SharedString::default(),
            avatar_proxied: SharedString::default(),
            create_time,
            update_time: 0,
            day_label: local_day_key(create_time),
            time_hhmm: format_local_time_hhmm(create_time).into(),
            local_date: local_datetime(create_time).map(|dt| dt.date_naive()),
            code: MessageCode::Chat,
            is_edited: false,
            is_forwarded: false,
            show_forwarded_label: false,
            combined_with_prev: false,
            highlights_viewer_direct: false,
            ogp: None,
            poll: None,
            spans,
            mention_targets: Vec::new(),
            references: Vec::new(),
            reactions: Vec::new(),
            attachments: Vec::new(),
            album_layout: None,
            viewer_media: Vec::new().into(),
        }
    }

    pub fn with_attachments(mut self, attachments: Vec<MessageAttachment>) -> Self {
        self.attachments = attachments;
        self
    }

    pub fn with_media_presentation(
        mut self,
        album_layout: Option<AlbumLayout>,
        viewer_media: Arc<[ViewerMedia]>,
    ) -> Self {
        self.album_layout = album_layout;
        self.viewer_media = viewer_media;
        self
    }

    pub fn with_code(mut self, code: MessageCode) -> Self {
        self.code = code;
        self
    }

    pub fn with_spans(mut self, spans: Vec<MessageSpan>) -> Self {
        self.spans = spans;
        self
    }

    pub fn with_mention_targets(mut self, mention_targets: Vec<MentionTarget>) -> Self {
        self.mention_targets = mention_targets;
        self
    }

    pub fn with_viewer_highlight(mut self, highlights_viewer_direct: bool) -> Self {
        self.highlights_viewer_direct = highlights_viewer_direct;
        self
    }

    pub fn with_references(mut self, references: Vec<MessageReference>) -> Self {
        self.references = references;
        self
    }

    pub fn with_reactions(mut self, reactions: Vec<Reaction>) -> Self {
        self.reactions = reactions;
        self
    }

    pub fn with_edited(mut self, update_time: i64, hide_editted: bool) -> Self {
        self.update_time = update_time;
        self.is_edited = update_time > 0 && update_time > self.create_time && !hide_editted;
        self
    }

    pub fn with_forwarded(mut self, forwarded: bool) -> Self {
        self.is_forwarded = forwarded;
        self
    }

    pub fn with_ogp(mut self, ogp: Option<OgpPreview>) -> Self {
        self.ogp = ogp;
        self
    }

    pub fn with_poll(mut self, poll: Option<PollData>) -> Self {
        self.poll = poll;
        self
    }

    pub fn with_avatar(mut self, avatar_url: impl Into<SharedString>) -> Self {
        self.avatar_url = avatar_url.into();
        self
    }

    pub fn with_avatar_proxied(mut self, proxied: impl Into<SharedString>) -> Self {
        self.avatar_proxied = proxied.into();
        self
    }
}
