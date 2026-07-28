use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, MouseButton,
    MouseDownEvent, Render, Rgba, Subscription, Task, Window, deferred, div, prelude::*, px, svg,
};
use mezon_store::{
    AccountStore, BadgeService, ClanId, ClanMembersStore, FriendState, FriendStore, PresenceStore,
    Settings, UserId,
};
use std::time::{Duration, Instant};

use crate::app::shell::{FriendRemovalKind, Shell};
use crate::chat::message::SendTokenModal;
use crate::components::primitives::{Avatar, Icon, IconName};
use crate::image_cache::{LruImageCache, read_body_limited};
use crate::router::{Route, navigate};
use crate::theme::ActiveTheme;
use ui::Tooltip;

const AVATAR_FETCH_LIMIT: usize = 8 * 1024 * 1024;

pub struct UserProfileModal {
    focus_handle: FocusHandle,
    user_id: UserId,
    clan_id: ClanId,
    settings: Entity<Settings>,
    avatar_image_cache: Entity<LruImageCache>,
    banner_color: Option<Rgba>,
    is_self: bool,
    edit_options_open: bool,
    live_status: String,
    live_custom_status: String,
    account_status_snapshot: String,
    account_custom_snapshot: String,
    account_override_until: Option<Instant>,
    presence_status_snapshot: String,
    presence_custom_snapshot: String,
    _banner_task: Option<Task<()>>,
    _members_sub: Subscription,
    _presence_sub: Subscription,
    _friend_sub: Subscription,
    _account_sub: Subscription,
}

impl UserProfileModal {
    pub fn new(
        user_id: UserId,
        clan_id: ClanId,
        settings: Entity<Settings>,
        avatar_image_cache: Entity<LruImageCache>,
        cx: &mut Context<Self>,
    ) -> Self {
        let account = AccountStore::global(cx).read(cx).account.clone();
        let member_username = ClanMembersStore::global(cx)
            .read(cx)
            .member(clan_id, user_id)
            .map(|member| member.user.username.clone())
            .unwrap_or_default();
        let is_self = BadgeService::global(cx)
            .read(cx)
            .current_user_id(cx)
            .is_some_and(|id| id == user_id)
            || account
                .as_ref()
                .is_some_and(|account| account.username == member_username);
        let presence = PresenceStore::global(cx);
        let presence = presence.read(cx);
        let presence_status_snapshot = presence
            .presence_status(user_id)
            .map(str::to_string)
            .unwrap_or_else(|| {
                if presence.is_online(user_id) {
                    "Online".to_string()
                } else {
                    "Invisible".to_string()
                }
            });
        let presence_custom_snapshot = presence.user_status(user_id).unwrap_or("").to_string();
        let (live_status, live_custom_status) = if is_self {
            account
                .as_ref()
                .map(|account| (account.status.clone(), account.user_status.clone()))
                .unwrap_or_default()
        } else {
            (
                presence_status_snapshot.clone(),
                presence.user_status(user_id).unwrap_or("").to_string(),
            )
        };
        let account_status_snapshot = account
            .as_ref()
            .map(|account| account.status.clone())
            .unwrap_or_default();
        let account_custom_snapshot = account
            .as_ref()
            .map(|account| account.user_status.clone())
            .unwrap_or_default();
        let members_sub = cx.observe(&ClanMembersStore::global(cx), |_, _, cx| cx.notify());
        let presence_sub = cx.observe(&PresenceStore::global(cx), |this, _, cx| {
            this.sync_presence_status(cx);
            cx.notify();
        });
        let friend_sub = cx.observe(&FriendStore::global(cx), |_, _, cx| cx.notify());
        let account_sub = cx.observe(&AccountStore::global(cx), |this, _, cx| {
            this.sync_account_status(cx);
            cx.notify();
        });
        let source_avatar = ClanMembersStore::global(cx)
            .read(cx)
            .member(clan_id, user_id)
            .map(|member| member.user.avatar_url.clone())
            .unwrap_or_default();

        let mut modal = Self {
            focus_handle: cx.focus_handle(),
            user_id,
            clan_id,
            settings,
            avatar_image_cache,
            banner_color: None,
            is_self,
            edit_options_open: false,
            live_status,
            live_custom_status,
            account_status_snapshot,
            account_custom_snapshot,
            account_override_until: None,
            presence_status_snapshot,
            presence_custom_snapshot,
            _banner_task: None,
            _members_sub: members_sub,
            _presence_sub: presence_sub,
            _friend_sub: friend_sub,
            _account_sub: account_sub,
        };
        modal.load_banner_color(source_avatar, cx);
        modal
    }

