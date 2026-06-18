use gpui::{AnyView, Context, Entity, StyleRefinement, Window, div, prelude::*, px};
use mezon_store::{AuthState, ChannelList, ClanList, MessagesStore, PresenceStore, Settings};

use crate::chat_area::ChatArea;
use crate::components::compositions::user_info_bar::UserInfoBar;
use crate::router::{Route, Router};
use crate::theme::{ActiveTheme, Theme};
use crate::{ChannelSidebar, ClanSidebar};

pub struct ChatLayout {
    pub(crate) channel_list: Entity<ChannelList>,
    pub chat_area: ChatArea,
    clan_sidebar: Entity<ClanSidebar>,
    channel_sidebar: Entity<ChannelSidebar>,
    user_info_bar: UserInfoBar,
    clan_list: Entity<ClanList>,
    auth_state: Entity<AuthState>,
    pending_channel_id: Option<String>,
}

impl ChatLayout {
    pub fn new(
        clan_list: Entity<ClanList>,
        auth_state: Entity<AuthState>,
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

        cx.observe(&PresenceStore::global(cx), |this, _, cx| {
            this.user_info_bar.sync_presence(cx);
            cx.notify();
        })
        .detach();
        cx.observe(&auth_state, |this, _, cx| {
            this.user_info_bar.sync_presence(cx);
            cx.notify();
        })
        .detach();

        let chat_area = ChatArea::new(settings.clone(), cx);
        cx.observe(&channel_list, |this, _, cx| {
            this.apply_pending_channel(cx);
            cx.notify();
        })
        .detach();
        cx.observe(&Router::global(cx), |this, _, cx| {
            this.sync_active_from_route(cx);
            cx.notify();
        })
        .detach();
        let mut this = Self {
            channel_list,
            clan_sidebar,
            channel_sidebar,
            user_info_bar,
            clan_list,
            auth_state,
            chat_area,
            pending_channel_id: None,
        };
        this.user_info_bar.sync_presence(cx);
        this
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
                    channel_list.select_channel(&channel_id, cx);
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
                channel_list.select_channel(&channel_id, cx);
            });
        }
    }
}

impl Render for ChatLayout {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::trace_render!("ChatLayout");
        self.chat_area.ensure_input(window, cx);
        let theme = cx.theme();
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
                            .child(
                                div().w(px(72.0)).h_full().child(
                                    AnyView::from(self.clan_sidebar.clone())
                                        .cached(StyleRefinement::default().size_full()),
                                ),
                            )
                            .child(
                                div().w(px(240.0)).h_full().child(
                                    AnyView::from(self.channel_sidebar.clone())
                                        .cached(StyleRefinement::default().size_full()),
                                ),
                            ),
                    )
                    .child(self.user_info_bar.render(theme, cx)),
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

        if self.channel_list.read(cx).active_channel().is_none() {
            tracing::warn!("send: no active channel");
            return;
        }

        MessagesStore::global(cx).update(cx, |store, cx| {
            store.send_message(content, uid, uname, cx);
        });
    }

    fn render_content(&self, cx: &Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();

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
                    theme,
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
            | Route::NotFound { .. } => div().into_any_element(),
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
        theme: &Theme,
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
