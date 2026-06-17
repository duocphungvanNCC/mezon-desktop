use std::rc::Rc;
use std::sync::Arc;

use gpui::{Context, Entity, Window, div, prelude::*, px};
use mezon_client::AppApi;
use mezon_store::{AuthState, ChannelList, ClanList, Message, MessageAttachment, Settings};

const MESSAGE_PAGE_LIMIT: u32 = 50;
const DIRECTION_BEFORE: i32 = 3;
const LOAD_MORE_ITEM_THRESHOLD: usize = 6;

use crate::chat_area::ChatArea;
use crate::components::compositions::user_info_bar::UserInfoBar;
use crate::router::{Route, Router};
use crate::theme::{Theme, resolve_theme};
use crate::{ChannelSidebar, ClanSidebar};

fn to_store_attachments(
    attachments: Vec<mezon_client::transport::ApiAttachment>,
) -> Vec<MessageAttachment> {
    attachments
        .into_iter()
        .map(|a| MessageAttachment {
            url: a.url,
            filename: a.filename,
            filetype: a.filetype,
            width: a.width.max(0) as u32,
            height: a.height.max(0) as u32,
        })
        .collect()
}

pub struct ChatLayout {
    settings: Entity<Settings>,
    pub(crate) channel_list: Entity<ChannelList>,
    pub chat_area: ChatArea,
    clan_sidebar: Entity<ClanSidebar>,
    channel_sidebar: Entity<ChannelSidebar>,
    user_info_bar: UserInfoBar,
    /// Guard: kick off the initial clan load only once, on first render.
    loaded: bool,
    pub(crate) api: Arc<AppApi>,
    clan_list: Entity<ClanList>,
    auth_state: Entity<AuthState>,
    last_fetched_channel_id: Option<String>,
    pending_channel_id: Option<String>,
    loading_more: bool,
    has_more: bool,
    list_scroll_installed: bool,
}