    fn sync_account_status(&mut self, cx: &App) -> bool {
        if !self.is_self {
            return false;
        }
        let Some(account) = AccountStore::global(cx).read(cx).account.as_ref() else {
            return false;
        };
        let account_changed = self.account_status_snapshot != account.status
            || self.account_custom_snapshot != account.user_status;
        if !account_changed {
            return false;
        }
        let changed =
            self.live_status != account.status || self.live_custom_status != account.user_status;
        self.account_status_snapshot = account.status.clone();
        self.account_custom_snapshot = account.user_status.clone();
        self.live_status = account.status.clone();
        self.live_custom_status = account.user_status.clone();
        self.account_override_until = Some(Instant::now() + Duration::from_secs(2));
        changed
    }

    fn sync_presence_status(&mut self, cx: &App) -> bool {
        if self
            .account_override_until
            .is_some_and(|until| Instant::now() < until)
        {
            return false;
        }
        self.account_override_until = None;
        let presence = PresenceStore::global(cx);
        let presence = presence.read(cx);
        let status = presence.presence_status(self.user_id).unwrap_or_else(|| {
            if presence.is_online(self.user_id) {
                "Online"
            } else {
                "Invisible"
            }
        });
        let custom_status = presence.user_status(self.user_id).unwrap_or("");
        let presence_changed = self.presence_status_snapshot.as_str() != status
            || self.presence_custom_snapshot.as_str() != custom_status;
        if !presence_changed {
            return false;
        }
        let changed =
            self.live_status != status || self.live_custom_status.as_str() != custom_status;
        self.presence_status_snapshot = status.to_string();
        self.presence_custom_snapshot = custom_status.to_string();
        self.live_status = status.to_string();
        self.live_custom_status = custom_status.to_string();
        changed
    }

    fn load_banner_color(&mut self, avatar_url: String, cx: &mut Context<Self>) {
        if avatar_url.is_empty() {
            return;
        }
        let client = cx.http_client();
        self._banner_task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let mut response = client.get(&avatar_url, ().into(), true).await?;
                if !response.status().is_success() {
                    anyhow::bail!("avatar fetch returned {}", response.status());
                }
                let bytes = read_body_limited(&mut response, AVATAR_FETCH_LIMIT).await?;
                let color = cx
                    .background_executor()
                    .spawn(async move {
                        let image = image::load_from_memory(&bytes)?.to_rgba8();
                        anyhow::Ok(average_color(&image))
                    })
                    .await?;
                anyhow::Ok(color)
            }
            .await;

            if let Ok(color) = result {
                let _ = this.update(cx, |this, cx| {
                    this.banner_color = color;
                    cx.notify();
                });
            }
        }));
    }

    fn close(cx: &mut App) {
        Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
    }
}

