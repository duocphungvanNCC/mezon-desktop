use std::collections::{HashMap, HashSet, VecDeque};

use gpui::{App, AppContext, Context, Entity, Global};
use mezon_client::RealtimeEvent;
use mezon_proto::api::ChannelMessage;

use crate::ids::{ChannelId, MessageId};
use crate::message::MessageCode;
use crate::messages::{MessagesStore, viewer_user_id};
use crate::realtime::{RealtimeDispatch, RealtimeKind};

const MAX_PROCESSED_BUZZES: usize = 200;

#[derive(Default)]
struct BuzzMark {
    in_channel: bool,
    topics: HashSet<ChannelId>,
}

impl BuzzMark {
    fn is_empty(&self) -> bool {
        !self.in_channel && self.topics.is_empty()
    }
}

#[derive(Default)]
pub struct BuzzStore {
    marks: HashMap<ChannelId, BuzzMark>,
    processed: HashSet<(ChannelId, MessageId)>,
    processed_order: VecDeque<(ChannelId, MessageId)>,
}

struct GlobalBuzzStore(Entity<BuzzStore>);
impl Global for GlobalBuzzStore {}

impl BuzzStore {
    pub fn init(cx: &mut App) -> Entity<Self> {
        let entity = cx.new(Self::new);
        cx.set_global(GlobalBuzzStore(entity.clone()));
        entity
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let entity = cx.entity();
        RealtimeDispatch::global(cx).update(cx, |dispatch, _| {
            dispatch.on(RealtimeKind::ChannelMessage, &entity, |this, event, cx| {
                this.handle_event(event, cx);
            });
        });
        Self::default()
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalBuzzStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalBuzzStore>().map(|g| g.0.clone())
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.marks.clear();
        self.processed.clear();
        self.processed_order.clear();
        cx.notify();
    }

    pub fn has_buzz(&self, channel_id: ChannelId) -> bool {
        self.marks.contains_key(&channel_id)
    }

    pub fn clear_opened(&mut self, channel_id: ChannelId, cx: &mut Context<Self>) {
        if self.marks.remove(&channel_id).is_some() {
            cx.notify();
        }
    }

    pub fn clear_seen(&mut self, channel_id: ChannelId, cx: &mut Context<Self>) {
        if self.unmark_channel(channel_id) {
            cx.notify();
        }
    }

    pub fn clear_topic_seen(&mut self, topic_id: ChannelId, cx: &mut Context<Self>) {
        if self.unmark_topic(topic_id) {
            cx.notify();
        }
    }

    fn handle_event(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        let RealtimeEvent::ChannelMessage(m) = event else {
            return;
        };
        if self.receive_buzz(m, cx) {
            MessagesStore::global(cx).update(cx, |store, cx| store.play_buzz_sound(cx));
        }
    }

    fn receive_buzz(&mut self, m: &ChannelMessage, cx: &mut Context<Self>) -> bool {
        if MessageCode::from_raw(m.code) != MessageCode::MessageBuzz {
            return false;
        }
        if viewer_user_id(cx).is_some_and(|uid| uid.get() == m.sender_id) {
            return false;
        }
        let (channel_id, topic_id) = buzz_target(m);
        if !self.note_processed(topic_id.unwrap_or(channel_id), MessageId(m.message_id)) {
            return false;
        }
        if !is_buzz_on_screen(channel_id, topic_id, cx) {
            self.mark(channel_id, topic_id);
            cx.notify();
        }
        true
    }

    fn note_processed(&mut self, storage_id: ChannelId, message_id: MessageId) -> bool {
        if message_id.is_zero() {
            return true;
        }
        let key = (storage_id, message_id);
        if !self.processed.insert(key) {
            return false;
        }
        self.processed_order.push_back(key);
        while self.processed_order.len() > MAX_PROCESSED_BUZZES {
            if let Some(evicted) = self.processed_order.pop_front() {
                self.processed.remove(&evicted);
            }
        }
        true
    }

    fn mark(&mut self, channel_id: ChannelId, topic_id: Option<ChannelId>) {
        let mark = self.marks.entry(channel_id).or_default();
        match topic_id {
            Some(topic_id) => {
                mark.topics.insert(topic_id);
            }
            None => mark.in_channel = true,
        }
    }

    fn unmark_channel(&mut self, channel_id: ChannelId) -> bool {
        let Some(mark) = self.marks.get_mut(&channel_id) else {
            return false;
        };
        if !mark.in_channel {
            return false;
        }
        mark.in_channel = false;
        if mark.is_empty() {
            self.marks.remove(&channel_id);
        }
        true
    }

    fn unmark_topic(&mut self, topic_id: ChannelId) -> bool {
        let mut changed = false;
        self.marks.retain(|_, mark| {
            changed |= mark.topics.remove(&topic_id);
            !mark.is_empty()
        });
        changed
    }
}

