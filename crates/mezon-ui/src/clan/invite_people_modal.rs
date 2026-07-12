use std::collections::HashSet;
use std::io::Cursor;
use std::sync::Arc;
use gpui::{
    AnyElement, App, ClickEvent, ClipboardItem, Context, Entity, FocusHandle, Focusable,
    FontWeight, Image as ClipboardImage, ImageFormat, ObjectFit, RenderImage, SharedString,
    Subscription, UniformListScrollHandle, Window, div, img, prelude::*, px, uniform_list,
};
use mezon_store::{
    AppConfig, ChannelId, ClanId, ClanInviteLink, ClanList, DirectMessageStore, Friend,
    FriendEvent, FriendState, FriendStore, UserId,
};
use crate::app::shell::Shell;
use crate::components::primitives::{
    Avatar, Button, ButtonVariants, Icon, IconName, Input, InputEvent, InputState,
};
use crate::theme::{ActiveTheme, Theme};
use crate::util::imgproxy;

// const ROW_HEIGHT: f32 = 74.;
// const AVATAR_SIZE: f32 = 48.;

#[derive(Clone)]
struct InviteFriendRow {
    id: UserId,
    label: SharedString,
    avatar_src: SharedString,
    avatar_raw: SharedString,
}

#[derive(Clone)]
struct QrInviteImage {
    render: Arc<RenderImage>,
    clipboard: ClipboardImage,
}

pub struct InvitePeopleModal {
    focus_handle: FocusHandle,
    clan_name: String,
    clan_avatar_src: SharedString,
    clan_avatar_raw: SharedString,
    locale: String,
    invite_link: String,
    invite_link_loading: bool,
    invite_link_error: Option<String>,
    qr_image: Option<QrInviteImage>,
    show_qr: bool,
    search_input: Entity<InputState>,
    rows: Vec<InviteFriendRow>,
    sent: HashSet<UserId>,
    sending: HashSet<UserId>,
    copied: bool,
    scroll: UniformListScrollHandle,
    _input_sub: Subscription,
    _friend_sub: Subscription,
}

