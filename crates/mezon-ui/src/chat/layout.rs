use gpui::{AnyView, Context, Entity, StyleRefinement, Window, div, prelude::*, px};
use mezon_store::{
    AuthState, ChannelList, ClanList, DirectChannel, DirectMessageStore, MessagesStore,
    PinnedMessagesStore, PresenceEvent, PresenceStore, Settings,
};
use ui::PopoverMenuHandle;

use crate::chat::area::ChatArea;
use crate::chat::pinned_popover::PinnedPopoverPanel;
use crate::components::compositions::user_info_bar::UserInfoBar;
use crate::router::{Route, Router};
use crate::theme::{ActiveTheme, Theme};
use crate::{ChannelSidebar, ClanSidebar, DirectSidebar};

pub struct ChatLayout {
    pub(crate) channel_list: Entity<ChannelList>,
    pub chat_area: ChatArea,
    clan_sidebar: Entity<ClanSidebar>,
    channel_sidebar: Entity<ChannelSidebar>,
    direct_sidebar: Entity<DirectSidebar>,
    direct_store: Entity<DirectMessageStore>,
    user_info_bar: UserInfoBar,
    clan_list: Entity<ClanList>,
    auth_state: Entity<AuthState>,
    settings: Entity<Settings>,
    pending_channel_id: Option<String>,
    pin_popover_handle: PopoverMenuHandle<PinnedPopoverPanel>,
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

        let settings_for_direct = settings.clone();
        let direct_sidebar = cx.new(move |cx| DirectSidebar::new(settings_for_direct, cx));

        let user_info_bar = UserInfoBar::new(auth_state.clone());

        // The user bar only shows the current user's online status (`user_online`). Typing churns
        // constantly and never touches `user_online`, so subscribe (not observe) and skip
        // `TypingChanged` — keeping typing events from dirtying the shell.
        cx.subscribe(&PresenceStore::global(cx), |this, _, event, cx| {
            if matches!(event, PresenceEvent::TypingChanged { .. }) {
                return;
            }
            this.user_info_bar.sync_presence(cx);
            cx.notify();
        })
        .detach();
        cx.observe(&auth_state, |this, _, cx| {
            this.user_info_bar.sync_presence(cx);
            cx.notify();
        })
        .detach();

        let direct_store = DirectMessageStore::global(cx);
        cx.observe(&direct_store, |_, _, cx| cx.notify()).detach();

        let messages_store = MessagesStore::global(cx);
        cx.observe(&messages_store, |_, _, cx| cx.notify()).detach();

        let pinned_store = PinnedMessagesStore::global(cx);
        cx.observe(&pinned_store, |_, _, cx| cx.notify()).detach();

        let chat_area = ChatArea::new(settings.clone(), cx);
        cx.observe(&channel_list, |this, _, cx| {
            this.apply_pending_channel(cx);
            this.ensure_active_channel_for_clan(cx);
            this.pin_popover_handle.hide(cx);
            cx.notify();
        })
        .detach();
        cx.observe(&Router::global(cx), |this, _, cx| {
            if matches!(
                Router::global(cx).read(cx).route(),
                Route::Direct | Route::DirectMessage { .. }
            ) {
                this.pin_popover_handle.hide(cx);
            }
            this.sync_active_from_route(cx);
            cx.notify();
        })
        .detach();
        let mut this = Self {
            channel_list,
            clan_sidebar,
            channel_sidebar,
            direct_sidebar,
            direct_store,
            user_info_bar,
            clan_list,
            auth_state,
            chat_area,
            settings,
            pending_channel_id: None,
            pin_popover_handle: PopoverMenuHandle::default(),
        };
        this.user_info_bar.sync_presence(cx);
        this.sync_active_from_route(cx);
        this
    }

    fn sync_active_from_route(&mut self, cx: &mut Context<Self>) {
        match Router::global(cx).read(cx).route() {
            Route::Channel {
                clan_id,
                channel_id,
            } => self.sync_channel_route(clan_id, channel_id, cx),
            Route::DirectMessage {
                direct_id,
                message_type,
            } => {
                self.pending_channel_id = None;
                self.direct_store
                    .update(cx, |store, cx| store.ensure_loaded(cx));
                let channel_type = message_type.parse::<i32>().unwrap_or(3);
                MessagesStore::global(cx).update(cx, |store, cx| {
                    store.open_direct(direct_id, channel_type, cx)
                });
            }
            Route::Direct => {
                self.pending_channel_id = None;
                self.direct_store
                    .update(cx, |store, cx| store.ensure_loaded(cx));
            }
            _ => {
                self.pending_channel_id = None;
            }
        }
    }

    fn sync_channel_route(&mut self, clan_id: String, channel_id: String, cx: &mut Context<Self>) {
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
            // Force the messages store onto this channel even when it's already the active
            // `ChannelList` selection — covers returning from a DM (where `ChannelList` never
            // re-emits) so the timeline switches back instead of showing the DM.
            MessagesStore::global(cx).update(cx, |store, cx| store.open_channel(channel_id, cx));
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

    fn ensure_active_channel_for_clan(&mut self, cx: &mut Context<Self>) {
        if matches!(
            Router::global(cx).read(cx).route(),
            Route::Direct | Route::DirectMessage { .. }
        ) {
            return;
        }
        let Some(clan_id) = self.clan_list.read(cx).active_clan_id.clone() else {
            return;
        };

        if let Route::Channel {
            clan_id: route_clan,
            channel_id,
        } = Router::global(cx).read(cx).route()
            && route_clan == clan_id
            && self
                .channel_list
                .read(cx)
                .channel_in_clan(&clan_id, &channel_id)
        {
            return;
        }

        let welcome = self.clan_list.read(cx).welcome_channel_id(&clan_id);
        let target = {
            let channels = self.channel_list.read(cx);
            welcome
                .filter(|w| channels.channel_in_clan(&clan_id, w))
                .or_else(|| channels.default_channel_id(&clan_id))
        };
        let Some(channel_id) = target else {
            return;
        };

        crate::router::navigate(
            cx,
            Route::Channel {
                clan_id,
                channel_id,
            },
        );
    }
}

