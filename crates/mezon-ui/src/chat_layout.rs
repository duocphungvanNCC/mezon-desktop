use std::sync::Arc;

use gpui::{Context, Entity, Window, div, prelude::*, px};
use mezon_client::AppApi;
use mezon_store::{AuthState, ChannelList, ClanList, Message, Settings};

use crate::chat_area::ChatArea;
use crate::components::compositions::user_info_bar::UserInfoBar;
use crate::router::{Route, Router};
use crate::theme::{Theme, resolve_theme};
use crate::{ChannelSidebar, ClanSidebar};

pub struct ChatLayout {
    router: Router,
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
}

impl ChatLayout {
    pub fn new(
        router: Router,
        clan_list: Entity<ClanList>,
        auth_state: Entity<AuthState>,
        api: Arc<AppApi>,
        navigate: crate::components::NavigateFn,
        settings: Entity<Settings>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();

        let channel_list = ChannelList::global(cx);

        let on_navigate: Option<crate::components::NavigateFn> = {
            let nav = navigate.clone();
            Some(Arc::new(move |op, cx| nav(op, cx)))
        };

        let on_settings: Option<crate::components::NavigateFn> = {
            let nav = navigate.clone();
            Some(Arc::new(move |op, cx| nav(op, cx)))
        };

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
                on_navigate,
                settings_for_channel,
                cx,
            )
        });

        let user_info_bar = UserInfoBar::new(auth_state.clone(), on_settings);

        // Channel loading is driven by ChannelList's own subscription to ClanList events
        // (store-to-store, Zed-style). These observes just keep the shell repainting.
        cx.observe(&auth_state, |_, _, cx| cx.notify()).detach();
        cx.observe(&channel_list, |_, _, cx| cx.notify()).detach();
        cx.observe(&clan_list, |_, _, cx| cx.notify()).detach();

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
                                );
                                let msgs = &mut this.chat_area.messages;
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
            router,
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
        }
    }
}

impl Render for ChatLayout {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = resolve_theme(&self.settings.read(cx).theme);

        if !self.loaded {
            self.loaded = true;
            self.clan_list.update(cx, |clans, cx| clans.reload(cx));
        }

        let active_ch = self.channel_list.read(cx).active_channel().cloned();
        if let Some(ref ch) = active_ch {
            let prev_id = self.last_fetched_channel_id.clone();
            if Some(&ch.id) != prev_id.as_ref() {
                self.last_fetched_channel_id = Some(ch.id.clone());
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
                        .list_channel_messages(&cl_id, &ch_id, 20)
                        .await
                    {
                        Ok(msgs) => {
                            tracing::info!("Fetched {} messages for channel {}", msgs.len(), ch_id);
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
                                })
                                .collect();
                            store_msgs.sort_by_key(|m| m.create_time);
                            let fetched_ch_id = ch_id.clone();
                            let _ = this.update(cx, |this, cx| {
                                if this.last_fetched_channel_id.as_deref() != Some(&fetched_ch_id) {
                                    return;
                                }
                                this.chat_area.messages = store_msgs;
                                cx.notify();
                            });
                        }
                        Err(e) => tracing::error!("Failed to fetch messages for {ch_id}: {e}"),
                    },
                )
                .detach();
            }
        }

        self.chat_area.ensure_input(_window, cx);
        let content = self.render_content(cx);

        div()
            .flex()
            .flex_row()
            .flex_1()
            .size_full()
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
                    .bg(theme.bg_secondary)
                    .child(content),
            )
    }
}

impl ChatLayout {
    fn render_content(&self, cx: &Context<Self>) -> gpui::AnyElement {
        let theme = resolve_theme(&self.settings.read(cx).theme);

        let (session_user_id, session_user_name) = match self.auth_state.read(cx) {
            AuthState::Authenticated(session) => {
                (session.user_id.clone(), session.username.clone())
            }
            _ => (String::new(), String::new()),
        };

        // Use channel_list.active_channel_id to detect channel selection instead
        // of self.router.route(), because the router clone in ChatLayout is stale
        // (only the RootView's router gets updated on navigation).
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

        let route = self.router.route();
        let current_path = self.router.current_path().to_string();

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
