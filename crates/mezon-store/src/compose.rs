use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{App, AppContext, Entity, Global};

use crate::ids::ChannelId;

pub const MAX_DRAFT_CHANNELS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeTokenKind {
    Mention { user_id: String, role_id: String },
    Hashtag { channel_id: String },
    Emoji { emoji_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeToken {
    pub kind: ComposeTokenKind,
    pub display: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct PendingAttachment {
    pub path: PathBuf,
    pub filename: String,
    pub filetype: String,
    pub size: u64,
    pub is_image: bool,
    pub is_video: bool,
    pub width: u32,
    pub height: u32,
    pub duration: i32,
    pub poster_jpeg: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default)]
pub struct ComposeDraft {
    pub text: String,
    pub tokens: Vec<ComposeToken>,
    pub attachments: Vec<PendingAttachment>,
}

impl ComposeDraft {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.attachments.is_empty()
    }
}

pub struct ComposeStore {
    by_channel: HashMap<ChannelId, ComposeDraft>,
    recent: Vec<ChannelId>,
}

struct GlobalComposeStore(Entity<ComposeStore>);
impl Global for GlobalComposeStore {}

impl ComposeStore {
    pub fn init(cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|_| Self::new());
        cx.set_global(GlobalComposeStore(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalComposeStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalComposeStore>()
            .map(|global| global.0.clone())
    }

    fn new() -> Self {
        Self {
            by_channel: HashMap::new(),
            recent: Vec::new(),
        }
    }

    pub fn draft(&self, channel_id: ChannelId) -> Option<&ComposeDraft> {
        self.by_channel.get(&channel_id)
    }

    pub fn set_draft(&mut self, channel_id: ChannelId, draft: ComposeDraft) {
        if draft.is_empty() {
            self.clear_draft(channel_id);
            return;
        }
        self.by_channel.insert(channel_id, draft);
        self.touch(channel_id);
        self.evict_oldest();
    }

    pub fn take_draft(&mut self, channel_id: ChannelId) -> Option<ComposeDraft> {
        self.recent.retain(|id| *id != channel_id);
        self.by_channel.remove(&channel_id)
    }

    pub fn clear_draft(&mut self, channel_id: ChannelId) {
        if self.by_channel.remove(&channel_id).is_some() {
            self.recent.retain(|id| *id != channel_id);
        }
    }

    pub fn clear_all(&mut self) {
        self.by_channel.clear();
        self.recent.clear();
    }

    pub fn len(&self) -> usize {
        self.by_channel.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_channel.is_empty()
    }

    fn touch(&mut self, channel_id: ChannelId) {
        self.recent.retain(|id| *id != channel_id);
        self.recent.push(channel_id);
    }

    fn evict_oldest(&mut self) {
        while self.recent.len() > MAX_DRAFT_CHANNELS {
            let oldest = self.recent.remove(0);
            self.by_channel.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_draft(text: &str) -> ComposeDraft {
        ComposeDraft {
            text: text.to_string(),
            ..Default::default()
        }
    }

    fn attachment() -> PendingAttachment {
        PendingAttachment {
            path: PathBuf::from("a.png"),
            filename: "a.png".to_string(),
            filetype: "image/png".to_string(),
            size: 10,
            is_image: true,
            is_video: false,
            width: 1,
            height: 1,
            duration: 0,
            poster_jpeg: None,
        }
    }

    #[test]
    fn draft_round_trips_per_channel() {
        let mut store = ComposeStore::new();
        store.set_draft(ChannelId(1), text_draft("hello"));
        store.set_draft(ChannelId(2), text_draft("world"));

        assert_eq!(
            store.draft(ChannelId(1)).map(|d| d.text.as_str()),
            Some("hello")
        );
        assert_eq!(
            store.draft(ChannelId(2)).map(|d| d.text.as_str()),
            Some("world")
        );
        assert!(store.draft(ChannelId(3)).is_none());
    }

    #[test]
    fn blank_draft_clears_instead_of_storing() {
        let mut store = ComposeStore::new();
        store.set_draft(ChannelId(1), text_draft("hello"));
        store.set_draft(ChannelId(1), text_draft("   \n "));

        assert!(store.draft(ChannelId(1)).is_none());
        assert!(store.is_empty());
    }

    #[test]
    fn blank_text_with_attachment_is_kept() {
        let mut store = ComposeStore::new();
        store.set_draft(
            ChannelId(1),
            ComposeDraft {
                text: String::new(),
                tokens: Vec::new(),
                attachments: vec![attachment()],
            },
        );

        assert_eq!(
            store.draft(ChannelId(1)).map(|d| d.attachments.len()),
            Some(1)
        );
    }

    #[test]
    fn take_draft_removes_it() {
        let mut store = ComposeStore::new();
        store.set_draft(ChannelId(1), text_draft("hello"));

        assert_eq!(
            store.take_draft(ChannelId(1)).map(|d| d.text),
            Some("hello".to_string())
        );
        assert!(store.draft(ChannelId(1)).is_none());
        assert!(store.take_draft(ChannelId(1)).is_none());
    }

    #[test]
    fn tokens_survive_the_round_trip() {
        let mut store = ComposeStore::new();
        store.set_draft(
            ChannelId(1),
            ComposeDraft {
                text: "hi @bob".to_string(),
                tokens: vec![ComposeToken {
                    kind: ComposeTokenKind::Mention {
                        user_id: "42".to_string(),
                        role_id: String::new(),
                    },
                    display: "@bob".to_string(),
                    start: 3,
                    end: 7,
                }],
                attachments: Vec::new(),
            },
        );

        let tokens = &store.draft(ChannelId(1)).expect("draft").tokens;
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].start, 3);
        assert_eq!(tokens[0].end, 7);
    }

    #[test]
    fn evicts_least_recently_updated_channel_at_cap() {
        let mut store = ComposeStore::new();
        for id in 0..MAX_DRAFT_CHANNELS as i64 {
            store.set_draft(ChannelId(id), text_draft("draft"));
        }
        store.set_draft(ChannelId(0), text_draft("refreshed"));
        store.set_draft(ChannelId(999), text_draft("newest"));

        assert_eq!(store.len(), MAX_DRAFT_CHANNELS);
        assert!(store.draft(ChannelId(1)).is_none());
        assert_eq!(
            store.draft(ChannelId(0)).map(|d| d.text.as_str()),
            Some("refreshed")
        );
        assert_eq!(
            store.draft(ChannelId(999)).map(|d| d.text.as_str()),
            Some("newest")
        );
    }

    #[test]
    fn clear_all_empties_the_store() {
        let mut store = ComposeStore::new();
        store.set_draft(ChannelId(1), text_draft("hello"));
        store.clear_all();

        assert!(store.is_empty());
        assert!(store.draft(ChannelId(1)).is_none());
    }
}