impl ChatLayout {
    pub fn new(
        clan_list: Entity<ClanList>,
        auth_state: Entity<AuthState>,
        api: Arc<AppApi>,
        settings: Entity<Settings>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();

        let channel_list = ChannelList::global(cx);

        let clan_list_for_sidebar = clan_list.clone();
        let settings_for_clan = settings.clone();
        let clan_sidebar =
            cx.new(move |cx| ClanSidebar::new(clan_list_for_sidebar, settings_for_clan, cx));

        let clan_list_for_channel = clan_list.clone();
        let channel_list_for_channel = channel_list.clone();
        let settings_for_channel = settings.clone();
        let channel_sidebar = cx.new(move |cx| {
            ChannelSidebar::new(
                clan_list_for_channel,
                channel_list_for_channel,
                settings_for_channel,
                cx,
            )
        });

        let user_info_bar = UserInfoBar::new(auth_state.clone());

        // Channel loading is driven by ChannelList's own subscription to ClanList events
        // (store-to-store, Zed-style). These observes just keep the shell repainting.
        cx.observe(&auth_state, |_, _, cx| cx.notify()).detach();
        cx.observe(&channel_list, |this, _, cx| {
            this.apply_pending_channel(cx);
            cx.notify();
        })
        .detach();
        cx.observe(&clan_list, |_, _, cx| cx.notify()).detach();
        cx.observe(&Router::global(cx), |this, _, cx| {
            this.sync_active_from_route(cx);
            cx.notify();
        })
        .detach();

        {
            let api = api.clone();
            cx.spawn(async move |this, cx| {
                let mut rx = api.subscribe();
                loop {
                    match rx.recv().await {
                        Ok(mezon_client::RealtimeEvent::ChannelMessage(m)) => {
                            let channel_id = m.channel_id.to_string();
                            tracing::info!("onchannelmessage: channel_id={channel_id}");
                            let api_msg = mezon_client::MezonTransport::message_from_proto(m);
                            let result = this.update(cx, |this, cx| {
                                let active = this.channel_list.read(cx).active_channel_id.clone();
                                if active.as_deref() != Some(channel_id.as_str()) {
                                    tracing::info!(
                                        "skip — not the open channel (active={active:?})"
                                    );
                                    return;
                                }
                                let msg = Message::new(
                                    api_msg.message_id,
                                    api_msg.content,
                                    api_msg.sender_id,
                                    api_msg.sender_name,
                                    api_msg.create_time,
                                )
                                .with_avatar(api_msg.avatar)
                                .with_attachments(to_store_attachments(api_msg.attachments));
                                let msgs = Rc::make_mut(&mut this.chat_area.messages);
                                if msgs.iter().any(|x| x.id == msg.id) {
                                    tracing::info!(
                                        "skip duplicate id={} sender_name={}",
                                        msg.id,
                                        msg.sender_name
                                    );
                                    return;
                                }
                                if let Some(slot) = msgs.iter_mut().find(|x| {
                                    x.id.starts_with("temp-")
                                        && x.sender_id == msg.sender_id
                                        && x.content == msg.content
                                }) {
                                    tracing::info!(
                                        "reconciled temp -> id={} sender_name={}",
                                        msg.id,
                                        msg.sender_name
                                    );
                                    *slot = msg;
                                } else {
                                    tracing::info!(
                                        "appended id={} sender_id={} sender_name={}",
                                        msg.id,
                                        msg.sender_id,
                                        msg.sender_name
                                    );
                                    msgs.push(msg);
                                }
                                msgs.sort_by_key(|m| m.create_time);
                                let count = this.chat_area.messages.len();
                                this.chat_area.list_state.reset(count);
                                cx.notify();
                            });
                            if result.is_err() {
                                break; // view dropped
                            }
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            })
            .detach();
        }

        Self {
            settings,
            channel_list,
            chat_area: ChatArea::new(),
            clan_sidebar,
            channel_sidebar,
            user_info_bar,
            loaded: false,
            api,
            clan_list,
            auth_state,
            last_fetched_channel_id: None,
            pending_channel_id: None,
            loading_more: false,
            has_more: true,
            list_scroll_installed: false,
        }
    }

    fn sync_active_from_route(&mut self, cx: &mut Context<Self>) {
        let Route::Channel {
            clan_id,
            channel_id,
        } = Router::global(cx).read(cx).route()
        else {
            self.pending_channel_id = None;
            return;
        };
        if self.clan_list.read(cx).active_clan_id.as_deref() != Some(clan_id.as_str()) {
            self.clan_list
                .update(cx, |clan_list, cx| clan_list.select_clan(&clan_id, cx));
        }
        let (present, already_active) = {
            let channels = self.channel_list.read(cx);
            (
                channels.find_channel(&channel_id).is_some(),
                channels.active_channel_id.as_deref() == Some(channel_id.as_str()),
            )
        };
        if present {
            self.pending_channel_id = None;
            if !already_active {
                self.channel_list.update(cx, |channel_list, cx| {
                    channel_list.select_channel(&channel_id, cx)
                });
            }
        } else {
            self.pending_channel_id = Some(channel_id);
        }
    }

    fn apply_pending_channel(&mut self, cx: &mut Context<Self>) {
        let Some(channel_id) = self.pending_channel_id.clone() else {
            return;
        };
        if self
            .channel_list
            .read(cx)
            .find_channel(&channel_id)
            .is_some()
        {
            self.pending_channel_id = None;
            self.channel_list.update(cx, |channel_list, cx| {
                channel_list.select_channel(&channel_id, cx)
            });
        }
    }

    fn maybe_load_more(&mut self, cx: &mut Context<Self>) {
        if self.has_more && !self.loading_more {
            self.load_more_messages(cx);
        }
    }

    fn load_more_messages(&mut self, cx: &mut Context<Self>) {
        let Some(ch) = self.channel_list.read(cx).active_channel().cloned() else {
            return;
        };
        let Some(oldest_id) = self
            .chat_area
            .messages
            .first()
            .map(|m| m.id.clone())
            .filter(|id| !id.starts_with("temp-"))
        else {
            return;
        };
        self.loading_more = true;
        let api = self.api.clone();
        let cl_id = ch.clan_id.clone();
        let ch_id = ch.id.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .list_channel_messages(
                    &cl_id,
                    &ch_id,
                    &oldest_id,
                    DIRECTION_BEFORE,
                    MESSAGE_PAGE_LIMIT,
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading_more = false;
                let msgs = match result {
                    Ok(msgs) => msgs,
                    Err(e) => {
                        tracing::error!("Failed to load more messages for {ch_id}: {e}");
                        return;
                    }
                };
                if this.last_fetched_channel_id.as_deref() != Some(&ch_id) {
                    return;
                }
                let reached_start = (msgs.len() as u32) < MESSAGE_PAGE_LIMIT;
                let existing: std::collections::HashSet<String> = this
                    .chat_area
                    .messages
                    .iter()
                    .map(|m| m.id.clone())
                    .collect();
                let mut older: Vec<Message> = msgs
                    .into_iter()
                    .filter(|m| !existing.contains(&m.message_id))
                    .map(|m| {
                        Message::new(
                            m.message_id,
                            m.content,
                            m.sender_id,
                            m.sender_name,
                            m.create_time,
                        )
                        .with_avatar(m.avatar)
                        .with_attachments(to_store_attachments(m.attachments))
                    })
                    .collect();
                if older.is_empty() {
                    this.has_more = false;
                    return;
                }
                this.has_more = !reached_start;
                let prepended = older.len();
                let mut existing =
                    match Rc::try_unwrap(std::mem::take(&mut this.chat_area.messages)) {
                        Ok(v) => v,
                        Err(rc) => (*rc).clone(),
                    };
                older.append(&mut existing);
                older.sort_by_key(|m| m.create_time);
                this.chat_area.messages = Rc::new(older);
                this.chat_area.list_state.splice(0..0, prepended);
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for ChatLayout {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = resolve_theme(&self.settings.read(cx).theme);

        if !self.loaded {
            self.loaded = true;
            self.clan_list.update(cx, |clans, cx| clans.reload(cx));
        }

        if !self.list_scroll_installed {
            self.list_scroll_installed = true;
            let weak = cx.entity().downgrade();
            self.chat_area
                .list_state
                .set_scroll_handler(move |event, _window, cx| {
                    if event.visible_range.start < LOAD_MORE_ITEM_THRESHOLD
                        && let Some(this) = weak.upgrade()
                    {
                        this.update(cx, |this, cx| this.maybe_load_more(cx));
                    }
                });
        }

        let active_ch = self.channel_list.read(cx).active_channel().cloned();
        if let Some(ref ch) = active_ch {
            let prev_id = self.last_fetched_channel_id.clone();
            if Some(&ch.id) != prev_id.as_ref() {
                self.last_fetched_channel_id = Some(ch.id.clone());
                self.loading_more = false;
                self.has_more = true;
                let api = self.api.clone();
                let ch_id = ch.id.clone();
                let cl_id = ch.clan_id.clone();
                let is_public = !ch.private;
                {
                    let api = api.clone();
                    let cl_id = cl_id.clone();
                    let ch_id = ch_id.clone();
                    cx.spawn(
                        async move |_t: gpui::WeakEntity<Self>, _c: &mut gpui::AsyncApp| {
                            const CHANNEL_TYPE_CHANNEL: i32 = 1;
                            if let Err(e) = api
                                .join_chat(&cl_id, &ch_id, CHANNEL_TYPE_CHANNEL, is_public)
                                .await
                            {
                                tracing::warn!("join_chat failed: {e}");
                            }
                        },
                    )
                    .detach();
                }
                cx.spawn(
                    async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| match api
                        .list_channel_messages(&cl_id, &ch_id, "", 0, MESSAGE_PAGE_LIMIT)
                        .await
                    {
                        Ok(msgs) => {
                            tracing::info!("Fetched {} messages for channel {}", msgs.len(), ch_id);
                            let reached_start = (msgs.len() as u32) < MESSAGE_PAGE_LIMIT;
                            let mut store_msgs: Vec<Message> = msgs
                                .into_iter()
                                .map(|m| {
                                    Message::new(
                                        m.message_id,
                                        m.content,
                                        m.sender_id,
                                        m.sender_name,
                                        m.create_time,
                                    )
                                    .with_avatar(m.avatar)
                                    .with_attachments(to_store_attachments(m.attachments))
                                })
                                .collect();
                            store_msgs.sort_by_key(|m| m.create_time);
                            let fetched_ch_id = ch_id.clone();
                            let _ = this.update(cx, |this, cx| {
                                if this.last_fetched_channel_id.as_deref() != Some(&fetched_ch_id) {
                                    return;
                                }
                                this.has_more = !reached_start;
                                this.chat_area.messages = Rc::new(store_msgs);
                                let count = this.chat_area.messages.len();
                                this.chat_area.list_state.reset(count);
                                cx.notify();
                            });
                        }
                        Err(e) => tracing::error!("Failed to fetch messages for {ch_id}: {e}"),
                    },
                )
                .detach();
            }
        }

        self.chat_area.ensure_input(window, cx);
        let content = self.render_content(cx);

        div()
            .flex()
            .flex_row()
            .flex_1()
            .w_full()
            .min_h_0()
            .bg(theme.bg_primary)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(312.0))
                    .h_full()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_1()
                            .min_h_0()
                            .child(div().w(px(72.0)).h_full().child(self.clan_sidebar.clone()))
                            .child(
                                div()
                                    .w(px(240.0))
                                    .h_full()
                                    .child(self.channel_sidebar.clone()),
                            ),
                    )
                    .child(self.user_info_bar.render(&theme, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .bg(theme.bg_primary)
                    .child(content),
            )
    }
}

impl ChatLayout {
    pub(crate) fn send_current_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.chat_area.input_state.clone() else {
            return;
        };
        let content = input.read(cx).value().trim().to_string();
        if content.is_empty() {
            return;
        }
        input.update(cx, |state, cx| state.set_value("", window, cx));

        let (uid, uname) = match self.auth_state.read(cx) {
            AuthState::Authenticated(session) => {
                (session.user_id.clone(), session.username.clone())
            }
            _ => (String::new(), String::new()),
        };

        let Some(channel) = self.channel_list.read(cx).active_channel().cloned() else {
            tracing::warn!("send: no active channel");
            return;
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default();
        let temp_id = format!("temp-{now}");

        Rc::make_mut(&mut self.chat_area.messages).push(Message::new(
            temp_id.clone(),
            content.clone(),
            uid,
            uname,
            now,
        ));
        let count = self.chat_area.messages.len();
        self.chat_area.list_state.reset(count);
        cx.notify();

        let api = self.api.clone();
        let clan_id = channel.clan_id.clone();
        let channel_id = channel.id.clone();
        let is_public = !channel.private;
        cx.spawn(async move |this, cx| {
            match api
                .send_channel_message(&clan_id, &channel_id, &content, is_public)
                .await
            {
                Ok(sent) => {
                    let _ = this.update(cx, |this, cx| {
                        if let Some(slot) = Rc::make_mut(&mut this.chat_area.messages)
                            .iter_mut()
                            .find(|m| m.id == temp_id)
                        {
                            slot.id = sent.message_id;
                            cx.notify();
                        }
                    });
                }
                Err(e) => tracing::error!("send_channel_message failed: {e}"),
            }
        })
        .detach();
    }

    fn render_content(&self, cx: &Context<Self>) -> gpui::AnyElement {
        let theme = resolve_theme(&self.settings.read(cx).theme);

        let (session_user_id, session_user_name) = match self.auth_state.read(cx) {
            AuthState::Authenticated(session) => {
                (session.user_id.clone(), session.username.clone())
            }
            _ => (String::new(), String::new()),
        };

        let channels = self.channel_list.read(cx);
        if let Some(ch) = channels.active_channel() {
            return self
                .chat_area
                .render(
                    &theme,
                    cx.entity(),
                    &ch.name,
                    &session_user_id,
                    &session_user_name,
                )
                .into_any_element();
        }

        let router = Router::global(cx);
        let route = router.read(cx).route();
        let current_path = router.read(cx).current_path();

        let placeholder = match route {
            Route::Chat => self.render_placeholder(
                theme,
                crate::components::primitives::IconName::Inbox,
                "Chat",
                &current_path,
            ),
            Route::Direct => self.render_placeholder(
                theme,
                crate::components::primitives::IconName::CircleUser,
                "Direct Messages",
                &current_path,
            ),
            Route::DirectMessage {
                direct_id,
                message_type: _,
            } => self.render_placeholder(
                theme,
                crate::components::primitives::IconName::CircleUser,
                &format!("Direct {direct_id}"),
                &current_path,
            ),
            Route::Channel {
                clan_id: _,
                channel_id,
            } => self.render_placeholder(
                theme,
                crate::components::primitives::IconName::FolderOpen,
                &format!("#{channel_id}"),
                &current_path,
            ),
            Route::SettingsAccount
            | Route::SettingsProfile
            | Route::SettingsDevices
            | Route::SettingsAppearance
            | Route::SettingsActivity
            | Route::SettingsNotifications
            | Route::SettingsLanguage
            | Route::SettingsVoice
            | Route::SettingsAdvanced
            | Route::NotFound { .. } => {
                // Handled by RootView, not rendered here
                div().into_any_element()
            }
        };

        div()
            .flex_1()
            .min_h_0()
            .p_6()
            .child(placeholder)
            .into_any_element()
    }

    fn render_placeholder(
        &self,
        theme: Theme,
        icon: crate::components::primitives::IconName,
        title: &str,
        _path: &str,
    ) -> gpui::AnyElement {
        use crate::components::primitives::Icon;

        div()
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .flex_col()
            .gap_4()
            .child(Icon::new(icon).size_8().text_color(theme.text_muted))
            .child(
                div()
                    .text_base()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_primary)
                    .child(title.to_string()),
            )
            .into_any_element()
    }
}