impl Focusable for UserProfileModal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for UserProfileModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let locale = self.settings.read(cx).language.clone();
        let member = ClanMembersStore::global(cx)
            .read(cx)
            .member(self.clan_id, self.user_id)
            .cloned();
        let (display_name, username, avatar, about_me, created_at) = member
            .as_ref()
            .map(|member| {
                (
                    member.name().to_string(),
                    member.user.username.clone(),
                    member.avatar().to_string(),
                    member.user.about_me.clone(),
                    member.user.create_time_seconds,
                )
            })
            .unwrap_or_default();
        let is_self = self.is_self;
        let custom_status = self.live_custom_status.clone();
        let (status_icon, status_color) = profile_status(&self.live_status, theme);
        let friend_state = FriendStore::global(cx)
            .read(cx)
            .friend(self.user_id)
            .map(|friend| friend.state);

        let member_since = format_member_since(created_at);
        let mut avatar_view = Avatar::new()
            .name(display_name.clone())
            .size_px(px(96.))
            .image_cache(self.avatar_image_cache.clone());
        if !avatar.is_empty() {
            avatar_view = avatar_view.src(avatar.clone());
        }

        let banner_color = self.banner_color.unwrap_or(gpui::rgb(0xF7E4F0));

        div()
            .id("full-user-profile-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::hsla(0., 0., 0., 0.8))
            .track_focus(&self.focus_handle)
            .key_context("modal_backdrop")
            .on_action(|_: &::menu::Cancel, _window, cx| Self::close(cx))
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _window, cx| {
                Self::close(cx);
            })
            .child(
                div()
                    .id("full-user-profile-card")
                    .occlude()
                    .relative()
                    .w(px(600.))
                    .h(px(640.))
                    .max_h_full()
                    .rounded_lg()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.bg_floating)
                    .shadow_lg()
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(180.))
                            .rounded_tl_lg()
                            .rounded_tr_lg()
                            .bg(banner_color)
                            .child(render_profile_actions(
                                is_self,
                                friend_state,
                                self.user_id,
                                &username,
                                &display_name,
                                &avatar,
                                &locale,
                                theme,
                            )),
                    )
                    .child(
                        div()
                            .h(px(460.))
                            .pt(px(72.))
                            .px(px(20.))
                            .pb(px(16.))
                            .rounded_bl_lg()
                            .rounded_br_lg()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .bg(theme.tokens.bg_primary)
                            .child(
                                div()
                                    .child(
                                        div()
                                            .text_size(px(24.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.text_secondary)
                                            .child(display_name),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(theme.text_secondary)
                                            .child(username),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .rounded_lg()
                                    .bg(theme.tokens.bg_secondary)
                                    .shadow_sm()
                                    .p_4()
                                    .child(
                                        div()
                                            .pb_2()
                                            .border_b_1()
                                            .border_color(theme.border)
                                            .text_sm()
                                            .text_color(theme.text_secondary)
                                            .child(mezon_i18n::t(
                                                &locale,
                                                "userProfile.labels.aboutMe",
                                            )),
                                    )
                                    .when(!about_me.is_empty(), |content| {
                                        content.child(
                                            div()
                                                .mt_4()
                                                .text_sm()
                                                .text_color(theme.text_secondary)
                                                .child(about_me),
                                        )
                                    })
                                    .child(
                                        div()
                                            .mt_4()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.text_secondary)
                                            .child(mezon_i18n::t(
                                                &locale,
                                                "userProfile.labels.memberSince",
                                            )),
                                    )
                                    .child(
                                        div()
                                            .mt_2()
                                            .text_sm()
                                            .text_color(theme.text_secondary)
                                            .child(member_since),
                                    ),
                            ),
                    )
                    .child(deferred(
                        div()
                            .absolute()
                            .left(px(20.))
                            .top(px(132.))
                            .size(px(108.))
                            .rounded_full()
                            .bg(theme.bg_floating)
                            .p(px(6.))
                            .child(avatar_view)
                            .child(
                                div()
                                    .absolute()
                                    .right(px(5.))
                                    .bottom(px(5.))
                                    .p(px(2.))
                                    .rounded_full()
                                    .bg(theme.bg_floating)
                                    .child(
                                        Icon::new(status_icon)
                                            .size(px(19.))
                                            .text_color(status_color),
                                    ),
                            )
                            .when(!custom_status.is_empty(), |avatar| {
                                avatar.child(
                                    div()
                                        .absolute()
                                        .right(px(-20.))
                                        .top(px(25.))
                                        .size(px(14.))
                                        .rounded_full()
                                        .bg(theme.tokens.bg_secondary)
                                        .border_1()
                                        .border_color(theme.bg_floating),
                                )
                            }),
                    ))
                    .when(!custom_status.is_empty(), |card| {
                        card.child(deferred(
                            div()
                                .absolute()
                                .left(px(134.))
                                .top(px(194.))
                                .max_w(px(250.))
                                .max_h(px(64.))
                                .child(
                                    div()
                                        .max_w(px(250.))
                                        .max_h(px(64.))
                                        .px_4()
                                        .py_3()
                                        .rounded_xl()
                                        .bg(theme.tokens.bg_secondary)
                                        .border_1()
                                        .border_color(theme.border)
                                        .shadow_md()
                                        .text_sm()
                                        .text_color(theme.text_secondary)
                                        .overflow_hidden()
                                        .child(custom_status),
                                ),
                        ))
                    })
                    .when(is_self, |card| {
                        card.child(deferred(
                            div()
                                .id("full-profile-edit")
                                .absolute()
                                .right(px(18.))
                                .top(px(200.))
                                .h(px(34.))
                                .px_3()
                                .flex()
                                .items_center()
                                .gap_1()
                                .rounded(px(4.))
                                .cursor_pointer()
                                .hover(|style| style.bg(theme.bg_hover))
                                .child(
                                    Icon::new(IconName::PenEdit)
                                        .size(px(16.))
                                        .text_color(theme.text_secondary),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(theme.text_primary)
                                        .child(mezon_i18n::t(
                                            &locale,
                                            "userProfile.labels.editProfile",
                                        )),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.edit_options_open = !this.edit_options_open;
                                    cx.notify();
                                })),
                        ))
                        .when(self.edit_options_open, |card| {
                            let clan_id = self.clan_id;
                            card.child(deferred(
                                div()
                                    .id("full-profile-edit-options")
                                    .occlude()
                                    .absolute()
                                    .right(px(-192.))
                                    .top(px(180.))
                                    .w(px(180.))
                                    .p_2()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(theme.border)
                                    .bg(theme.tokens.bg_secondary)
                                    .shadow_lg()
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation();
                                    })
                                    .child(
                                        div()
                                            .id("edit-clan-profile-option")
                                            .cursor_pointer()
                                            .px_3()
                                            .py_2()
                                            .rounded_sm()
                                            .text_sm()
                                            .text_color(theme.text_secondary)
                                            .hover(|style| style.bg(theme.bg_hover))
                                            .child(mezon_i18n::t(
                                                &locale,
                                                "common.userProfile.editClanProfile",
                                            ))
                                            .on_click(move |_, _, cx| {
                                                crate::settings::request_clan_profile(clan_id);
                                                Self::close(cx);
                                                navigate(cx, Route::SettingsProfile);
                                            }),
                                    )
                                    .child(
                                        div()
                                            .id("edit-main-profile-option")
                                            .cursor_pointer()
                                            .px_3()
                                            .py_2()
                                            .rounded_sm()
                                            .text_sm()
                                            .text_color(theme.text_secondary)
                                            .hover(|style| style.bg(theme.bg_hover))
                                            .child(mezon_i18n::t(
                                                &locale,
                                                "common.userProfile.editMainProfile",
                                            ))
                                            .on_click(|_, _, cx| {
                                                crate::settings::request_user_profile();
                                                Self::close(cx);
                                                navigate(cx, Route::SettingsProfile);
                                            }),
                                    ),
                            ))
                        })
                    }),
            )
    }
}