impl Render for ChatLayout {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::trace_render!("ChatLayout");
        self.chat_area.ensure_input(window, cx);
        let nav_body = self.render_nav_body(cx);
        let content = self.render_content(window, cx);
        let theme = cx.theme();

        div()
            .flex()
            .flex_row()
            .flex_1()
            .w_full()
            .h_full()
            .min_h_0()
            .bg(theme.bg_primary)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(344.0))
                    .h_full()
                    .relative()
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
                            .child(div().w(px(272.0)).h_full().child(nav_body)),
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

        MessagesStore::global(cx).update(cx, |store, cx| {
            store.send_message(content, uid, uname, cx);
        });
    }

    fn current_dm(&self, cx: &Context<Self>) -> Option<DirectChannel> {
        let Route::DirectMessage { direct_id, .. } = Router::global(cx).read(cx).route() else {
            return None;
        };
        self.direct_store.read(cx).find(&direct_id).cloned()
    }

    fn is_dm_route(&self, cx: &Context<Self>) -> bool {
        matches!(
            Router::global(cx).read(cx).route(),
            Route::Direct | Route::DirectMessage { .. }
        )
    }

    fn render_nav_body(&self, cx: &Context<Self>) -> gpui::AnyElement {
        let view: AnyView = if self.is_dm_route(cx) {
            self.direct_sidebar.clone().into()
        } else {
            self.channel_sidebar.clone().into()
        };
        view.cached(StyleRefinement::default().size_full())
            .into_any_element()
    }

    fn render_content(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let locale = self.settings.read(cx).language.clone();
        let theme = cx.theme().clone();

        if self.is_dm_route(cx) {
            if let Some(dm) = self.current_dm(cx) {
                return self
                    .chat_area
                    .render(
                        &theme,
                        &locale,
                        cx.entity(),
                        &dm.label,
                        true,
                        None,
                        window,
                        cx,
                    )
                    .into_any_element();
            }
            return div()
                .flex()
                .size_full()
                .items_center()
                .justify_center()
                .flex_col()
                .gap_4()
                .child(
                    crate::components::primitives::Icon::new(
                        crate::components::primitives::IconName::People,
                    )
                    .size_8()
                    .text_color(theme.text_muted),
                )
                .child(
                    div()
                        .text_base()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text_primary)
                        .child(mezon_i18n::t(&locale, "dm.emptyState")),
                )
                .into_any_element();
        }

        let channels = self.channel_list.read(cx);
        if let Some(ch) = channels.active_channel() {
            let name = ch.name.clone();
            return self
                .chat_area
                .render(
                    &theme,
                    &locale,
                    cx.entity(),
                    &name,
                    false,
                    Some(self.pin_popover_handle.clone()),
                    window,
                    cx,
                )
                .into_any_element();
        }

        let router = Router::global(cx);
        let route = router.read(cx).route();
        let current_path = router.read(cx).current_path();

        let placeholder = match route {
            Route::Chat => self.render_placeholder(
                &theme,
                crate::components::primitives::IconName::Inbox,
                mezon_i18n::t(&locale, "nav.chat"),
                &current_path,
            ),
            Route::Direct => self.render_placeholder(
                &theme,
                crate::components::primitives::IconName::People,
                mezon_i18n::t(&locale, "dm.title"),
                &current_path,
            ),
            Route::DirectMessage {
                direct_id,
                message_type: _,
            } => self.render_placeholder(
                &theme,
                crate::components::primitives::IconName::People,
                &format!("Direct {direct_id}"),
                &current_path,
            ),
            Route::Channel {
                clan_id: _,
                channel_id,
            } => self.render_placeholder(
                &theme,
                crate::components::primitives::IconName::Hashtag,
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