fn buzz_target(m: &ChannelMessage) -> (ChannelId, Option<ChannelId>) {
    let topic_id = (m.topic_id != 0).then_some(ChannelId(m.topic_id));
    (ChannelId(m.channel_id), topic_id)
}

fn is_buzz_on_screen(channel_id: ChannelId, topic_id: Option<ChannelId>, cx: &App) -> bool {
    if cx.active_window().is_none() {
        return false;
    }
    let messages = MessagesStore::global(cx).read(cx);
    match topic_id {
        Some(topic_id) => messages.active_topic_id() == Some(topic_id),
        None => messages.active_channel_id() == Some(channel_id),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    const VIEWER: i64 = 77;
    const OTHER: i64 = 88;
    const STREAM_MODE_CHANNEL: i32 = 2;
    const STREAM_MODE_GROUP: i32 = 3;
    const STREAM_MODE_DM: i32 = 4;
    const STREAM_MODE_THREAD: i32 = 6;
    const GROUP_CHANNEL_TYPE: i32 = 2;
    const DM_CHANNEL_TYPE: i32 = 3;

    fn channel(id: i64) -> ChannelId {
        ChannelId(id)
    }

    fn init_stores(cx: &mut App) -> (Entity<BuzzStore>, Entity<MessagesStore>) {
        let api = Arc::new(mezon_client::AppApi::new(
            Arc::new(mezon_client::TransportClient::new(String::new())),
            String::new(),
        ));
        RealtimeDispatch::init(api.clone(), cx);
        let auth_state = cx.new(|_| {
            crate::AuthState::Authenticated(mezon_client::Session {
                user_id: VIEWER.to_string(),
                ..Default::default()
            })
        });
        crate::badge::BadgeService::init(auth_state, cx);
        crate::clan::ClanList::init(api.clone(), cx);
        crate::channel::ChannelList::init(api.clone(), cx);
        let messages = MessagesStore::init(api, cx);
        (BuzzStore::init(cx), messages)
    }

    struct Incoming {
        clan_id: i64,
        channel_id: i64,
        topic_id: i64,
        mode: i32,
        code: i32,
        sender_id: i64,
        message_id: i64,
    }

    impl Incoming {
        fn buzz(clan_id: i64, channel_id: i64, mode: i32, message_id: i64) -> Self {
            Self {
                clan_id,
                channel_id,
                topic_id: 0,
                mode,
                code: mezon_client::transport::MESSAGE_BUZZ_CODE,
                sender_id: OTHER,
                message_id,
            }
        }

        fn message(self) -> ChannelMessage {
            ChannelMessage {
                clan_id: self.clan_id,
                channel_id: self.channel_id,
                topic_id: self.topic_id,
                mode: self.mode,
                code: self.code,
                sender_id: self.sender_id,
                message_id: self.message_id,
                ..Default::default()
            }
        }
    }

    fn deliver(store: &Entity<BuzzStore>, incoming: Incoming, cx: &mut App) -> bool {
        let message = incoming.message();
        store.update(cx, |store, cx| store.receive_buzz(&message, cx))
    }

    #[gpui::test]
    fn a_buzz_from_someone_else_marks_its_row_in_every_stream_mode(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let (store, _) = init_stores(cx);
            assert!(deliver(
                &store,
                Incoming::buzz(1, 10, STREAM_MODE_CHANNEL, 101),
                cx
            ));
            assert!(deliver(
                &store,
                Incoming::buzz(1, 11, STREAM_MODE_THREAD, 102),
                cx
            ));
            assert!(deliver(
                &store,
                Incoming::buzz(0, 12, STREAM_MODE_DM, 103),
                cx
            ));
            assert!(deliver(
                &store,
                Incoming::buzz(0, 13, STREAM_MODE_GROUP, 104),
                cx
            ));
            assert!(deliver(
                &store,
                Incoming {
                    topic_id: 99,
                    ..Incoming::buzz(1, 14, STREAM_MODE_CHANNEL, 105)
                },
                cx,
            ));
            assert!(!deliver(
                &store,
                Incoming::buzz(1, 10, STREAM_MODE_CHANNEL, 101),
                cx
            ));
            let store = store.read(cx);
            for row in 10..=14 {
                assert!(
                    store.has_buzz(channel(row)),
                    "row {row} should carry the buzz"
                );
            }
            assert!(!store.has_buzz(channel(99)));
        });
    }

    #[gpui::test]
    fn own_buzzes_and_plain_messages_mark_nothing(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let (store, _) = init_stores(cx);
            assert!(!deliver(
                &store,
                Incoming {
                    sender_id: VIEWER,
                    ..Incoming::buzz(0, 12, STREAM_MODE_DM, 201)
                },
                cx,
            ));
            assert!(!deliver(
                &store,
                Incoming {
                    code: 0,
                    ..Incoming::buzz(0, 13, STREAM_MODE_GROUP, 202)
                },
                cx,
            ));
            assert!(!deliver(
                &store,
                Incoming {
                    code: 0,
                    ..Incoming::buzz(1, 10, STREAM_MODE_CHANNEL, 203)
                },
                cx,
            ));
            assert!(store.read(cx).marks.is_empty());
        });
    }

    #[gpui::test]
    fn opening_a_group_clears_its_buzz_and_a_seen_tail_clears_a_later_one(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let (store, messages) = init_stores(cx);
            assert!(deliver(
                &store,
                Incoming::buzz(0, 13, STREAM_MODE_GROUP, 301),
                cx
            ));
            assert!(deliver(
                &store,
                Incoming::buzz(0, 12, STREAM_MODE_DM, 302),
                cx
            ));
            messages.update(cx, |messages, cx| {
                messages.open_direct(channel(13), GROUP_CHANNEL_TYPE, cx)
            });
            assert!(!store.read(cx).has_buzz(channel(13)));
            assert!(store.read(cx).has_buzz(channel(12)));

            assert!(deliver(
                &store,
                Incoming::buzz(0, 13, STREAM_MODE_GROUP, 303),
                cx
            ));
            assert!(store.read(cx).has_buzz(channel(13)));
            messages.update(cx, |messages, cx| {
                messages.note_viewport_seen(MessageId(303), 1, false, cx)
            });
            assert!(store.read(cx).has_buzz(channel(13)));
            messages.update(cx, |messages, cx| {
                messages.note_viewport_seen(MessageId(303), 1, true, cx)
            });
            assert!(!store.read(cx).has_buzz(channel(13)));
            assert!(store.read(cx).has_buzz(channel(12)));
        });
    }

    #[gpui::test]
    fn seeing_a_topic_clears_the_buzz_it_left_on_its_channel(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let (store, messages) = init_stores(cx);
            messages.update(cx, |messages, cx| {
                messages.open_direct(channel(12), DM_CHANNEL_TYPE, cx)
            });
            assert!(deliver(
                &store,
                Incoming {
                    topic_id: 99,
                    ..Incoming::buzz(1, 14, STREAM_MODE_CHANNEL, 401)
                },
                cx,
            ));
            assert!(store.read(cx).has_buzz(channel(14)));
            messages.update(cx, |messages, cx| {
                messages.note_topic_viewport_seen(channel(99), MessageId(401), 1, true, cx)
            });
            assert!(!store.read(cx).has_buzz(channel(14)));
        });
    }

    #[test]
    fn channel_buzz_is_cleared_once_the_channel_tail_is_seen() {
        let mut store = BuzzStore::default();
        store.mark(channel(1), None);
        assert!(store.has_buzz(channel(1)));
        assert!(store.unmark_channel(channel(1)));
        assert!(!store.has_buzz(channel(1)));
    }

    #[test]
    fn topic_buzz_marks_its_channel_until_the_topic_is_seen() {
        let mut store = BuzzStore::default();
        store.mark(channel(1), Some(channel(9)));
        assert!(store.has_buzz(channel(1)));
        assert!(!store.unmark_channel(channel(1)));
        assert!(store.has_buzz(channel(1)));
        assert!(store.unmark_topic(channel(9)));
        assert!(!store.has_buzz(channel(1)));
    }

    #[test]
    fn seeing_the_channel_keeps_a_pending_topic_buzz() {
        let mut store = BuzzStore::default();
        store.mark(channel(1), None);
        store.mark(channel(1), Some(channel(9)));
        assert!(store.unmark_channel(channel(1)));
        assert!(store.has_buzz(channel(1)));
        assert!(!store.unmark_topic(channel(8)));
        assert!(store.unmark_topic(channel(9)));
        assert!(!store.has_buzz(channel(1)));
    }

    #[test]
    fn a_redelivered_buzz_is_processed_once() {
        let mut store = BuzzStore::default();
        assert!(store.note_processed(channel(1), MessageId(5)));
        assert!(!store.note_processed(channel(1), MessageId(5)));
        assert!(store.note_processed(channel(2), MessageId(5)));
    }

    #[test]
    fn processed_buzzes_stay_bounded() {
        let mut store = BuzzStore::default();
        for id in 1..=(MAX_PROCESSED_BUZZES as i64 + 10) {
            store.note_processed(channel(1), MessageId(id));
        }
        assert_eq!(store.processed.len(), MAX_PROCESSED_BUZZES);
        assert!(store.note_processed(channel(1), MessageId(1)));
    }

    #[test]
    fn buzz_target_splits_topic_from_channel() {
        let in_channel = ChannelMessage {
            channel_id: 7,
            ..Default::default()
        };
        assert_eq!(buzz_target(&in_channel), (channel(7), None));
        let in_topic = ChannelMessage {
            channel_id: 7,
            topic_id: 9,
            ..Default::default()
        };
        assert_eq!(buzz_target(&in_topic), (channel(7), Some(channel(9))));
    }
}