fn render_profile_actions(
    is_self: bool,
    friend_state: Option<FriendState>,
    user_id: UserId,
    username: &str,
    display_name: &str,
    avatar: &str,
    locale: &str,
    theme: &crate::theme::Theme,
) -> AnyElement {
    if is_self {
        return div().into_any_element();
    }

    let transfer_username = username.to_string();
    let transfer_locale = locale.to_string();
    let mut actions = div()
        .absolute()
        .top(px(10.))
        .right(px(10.))
        .flex()
        .items_center()
        .gap_2()
        .child(profile_action_button(
            "full-profile-transfer",
            IconName::Transaction,
            mezon_i18n::t(locale, "common.transfer"),
            theme,
            move |_, window, cx| {
                UserProfileModal::close(cx);
                SendTokenModal::open(
                    transfer_locale.clone().into(),
                    Some((user_id.0.to_string(), transfer_username.clone())),
                    window,
                    cx,
                );
            },
        ));

    if friend_state == Some(FriendState::Friend) {
        actions = actions.child(profile_share_contact_button(
            "full-profile-share-contact",
            mezon_i18n::t(locale, "common.shareContact"),
            theme,
        ));
    }

    if friend_state == Some(FriendState::InviteReceived) {
        let ignore_username = username.to_string();
        let ignore_locale = locale.to_string();
        return actions
            .child(profile_action_button(
                "full-profile-accept-friend",
                IconName::IConAcceptFriend,
                mezon_i18n::t(locale, "common.accept"),
                theme,
                move |_, _, cx| {
                    FriendStore::global(cx)
                        .update(cx, |store, cx| store.accept_friend(user_id, cx));
                },
            ))
            .child(profile_action_button(
                "full-profile-ignore-friend",
                IconName::IConIgnoreFriend,
                mezon_i18n::t(locale, "common.ignore"),
                theme,
                move |_, window, cx| {
                    UserProfileModal::close(cx);
                    Shell::global(cx).update(cx, |shell, cx| {
                        shell.confirm_remove_friend(
                            user_id,
                            &ignore_username,
                            FriendRemovalKind::RejectRequest,
                            &ignore_locale,
                            window,
                            cx,
                        );
                    });
                },
            ))
            .into_any_element();
    }

    let friend_icon = match friend_state {
        Some(FriendState::Friend) => IconName::IconFriend,
        Some(FriendState::InviteSent) => IconName::PendingFriend,
        Some(FriendState::Blocked) => return actions.into_any_element(),
        None => IconName::AddPerson,
        Some(FriendState::InviteReceived) => unreachable!(),
    };
    let action_username = username.to_string();
    let action_display_name = display_name.to_string();
    let action_avatar = avatar.to_string();
    let action_locale = locale.to_string();
    actions
        .child(profile_action_button(
            "full-profile-friend-state",
            friend_icon,
            match friend_state {
                Some(FriendState::Friend) => mezon_i18n::t(locale, "common.friend"),
                Some(FriendState::InviteSent) => mezon_i18n::t(locale, "common.pending"),
                Some(FriendState::InviteReceived) => mezon_i18n::t(locale, "common.accept"),
                _ => mezon_i18n::t(locale, "common.addFriend"),
            },
            theme,
            move |_, window, cx| match friend_state {
                Some(FriendState::Friend) => {
                    UserProfileModal::close(cx);
                    Shell::global(cx).update(cx, |shell, cx| {
                        shell.confirm_remove_friend(
                            user_id,
                            &action_username,
                            FriendRemovalKind::RemoveFriend,
                            &action_locale,
                            window,
                            cx,
                        );
                    });
                }
                Some(FriendState::InviteSent) => {
                    UserProfileModal::close(cx);
                    Shell::global(cx).update(cx, |shell, cx| {
                        shell.confirm_remove_friend(
                            user_id,
                            &action_username,
                            FriendRemovalKind::CancelRequest,
                            &action_locale,
                            window,
                            cx,
                        );
                    });
                }
                None => {
                    FriendStore::global(cx).update(cx, |store, cx| {
                        store.add_friend(
                            user_id,
                            action_username.clone(),
                            action_display_name.clone(),
                            action_avatar.clone(),
                            cx,
                        );
                    });
                }
                _ => {}
            },
        ))
        .into_any_element()
}

