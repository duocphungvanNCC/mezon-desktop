use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, AppContext, Context, Entity, Global, Task};
use mezon_client::{AppApi, RealtimeEvent};
use mezon_stream::{StreamEvent, StreamSession, StreamSessionConfig};
use mezon_voice::VideoFrameStore;

use crate::AppConfig;
use crate::AuthState;
use crate::ChannelType;
use crate::ids::{ChannelId, ClanId, UserId};
use crate::realtime::{RealtimeDispatch, RealtimeKind};

const STREAMING_CHANNEL_TYPE: i32 = 6;
const STREAM_MEMBER_STATE_ACTIVE: i32 = 1;
const STREAM_FETCH_LIMIT: i32 = 100;
const JOIN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamMember {
    pub user_id: UserId,
    pub channel_id: ChannelId,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamPhase {
    Idle,
    Joining {
        channel_id: ChannelId,
        clan_id: ClanId,
    },
    Joined {
        channel_id: ChannelId,
        clan_id: ClanId,
        is_live: bool,
    },
    Error(String),
}

pub struct StreamStore {
    api: Arc<AppApi>,
    phase: StreamPhase,
    members: HashMap<(ChannelId, UserId), StreamMember>,
    active_clan: Option<ClanId>,
    show_chat: bool,
    show_members: bool,
    controls_visible: bool,
    remote_video: bool,
    playback_blocked: bool,
    volume: f32,
    muted: bool,
    fullscreen: bool,
    error_message: Option<String>,
    frame_store: Arc<VideoFrameStore>,
    session: Option<StreamSession>,
    session_channel_label: String,
    session_clan_name: String,
    join_started: Option<Instant>,
    _fetch_task: Option<Task<()>>,
    _session_task: Option<Task<()>>,
    _join_timeout: Option<Task<()>>,
    _controls_hide_task: Option<Task<()>>,
}

struct GlobalStreamStore(Entity<StreamStore>);
impl Global for GlobalStreamStore {}

impl StreamStore {
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(api, cx));
        cx.set_global(GlobalStreamStore(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalStreamStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalStreamStore>().map(|g| g.0.clone())
    }

    fn new(api: Arc<AppApi>, cx: &mut Context<Self>) -> Self {
        Self::register_realtime(cx);
        Self {
            api,
            phase: StreamPhase::Idle,
            members: HashMap::new(),
            active_clan: None,
            show_chat: false,
            show_members: true,
            controls_visible: true,
            remote_video: false,
            playback_blocked: false,
            volume: 1.0,
            muted: false,
            fullscreen: false,
            error_message: None,
            frame_store: Arc::new(VideoFrameStore::default()),
            session: None,
            session_channel_label: String::new(),
            session_clan_name: String::new(),
            join_started: None,
            _fetch_task: None,
            _session_task: None,
            _join_timeout: None,
            _controls_hide_task: None,
        }
    }

    fn register_realtime(cx: &mut Context<Self>) {
        let entity = cx.entity();
        RealtimeDispatch::global(cx).update(cx, |dispatch, _| {
            dispatch.on(RealtimeKind::StreamingJoined, &entity, |this, event, cx| {
                this.handle_streaming_joined(event, cx)
            });
            dispatch.on(RealtimeKind::StreamingLeaved, &entity, |this, event, cx| {
                this.handle_streaming_leaved(event, cx)
            });
            dispatch.on(
                RealtimeKind::StreamingStarted,
                &entity,
                |this, event, cx| this.handle_streaming_started(event, cx),
            );
            dispatch.on(RealtimeKind::StreamingEnded, &entity, |this, event, cx| {
                this.handle_streaming_ended(event, cx)
            });
        });
    }

    pub fn phase(&self) -> &StreamPhase {
        &self.phase
    }

    pub fn is_joined(&self) -> bool {
        matches!(self.phase, StreamPhase::Joined { .. })
    }

    pub fn is_joining(&self) -> bool {
        matches!(self.phase, StreamPhase::Joining { .. })
    }

    pub fn session_channel_id(&self) -> Option<ChannelId> {
        match &self.phase {
            StreamPhase::Joining { channel_id, .. } | StreamPhase::Joined { channel_id, .. } => {
                Some(*channel_id)
            }
            _ => None,
        }
    }

    pub fn should_leave_for_active_channel(
        &self,
        active: Option<(ChannelType, ChannelId)>,
    ) -> bool {
        if !self.is_joined() && !self.is_joining() {
            return false;
        }
        let Some(session_channel) = self.session_channel_id() else {
            return false;
        };
        match active {
            Some((ChannelType::Stream, channel_id)) => session_channel != channel_id,
            _ => false,
        }
    }

    pub fn show_chat(&self) -> bool {
        self.show_chat
    }

    pub fn show_members(&self) -> bool {
        self.show_members
    }

    pub fn controls_visible(&self) -> bool {
        self.controls_visible
    }

    pub fn bump_controls_visible(&mut self, cx: &mut Context<Self>) {
        let was_visible = self.controls_visible;
        self.controls_visible = true;
        self._controls_hide_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            this.update(cx, |this, cx| {
                this.controls_visible = false;
                cx.notify();
            })
            .ok();
        }));
        if !was_visible {
            cx.notify();
        }
    }

    pub fn remote_video(&self) -> bool {
        self.remote_video
    }

    pub fn playback_blocked(&self) -> bool {
        self.playback_blocked
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    pub fn effective_volume(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume }
    }

    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    pub fn frame_store(&self) -> &Arc<VideoFrameStore> {
        &self.frame_store
    }

    pub fn session_channel_label(&self) -> &str {
        &self.session_channel_label
    }

    pub fn session_clan_name(&self) -> &str {
        &self.session_clan_name
    }

    pub fn members_for_channel(&self, channel_id: ChannelId) -> Vec<StreamMember> {
        self.members
            .values()
            .filter(|m| m.channel_id == channel_id)
            .cloned()
            .collect()
    }

    pub fn set_active_clan(&mut self, clan_id: Option<ClanId>, cx: &mut Context<Self>) {
        if self.active_clan == clan_id {
            return;
        }
        self.active_clan = clan_id;
        if let Some(clan_id) = clan_id {
            self.fetch_members(clan_id, cx);
        }
    }

    pub fn fetch_members(&mut self, clan_id: ClanId, cx: &mut Context<Self>) {
        let api = self.api.clone();
        self._fetch_task = Some(cx.spawn(async move |this, cx| {
            let result = api
                .list_streaming_channel_users(
                    clan_id.get(),
                    0,
                    STREAMING_CHANNEL_TYPE,
                    STREAM_MEMBER_STATE_ACTIVE,
                    STREAM_FETCH_LIMIT,
                )
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(list) => {
                        this.members.clear();
                        for user in list.streaming_channel_users {
                            let user_id = UserId(user.user_id);
                            let channel_id = ChannelId(user.channel_id);
                            let display_name = if user.participant.is_empty() {
                                user_id.to_string()
                            } else {
                                user.participant.clone()
                            };
                            this.members.insert(
                                (channel_id, user_id),
                                StreamMember {
                                    user_id,
                                    channel_id,
                                    display_name,
                                },
                            );
                        }
                        if let Some(store) = crate::ClanMembersStore::try_global(cx) {
                            store.update(cx, |store, cx| store.ensure_loaded(clan_id, cx));
                        }
                        if let Some(store) = crate::UsersByUserStore::try_global(cx) {
                            store.update(cx, |store, cx| store.ensure_loaded(cx));
                        }
                    }
                    Err(err) => tracing::warn!("list_streaming_channel_users failed: {err:#}"),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn toggle_chat(&mut self, cx: &mut Context<Self>) {
        self.show_chat = !self.show_chat;
        cx.notify();
    }

    pub fn toggle_members(&mut self, cx: &mut Context<Self>) {
        self.show_members = !self.show_members;
        cx.notify();
    }

    pub fn set_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        self.volume = volume.clamp(0.0, 1.0);
        if self.volume > 0.0 {
            self.muted = false;
        } else {
            self.muted = true;
        }
        self.sync_audio_playback();
        cx.notify();
    }

    pub fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        self.muted = !self.muted;
        self.sync_audio_playback();
        cx.notify();
    }

    pub fn toggle_fullscreen(&mut self, cx: &mut Context<Self>) {
        self.fullscreen = !self.fullscreen;
        cx.notify();
    }

    pub fn clear_fullscreen(&mut self, cx: &mut Context<Self>) {
        if self.fullscreen {
            self.fullscreen = false;
            cx.notify();
        }
    }

    fn sync_audio_playback(&self) {
        if let Some(session) = &self.session {
            session.audio().set_volume(self.volume);
            session.audio().set_muted(self.muted);
        }
    }

    pub fn clear_playback_blocked(&mut self, cx: &mut Context<Self>) {
        self.playback_blocked = false;
        cx.notify();
    }

    pub fn join_stream(
        &mut self,
        clan_id: ClanId,
        channel_id: ChannelId,
        channel_label: &str,
        clan_name: &str,
        auth: &AuthState,
        config: &AppConfig,
        output_device_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let AuthState::Authenticated(session) = auth else {
            self.phase = StreamPhase::Error("Not authenticated".into());
            self.error_message = Some("Not authenticated".into());
            cx.notify();
            return;
        };
        if config.stream_ws_url.is_empty() {
            self.phase = StreamPhase::Error("Stream server not configured".into());
            self.error_message = Some("Stream server not configured".into());
            cx.notify();
            return;
        }
        if self.members_for_channel(channel_id).is_empty() {
            return;
        }

        self.disconnect_session();
        self.session_channel_label = channel_label.to_string();
        self.session_clan_name = clan_name.to_string();
        self.phase = StreamPhase::Joining {
            channel_id,
            clan_id,
        };
        self.join_started = Some(Instant::now());
        self.error_message = None;
        self.remote_video = false;
        self.playback_blocked = false;
        cx.notify();

        let session_config = StreamSessionConfig {
            ws_base_url: config.stream_ws_url.clone(),
            username: session.username.clone(),
            token: session.ws_credential().to_string(),
            clan_id: clan_id.to_string(),
            channel_id: channel_id.to_string(),
            user_id: session.user_id.clone(),
            stream_id: channel_id.to_string(),
        };
        let frame_store = self.frame_store.clone();

        let stream_session = match StreamSession::start(
            session_config,
            frame_store,
            output_device_id,
            self.volume,
            self.muted,
        ) {
            Ok(session) => session,
            Err(err) => {
                self.phase = StreamPhase::Error(err.to_string());
                self.error_message = Some(err.to_string());
                cx.notify();
                return;
            }
        };
        let events = stream_session.events().clone();
        self.session = Some(stream_session);

        let _label = channel_label.to_string();
        self._session_task = Some(cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv_async().await {
                let stop = this
                    .update(cx, |this, cx| {
                        this.handle_stream_event(clan_id, channel_id, event, cx);
                        !this.is_joined() && !this.is_joining()
                    })
                    .unwrap_or(true);
                if stop {
                    break;
                }
            }
        }));

        self._join_timeout = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(JOIN_TIMEOUT).await;
            this.update(cx, |this, cx| {
                if this.is_joining() {
                    this.disconnect_session();
                    this.phase = StreamPhase::Error("Join timed out".into());
                    this.error_message = Some("Join timed out".into());
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    pub fn leave_stream(&mut self, cx: &mut Context<Self>) {
        self.disconnect_session();
        self.phase = StreamPhase::Idle;
        self.error_message = None;
        self.fullscreen = false;
        self.controls_visible = false;
        self._controls_hide_task = None;
        self.session_channel_label.clear();
        self.session_clan_name.clear();
        cx.notify();
    }

    fn disconnect_session(&mut self) {
        if let Some(session) = self.session.take() {
            session.disconnect();
        }
        self._session_task = None;
        self._join_timeout = None;
        self.remote_video = false;
        self.playback_blocked = false;
        self.join_started = None;
        self.frame_store.remove(1);
    }

    fn fail_stream(&mut self, message: String, cx: &mut Context<Self>) {
        self.disconnect_session();
        self.phase = StreamPhase::Error(message.clone());
        self.error_message = Some(message);
        cx.notify();
    }

    fn handle_stream_event(
        &mut self,
        clan_id: ClanId,
        channel_id: ChannelId,
        event: StreamEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            StreamEvent::Live => {
                self._join_timeout = None;
                self.phase = StreamPhase::Joined {
                    channel_id,
                    clan_id,
                    is_live: true,
                };
                self.bump_controls_visible(cx);
            }
            StreamEvent::NoBroadcast => {
                self._join_timeout = None;
                self.phase = StreamPhase::Joined {
                    channel_id,
                    clan_id,
                    is_live: false,
                };
                self.remote_video = false;
                self.bump_controls_visible(cx);
            }
            StreamEvent::RemoteVideo(active) => {
                self.remote_video = active;
                if let StreamPhase::Joined { is_live, .. } = &mut self.phase {
                    *is_live = active;
                }
            }
            StreamEvent::RemoteAudio(_) => {}
            StreamEvent::PlaybackBlocked => {
                self.playback_blocked = true;
            }
            StreamEvent::Error(message) => {
                self.fail_stream(message, cx);
                return;
            }
            StreamEvent::Disconnected => {
                if self.is_joined() || self.is_joining() {
                    self.disconnect_session();
                    self.phase = StreamPhase::Idle;
                }
            }
        }
        cx.notify();
    }

    fn handle_streaming_joined(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        let RealtimeEvent::StreamingJoined(e) = event else {
            return;
        };
        let channel_id = ChannelId(e.streaming_channel_id);
        let user_id = UserId(e.user_id);
        let display_name = if e.participant.is_empty() {
            user_id.to_string()
        } else {
            e.participant.clone()
        };
        self.members.insert(
            (channel_id, user_id),
            StreamMember {
                user_id,
                channel_id,
                display_name,
            },
        );
        cx.notify();
    }

    fn handle_streaming_leaved(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        let RealtimeEvent::StreamingLeaved(e) = event else {
            return;
        };
        let Ok(channel_id) = e.streaming_channel_id.parse::<i64>() else {
            return;
        };
        let channel_id = ChannelId(channel_id);
        if let Ok(user_id) = e.streaming_user_id.parse::<i64>() {
            self.members.remove(&(channel_id, UserId(user_id)));
        }
        cx.notify();
    }

    fn handle_streaming_started(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        let RealtimeEvent::StreamingStarted(e) = event else {
            return;
        };
        let channel_id = ChannelId(e.channel_id);
        if let StreamPhase::Joined {
            channel_id: active,
            is_live,
            ..
        } = &mut self.phase
            && *active == channel_id
        {
            *is_live = e.is_streaming;
            self.remote_video = e.is_streaming;
            cx.notify();
        }
    }

    fn handle_streaming_ended(&mut self, event: &RealtimeEvent, cx: &mut Context<Self>) {
        let RealtimeEvent::StreamingEnded(e) = event else {
            return;
        };
        let channel_id = ChannelId(e.channel_id);
        self.members.retain(|(cid, _), _| *cid != channel_id);
        if let StreamPhase::Joined {
            channel_id: active,
            is_live,
            ..
        } = &mut self.phase
            && *active == channel_id
        {
            *is_live = false;
            self.remote_video = false;
            cx.notify();
        } else {
            cx.notify();
        }
    }

    pub fn on_logout(&mut self, cx: &mut Context<Self>) {
        self.leave_stream(cx);
        self.members.clear();
        self.show_chat = false;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_member(channel_id: ChannelId, user_id: UserId, name: &str) -> StreamMember {
        StreamMember {
            user_id,
            channel_id,
            display_name: name.into(),
        }
    }

    #[test]
    fn members_for_channel_filters_by_id() {
        let ch_a = ChannelId(1);
        let ch_b = ChannelId(2);
        let mut members = HashMap::new();
        members.insert((ch_a, UserId(10)), sample_member(ch_a, UserId(10), "a"));
        members.insert((ch_b, UserId(20)), sample_member(ch_b, UserId(20), "b"));
        let filtered: Vec<_> = members
            .values()
            .filter(|m| m.channel_id == ch_a)
            .cloned()
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].user_id, UserId(10));
    }
}