impl Focusable for InvitePeopleModal {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl InvitePeopleModal {
    pub fn new(
        clan_id: ClanId,
        channel_id: Option<ChannelId>,
        clan_name: String,
        clan_avatar_url: String,
        locale: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        FriendStore::global(cx).update(cx, |store, cx| store.ensure_loaded(cx));
        DirectMessageStore::global(cx).update(cx, |store, cx| store.ensure_loaded(cx));

        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search for friends")
                .height(px(40.))
                .radius(px(8.))
        });
        let input_sub = cx.subscribe(
            &search_input,
            |this: &mut Self, _input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.rebuild_rows(cx);
                    cx.notify();
                }
            },
        );
        let friend_sub = cx.subscribe(
            &FriendStore::global(cx),
            |this: &mut Self, _store, _event: &FriendEvent, cx| {
                this.rebuild_rows(cx);
                cx.notify();
            },
        );
        search_input.update(cx, |input, cx| input.focus(window, cx));

        let clan_avatar_src = if clan_avatar_url.is_empty() {
            String::new()
        } else {
            imgproxy::avatar_url(cx, &clan_avatar_url)
        };
        let mut modal = Self {
            focus_handle: cx.focus_handle(),
            clan_name,
            clan_avatar_src: SharedString::from(clan_avatar_src),
            clan_avatar_raw: SharedString::from(clan_avatar_url),
            locale,
            invite_link: String::new(),
            invite_link_loading: true,
            invite_link_error: None,
            qr_image: None,
            show_qr: false,
            search_input,
            rows: Vec::new(),
            sent: HashSet::new(),
            sending: HashSet::new(),
            copied: false,
            scroll: UniformListScrollHandle::new(),
            _input_sub: input_sub,
            _friend_sub: friend_sub,
        };
        modal.rebuild_rows(cx);
        modal.load_invite_link(clan_id, channel_id, cx);
        modal
    }

    fn invite_link(&self) -> String {
        self.invite_link.clone()
    }

    fn invite_link_ready(&self) -> bool {
        !self.invite_link_loading
            && self.invite_link_error.is_none()
            && !self.invite_link.is_empty()
    }

    fn load_invite_link(
        &mut self,
        clan_id: ClanId,
        channel_id: Option<ChannelId>,
        cx: &mut Context<Self>,
    ) {
        let Some(clan_list) = ClanList::try_global(cx) else {
            self.invite_link_loading = false;
            self.invite_link_error = Some("Unable to prepare invite link".to_string());
            return;
        };
        let config = AppConfig::try_global(cx).cloned();
        let task = clan_list.update(cx, |store, cx| {
            store.create_invite_link(clan_id, channel_id, cx)
        });

        cx.spawn(async move |this, cx| match task.await {
            Ok(link) => {
                let invite_link = invite_link_from_api(&link, config.as_ref())
                    .ok_or_else(|| "Invite link response is empty".to_string());
                let _ = this.update(cx, |this, cx| match invite_link {
                    Ok(invite_link) => {
                        this.invite_link = invite_link;
                        this.qr_image = build_qr_invite_image(&this.invite_link);
                        this.invite_link_loading = false;
                        this.invite_link_error = None;
                        cx.notify();
                    }
                    Err(err) => {
                        this.invite_link.clear();
                        this.qr_image = None;
                        this.invite_link_loading = false;
                        this.invite_link_error = Some(err);
                        cx.notify();
                    }
                });
            }
            Err(err) => {
                tracing::warn!("create clan invite link failed: {err}");
                let _ = this.update(cx, |this, cx| {
                    this.invite_link.clear();
                    this.qr_image = None;
                    this.invite_link_loading = false;
                    this.invite_link_error = Some("Unable to create invite link".to_string());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn rebuild_rows(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).value().trim().to_lowercase();
        let mut rows: Vec<InviteFriendRow> = FriendStore::global(cx)
            .read(cx)
            .friends()
            .iter()
            .filter(|friend| friend.state == FriendState::Friend)
            .filter(|friend| friend_matches_query(friend, &query))
            .map(|friend| {
                let label = friend.label().to_string();
                let avatar_src = if friend.avatar_url.is_empty() {
                    String::new()
                } else {
                    imgproxy::avatar_url(cx, &friend.avatar_url)
                };
                InviteFriendRow {
                    id: friend.id,
                    label: SharedString::from(label),
                    avatar_src: SharedString::from(avatar_src),
                    avatar_raw: SharedString::from(friend.avatar_url.clone()),
                }
            })
            .collect();
        rows.sort_by_key(|row| row.label.to_lowercase());
        self.rows = rows;
    }

    fn invite_friend(&mut self, user_id: UserId, cx: &mut Context<Self>) {
        if self.sent.contains(&user_id) || self.sending.contains(&user_id) {
            return;
        }
        if !self.invite_link_ready() {
            return;
        }

        let Some(store) = DirectMessageStore::try_global(cx) else {
            Shell::global(cx).update(cx, |shell, cx| {
                shell.error("Unable to open direct messages", cx);
            });
            return;
        };

        self.sending.insert(user_id);
        cx.notify();

        let link = self.invite_link();
        let task = store.update(cx, |store, cx| {
            store.send_direct_text_to_user(user_id, link, cx)
        });
        cx.spawn(async move |this, cx| match task.await {
            Ok(()) => {
                let _ = this.update(cx, |this, cx| {
                    this.sending.remove(&user_id);
                    this.sent.insert(user_id);
                    cx.notify();
                });
            }
            Err(err) => {
                tracing::warn!("send clan invite failed: {err}");
                let message = format!("Unable to send invite: {err}");
                let _ = this.update(cx, |this, cx| {
                    this.sending.remove(&user_id);
                    cx.notify();
                });
                cx.update(|cx| {
                    Shell::global(cx).update(cx, |shell, cx| shell.error(message, cx));
                });
            }
        })
        .detach();
    }

    fn copy_invite_link(&mut self, cx: &mut Context<Self>) {
        if !self.invite_link_ready() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(self.invite_link()));
        self.copied = true;
        cx.notify();
    }

    fn open_qr(&mut self, cx: &mut Context<Self>) {
        if !self.invite_link_ready() {
            return;
        }
        self.show_qr = true;
        cx.notify();
    }

    fn close_qr(&mut self, cx: &mut Context<Self>) {
        self.show_qr = false;
        cx.notify();
    }

    fn copy_qr(&mut self, cx: &mut Context<Self>) {
        let Some(qr) = &self.qr_image else {
            let message =
                mezon_i18n::t(&self.locale, "inviteToChannel.qrModal.errorGenerating").to_string();
            Shell::global(cx).update(cx, |shell, cx| shell.error(message, cx));
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_image(&qr.clipboard));
        let message =
            mezon_i18n::t(&self.locale, "invitation.messages.qrCopiedSuccess").to_string();
        Shell::global(cx).update(cx, |shell, cx| shell.success(message, cx));
    }

    fn close(cx: &mut App) {
        Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
    }
}

impl Render for InvitePeopleModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.show_qr {
            return render_qr_modal(cx.theme(), cx.entity(), self);
        }

        let theme = cx.theme().clone();
        let entity = cx.entity();
        let title = SharedString::from(format!("Invite friends to {}", self.clan_name));
        let invite_link = self.invite_link();
        let copied = self.copied;
        let invite_ready = self.invite_link_ready();
        let invite_loading = self.invite_link_loading;
        let invite_error = self.invite_link_error.clone();

        let rows = self.rows.clone();
        let sent = self.sent.clone();
        let sending = self.sending.clone();
        let list_entity = entity.clone();
        let friend_list = uniform_list(
            "invite-friends-list",
            rows.len(),
            move |range, _window, cx| {
                let theme = cx.theme().clone();
                range
                    .map(|ix| match rows.get(ix) {
                        Some(row) => render_friend_row(
                            &theme,
                            row.clone(),
                            sent.contains(&row.id),
                            sending.contains(&row.id),
                            invite_ready,
                            list_entity.clone(),
                        ),
                        None => div().h(px(74.)).into_any_element(),
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.scroll)
        .h(px(250.))
        .min_h_0();

        let qr_entity = entity.clone();
        let list_body = if self.rows.is_empty() {
            div()
                .h(px(250.))
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.tokens.text_theme_primary)
                .child("No friends found")
                .into_any_element()
        } else {
            div()
                .h(px(250.))
                .min_h_0()
                .overflow_hidden()
                .child(friend_list)
                .into_any_element()
        };

        div()
            .track_focus(&self.focus_handle)
            .key_context("menu")
            .occlude()
            .on_action(cx.listener(|_, _: &::menu::Cancel, _window, cx| Self::close(cx)))
            .w(px(500.))
            .max_w(px(500.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(12.))
            .bg(theme.tokens.theme_setting_primary)
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(px(20.))
                    .py(px(15.))
                    .border_b_1()
                    .border_color(theme.tokens.border_theme_primary)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(20.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.tokens.text_theme_primary)
                            .child(title),
                    )
                    .child(
                        div()
                            .id("invite-people-close")
                            .size(px(30.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .cursor_pointer()
                            .opacity(0.65)
                            .hover(|s| s.opacity(1.0))
                            .on_click(|_: &ClickEvent, _window, cx| Self::close(cx))
                            .child(
                                Icon::new(IconName::Close)
                                    .size(px(24.))
                                    .text_color(theme.text_secondary),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .px(px(24.))
                    .pt(px(20.))
                    .child(
                        Input::new(&self.search_input).text_color(theme.tokens.text_theme_primary),
                    )
                    .child(list_body),
            )
            .child(
                div().px(px(24.)).pb(px(20.)).child(
                    div()
                        .border_t_1()
                        .border_color(theme.tokens.border_theme_primary)
                        .pt(px(18.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .mb(px(8.))
                                .text_size(px(14.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.tokens.text_theme_primary)
                                .child("OR, SEND A CLAN INVITE LINK TO A FRIEND")
                                .child(
                                    div()
                                        .id("invite-copy-qr")
                                        .text_color(theme.text_link)
                                        .when(invite_ready, |el| {
                                            el.cursor_pointer().on_click(
                                                move |_: &ClickEvent, _window, cx| {
                                                    qr_entity
                                                        .update(cx, |this, cx| this.open_qr(cx));
                                                },
                                            )
                                        })
                                        .when(!invite_ready, |el| el.opacity(0.55))
                                        .child("COPY QR"),
                                ),
                        )
                        .child(render_copy_link(
                            &theme,
                            invite_link,
                            copied,
                            invite_loading,
                            invite_error,
                            invite_ready,
                            entity.clone(),
                        )),
                ),
            )
            .into_any_element()
    }
}

fn render_qr_modal(
    theme: &Theme,
    entity: Entity<InvitePeopleModal>,
    modal: &InvitePeopleModal,
) -> AnyElement {
    let mut clan_avatar = Avatar::new()
        .name(SharedString::from(modal.clan_name.clone()))
        .size_px(px(44.));
    if !modal.clan_avatar_src.is_empty() {
        clan_avatar = clan_avatar.src(modal.clan_avatar_src.clone());
        if !modal.clan_avatar_raw.is_empty() && modal.clan_avatar_raw != modal.clan_avatar_src {
            clan_avatar = clan_avatar.fallback_src(modal.clan_avatar_raw.clone());
        }
    } else if !modal.clan_avatar_raw.is_empty() {
        clan_avatar = clan_avatar.src(modal.clan_avatar_raw.clone());
    }

    let cancel_entity = entity.clone();
    let copy_entity = entity.clone();
    let qr_content = match &modal.qr_image {
        Some(qr) => img(qr.render.clone())
            .size(px(320.))
            .object_fit(ObjectFit::Contain)
            .into_any_element(),
        None => div()
            .size(px(320.))
            .flex()
            .items_center()
            .justify_center()
            .text_color(theme.text_secondary)
            .child(mezon_i18n::t(
                &modal.locale,
                "inviteToChannel.qrModal.errorGenerating",
            ))
            .into_any_element(),
    };

    div()
        .track_focus(&modal.focus_handle)
        .key_context("menu")
        .occlude()
        .on_action({
            let entity = entity.clone();
            move |_: &::menu::Cancel, _window, cx| {
                entity.update(cx, |this, cx| this.close_qr(cx));
            }
        })
        .w(px(410.))
        .max_w(px(410.))
        .flex()
        .flex_col()
        .items_center()
        .rounded(px(12.))
        .bg(theme.tokens.theme_setting_primary)
        .shadow_lg()
        .p(px(24.))
        .child(
            div()
                .relative()
                .w(px(360.))
                .pt(px(24.))
                .child(
                    div()
                        .w_full()
                        .rounded(px(6.))
                        .bg(gpui::white())
                        .p(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(qr_content),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(2.))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .size(px(50.))
                                .rounded_full()
                                .bg(gpui::white())
                                .p(px(3.))
                                .child(clan_avatar),
                        ),
                ),
        )
        .child(
            div()
                .mt(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .gap_4()
                .child(
                    Button::new("invite-qr-cancel")
                        .label("Cancel")
                        .ghost()
                        .h(px(50.))
                        .min_w(px(100.))
                        .border_1()
                        .border_color(theme.tokens.border_theme_primary)
                        .on_click(move |_, _window, cx| {
                            cancel_entity.update(cx, |this, cx| this.close_qr(cx));
                        }),
                )
                .child(
                    Button::new("invite-qr-copy")
                        .label("Copy QR")
                        .primary()
                        .h(px(50.))
                        .min_w(px(114.))
                        .text_size(px(18.))
                        .font_weight(FontWeight::BOLD)
                        .on_click(move |_, _window, cx| {
                            copy_entity.update(cx, |this, cx| this.copy_qr(cx));
                        }),
                ),
        )
        .into_any_element()
}

fn friend_matches_query(friend: &Friend, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    friend.label().to_lowercase().contains(query) || friend.username.to_lowercase().contains(query)
}

fn render_friend_row(
    theme: &Theme,
    row: InviteFriendRow,
    sent: bool,
    sending: bool,
    invite_ready: bool,
    entity: Entity<InvitePeopleModal>,
) -> AnyElement {
    let mut avatar = Avatar::new()
        .name(row.label.clone())
        .size_px(px(48.));
    if !row.avatar_src.is_empty() {
        avatar = avatar.src(row.avatar_src.clone());
        if !row.avatar_raw.is_empty() && row.avatar_raw != row.avatar_src {
            avatar = avatar.fallback_src(row.avatar_raw.clone());
        }
    } else if !row.avatar_raw.is_empty() {
        avatar = avatar.src(row.avatar_raw.clone());
    }

    let action = render_invite_action(theme, row.id, sent, sending, invite_ready, entity);

    div()
        .id(SharedString::from(format!("invite-friend-row-{}", row.id)))
        .h(px(74.))
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .px(px(12.))
        .rounded_lg()
        .hover(|s| s.bg(theme.tokens.bg_item_hover))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .min_w_0()
                .flex_1()
                .child(avatar)
                .child(
                    div()
                        .truncate()
                        .text_size(px(18.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.tokens.text_theme_primary)
                        .child(row.label),
                ),
        )
        .child(action)
        .into_any_element()
}

fn render_invite_action(
    theme: &Theme,
    user_id: UserId,
    sent: bool,
    sending: bool,
    invite_ready: bool,
    entity: Entity<InvitePeopleModal>,
) -> AnyElement {
    if sent {
        return div()
            .min_w(px(88.))
            .h(px(42.))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(16.))
            .font_weight(FontWeight::BOLD)
            .text_color(theme.text_secondary)
            .child("Sent")
            .into_any_element();
    }

    let label = if sending { "Sending" } else { "Invite" };
    div()
        .id(SharedString::from(format!("invite-friend-{}", user_id)))
        .min_w(px(88.))
        .h(px(42.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_lg()
        .border_1()
        .border_color(theme.tokens.border_theme_primary)
        .text_size(px(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.tokens.text_theme_primary)
        .when(invite_ready && !sending, |el| {
            el.cursor_pointer()
                .hover(|s| s.bg(theme.status_online))
                .on_click(move |_: &ClickEvent, _window, cx| {
                    entity.update(cx, |this, cx| this.invite_friend(user_id, cx));
                })
        })
        .when(!invite_ready || sending, |el| el.opacity(0.65))
        .child(label)
        .into_any_element()
}

fn render_copy_link(
    theme: &Theme,
    invite_link: String,
    copied: bool,
    loading: bool,
    error: Option<String>,
    invite_ready: bool,
    entity: Entity<InvitePeopleModal>,
) -> AnyElement {
    let button_label = if copied { "Copied" } else { "Copy" };
    let button_bg = if copied {
        theme.status_online
    } else {
        theme.tokens.button_theme_primary
    };
    let display_text = if let Some(error) = error {
        error
    } else if loading {
        "Generating invite link...".to_string()
    } else {
        invite_link
    };

    div()
        .h(px(56.))
        .flex()
        .items_center()
        .overflow_hidden()
        .rounded_lg()
        .border_1()
        .border_color(theme.tokens.border_theme_primary)
        .bg(theme.tokens.bg_input_secondary)
        .child(
            div()
                .min_w_0()
                .flex_1()
                .px(px(20.))
                .truncate()
                .text_size(px(16.))
                .text_color(theme.tokens.text_theme_primary)
                .child(display_text),
        )
        .child(
            div()
                .id("invite-link-copy")
                .h_full()
                .w(px(150.))
                .flex()
                .items_center()
                .justify_center()
                .bg(button_bg)
                .text_size(px(18.))
                .font_weight(FontWeight::BOLD)
                .text_color(gpui::white())
                .when(invite_ready, |el| {
                    el.cursor_pointer().hover(|s| s.opacity(0.9)).on_click(
                        move |_: &ClickEvent, _window, cx| {
                            entity.update(cx, |this, cx| this.copy_invite_link(cx));
                        },
                    )
                })
                .when(!invite_ready, |el| el.opacity(0.65))
                .child(button_label),
        )
        .into_any_element()
}

fn invite_link_for_code(invite_code: &str, config: Option<&AppConfig>) -> String {
    let origin = invite_origin_from_config(config);
    format!("{origin}/invite/{invite_code}")
}

fn invite_link_from_api(link: &ClanInviteLink, config: Option<&AppConfig>) -> Option<String> {
    let invite_code = invite_code_from_link(&link.invite_link).or_else(|| {
        if link.id == 0 {
            None
        } else {
            Some(link.id.to_string())
        }
    })?;
    Some(invite_link_for_code(&invite_code, config))
}

fn invite_code_from_link(link: &str) -> Option<String> {
    let trimmed = link.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    let raw_code = if let Some(index) = lower.find("/invite/") {
        &trimmed[index + "/invite/".len()..]
    } else {
        trimmed
    };
    let code = raw_code.split(['?', '#', '/']).next().unwrap_or("").trim();
    if code.is_empty() {
        None
    } else {
        Some(code.to_string())
    }
}

fn invite_origin_from_config(config: Option<&AppConfig>) -> String {
    const DEV_ORIGIN: &str = "https://dev-mezon.nccsoft.vn";
    const PROD_ORIGIN: &str = "https://mezon.ai";

    let Some(config) = config else {
        return PROD_ORIGIN.to_string();
    };

    let domain = config.domain_url.trim().trim_end_matches('/');
    if is_dev_host(domain)
        || is_dev_host(config.client_host())
        || is_dev_host(&config.api_host)
        || is_dev_host(&config.api_gw_host)
        || is_dev_host(&config.redirect_uri)
        || is_dev_host(&config.oauth2_redirect_uri)
    {
        return DEV_ORIGIN.to_string();
    }

    if domain.is_empty() {
        PROD_ORIGIN.to_string()
    } else {
        domain.to_string()
    }
}

fn is_dev_host(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("dev-mezon") || value.contains("nccsoft")
}

fn build_qr_invite_image(data: &str) -> Option<QrInviteImage> {
    let code = qrcode::QrCode::new(data.as_bytes()).ok()?;
    let width = code.width();
    if width == 0 {
        return None;
    }

    let colors = code.to_colors();
    let scale = (320 / width).max(2);
    let dim = (width * scale) as u32;
    let mut buffer = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_pixel(
        dim,
        dim,
        image::Rgba([255, 255, 255, 255]),
    );

    for (i, color) in colors.iter().enumerate() {
        if *color != qrcode::Color::Dark {
            continue;
        }
        let ox = ((i % width) * scale) as u32;
        let oy = ((i / width) * scale) as u32;
        for dy in 0..scale as u32 {
            for dx in 0..scale as u32 {
                buffer.put_pixel(ox + dx, oy + dy, image::Rgba([0, 0, 0, 255]));
            }
        }
    }

    let render = Arc::new(RenderImage::new(vec![image::Frame::new(buffer.clone())]));
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(buffer)
        .write_to(&mut png, image::ImageFormat::Png)
        .ok()?;
    let clipboard = ClipboardImage::from_bytes(ImageFormat::Png, png.into_inner());

    Some(QrInviteImage { render, clipboard })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_origin_uses_dev_when_runtime_hosts_are_dev() {
        let cfg = AppConfig::dev_defaults();
        assert_eq!(
            invite_link_for_code("42", Some(&cfg)),
            "https://dev-mezon.nccsoft.vn/invite/42"
        );
    }

    #[test]
    fn invite_origin_uses_configured_production_domain() {
        let cfg = AppConfig {
            api_host: "api.mezon.ai".into(),
            api_gw_host: "api.mezon.ai".into(),
            redirect_uri: "https://mezon.ai".into(),
            oauth2_redirect_uri: "https://mezon.ai/login/callback".into(),
            domain_url: "https://mezon.ai/".into(),
            ..AppConfig::dev_defaults()
        };

        assert_eq!(
            invite_link_for_code("42", Some(&cfg)),
            "https://mezon.ai/invite/42"
        );
    }

    #[test]
    fn invite_link_uses_api_invite_id_instead_of_clan_id() {
        let cfg = AppConfig::dev_defaults();
        let link = ClanInviteLink {
            id: 2076277531916374016,
            invite_link: String::new(),
            ..Default::default()
        };

        assert_eq!(
            invite_link_from_api(&link, Some(&cfg)),
            Some("https://dev-mezon.nccsoft.vn/invite/2076277531916374016".to_string())
        );
    }

    #[test]
    fn invite_link_normalizes_api_url_to_runtime_origin() {
        let cfg = AppConfig::dev_defaults();
        let link = ClanInviteLink {
            invite_link: "https://mezon.ai/invite/2076277531916374016?utm=1".into(),
            id: 2076277531916374016,
            ..Default::default()
        };

        assert_eq!(
            invite_link_from_api(&link, Some(&cfg)),
            Some("https://dev-mezon.nccsoft.vn/invite/2076277531916374016".to_string())
        );
    }

    #[test]
    fn qr_invite_image_generates_renderable_and_clipboard_png() {
        let qr = build_qr_invite_image("https://mezon.ai/invite/42").expect("qr image");
        assert_eq!(qr.clipboard.format, ImageFormat::Png);
        assert!(!qr.clipboard.bytes.is_empty());
    }
}