fn profile_action_button(
    id: &'static str,
    icon: IconName,
    tooltip: impl Into<gpui::SharedString> + 'static,
    _theme: &crate::theme::Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .size(px(34.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(gpui::rgb(0x272120))
        .cursor_pointer()
        .tooltip(Tooltip::text(tooltip))
        .on_click(on_click)
        .child(Icon::new(icon).size(px(16.)).text_color(gpui::white()))
        .into_any_element()
}

fn profile_share_contact_button(
    id: &'static str,
    tooltip: impl Into<gpui::SharedString> + 'static,
    _theme: &crate::theme::Theme,
) -> AnyElement {
    div()
        .id(id)
        .size(px(34.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(gpui::rgb(0x272120))
        .cursor_pointer()
        .tooltip(Tooltip::text(tooltip))
        .on_click(|_, _, _| {})
        .child(
            div()
                .relative()
                .size(px(16.))
                .child(
                    svg()
                        .path("icons/icon-share-contact-base.svg")
                        .size(px(16.))
                        .text_color(gpui::rgb(0x656369)),
                )
                .child(
                    svg()
                        .path("icons/icon-share-contact-accent.svg")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size(px(16.))
                        .text_color(gpui::white()),
                ),
        )
        .into_any_element()
}

fn profile_status(status: &str, theme: &crate::theme::Theme) -> (IconName, Rgba) {
    match status.to_ascii_lowercase().as_str() {
        "idle" => (IconName::DarkModeIcon, theme.status_idle),
        "dnd" | "do not disturb" => (IconName::MinusCircleIcon, theme.status_dnd),
        "invisible" | "offline" => (IconName::OfflineStatus, theme.status_offline),
        _ => (IconName::OnlineStatus, theme.status_online),
    }
}

fn format_member_since(seconds: u32) -> String {
    if seconds == 0 {
        return String::new();
    }
    chrono::DateTime::from_timestamp(i64::from(seconds), 0)
        .map(|date| date.format("%B %-d, %Y").to_string())
        .unwrap_or_default()
}

fn average_color(image: &image::RgbaImage) -> Option<Rgba> {
    let mut red = 0f64;
    let mut green = 0f64;
    let mut blue = 0f64;
    let mut weight = 0f64;
    for pixel in image.pixels() {
        let alpha = f64::from(pixel[3]) / 255.;
        if alpha == 0. {
            continue;
        }
        red += f64::from(pixel[0]).powi(2) * alpha;
        green += f64::from(pixel[1]).powi(2) * alpha;
        blue += f64::from(pixel[2]).powi(2) * alpha;
        weight += alpha;
    }
    (weight > 0.).then(|| Rgba {
        r: (red / weight).sqrt() as f32 / 255.,
        g: (green / weight).sqrt() as f32 / 255.,
        b: (blue / weight).sqrt() as f32 / 255.,
        a: 1.,
    })
}

#[cfg(test)]
mod tests {
    use super::average_color;

    #[test]
    fn average_color_uses_avatar_pixels() {
        let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([120, 60, 30, 255]));
        let color = average_color(&image).expect("color image has an average");
        assert!((color.r - 120. / 255.).abs() < 0.001);
        assert!((color.g - 60. / 255.).abs() < 0.001);
        assert!((color.b - 30. / 255.).abs() < 0.001);
    }
}
