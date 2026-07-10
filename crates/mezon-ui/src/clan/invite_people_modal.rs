use std::collections::HashSet;

use gpui::{
    AnyElement, App, ClickEvent, ClipboardItem, Context, Entity, FocusHandle, Focusable,
    FontWeight, SharedString, Subscription, UniformListScrollHandle, Window, div, prelude::*, px,
    uniform_list,
};
use mezon_store::{
    ClanId, DirectMessageStore, Friend, FriendEvent, FriendState, FriendStore, UserId,
};

use crate::app::shell::Shell;
use crate::components::primitives::{Avatar, Icon, IconName, Input, InputEvent, InputState};
use crate::theme::{ActiveTheme, Theme};
use crate::util::imgproxy;

const ROW_HEIGHT: f32 = 74.;
const AVATAR_SIZE: f32 = 48.;

#[derive(Clone)]
struct InviteFriendRow {
    id: UserId,
    label: SharedString,
    avatar_src: SharedString,
    avatar_raw: SharedString,
}

pub struct InvitePeopleModal {
    focus_handle: FocusHandle,
    clan_id: ClanId,
    clan_name: String,
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
        clan_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        FriendStore::global(cx).update(cx, |store, cx| store.ensure_loaded(cx));
        DirectMessageStore::global(cx).update(cx, |store, cx| store.ensure_loaded(cx));

        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search for friends")
                .height(px(50.))
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

        let mut modal = Self {
            focus_handle: cx.focus_handle(),
            clan_id,
            clan_name,
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
        modal
    }

    fn invite_link(&self) -> String {
        format!("https://dev-mezon.nccsoft.vn/invite/{}", self.clan_id.get())
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
        cx.write_to_clipboard(ClipboardItem::new_string(self.invite_link()));
        self.copied = true;
        cx.notify();
    }

    fn close(cx: &mut App) {
        Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
    }
}

impl Render for InvitePeopleModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let entity = cx.entity();
        let title = SharedString::from(format!("Invite friends to {}", self.clan_name));
        let invite_link = self.invite_link();
        let copied = self.copied;

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
                            list_entity.clone(),
                        ),
                        None => div().h(px(ROW_HEIGHT)).into_any_element(),
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.scroll)
        .h(px(250.))
        .min_h_0();

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
            .w(px(600.))
            .max_w(px(600.))
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
                    .px(px(24.))
                    .py(px(20.))
                    .border_b_1()
                    .border_color(theme.tokens.border_theme_primary)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(24.))
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
                                        .cursor_pointer()
                                        .text_color(theme.text_link)
                                        .child("COPY QR"),
                                ),
                        )
                        .child(render_copy_link(
                            &theme,
                            invite_link,
                            copied,
                            entity.clone(),
                        )),
                ),
            )
    }
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
    entity: Entity<InvitePeopleModal>,
) -> AnyElement {
    let mut avatar = Avatar::new()
        .name(row.label.clone())
        .size_px(px(AVATAR_SIZE));
    if !row.avatar_src.is_empty() {
        avatar = avatar.src(row.avatar_src.clone());
        if !row.avatar_raw.is_empty() && row.avatar_raw != row.avatar_src {
            avatar = avatar.fallback_src(row.avatar_raw.clone());
        }
    } else if !row.avatar_raw.is_empty() {
        avatar = avatar.src(row.avatar_raw.clone());
    }

    let action = render_invite_action(theme, row.id, sent, sending, entity);

    div()
        .id(SharedString::from(format!("invite-friend-row-{}", row.id)))
        .h(px(ROW_HEIGHT))
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
        .when(!sending, |el| {
            el.cursor_pointer()
                .hover(|s| s.bg(theme.status_online))
                .on_click(move |_: &ClickEvent, _window, cx| {
                    entity.update(cx, |this, cx| this.invite_friend(user_id, cx));
                })
        })
        .when(sending, |el| el.opacity(0.65))
        .child(label)
        .into_any_element()
}

fn render_copy_link(
    theme: &Theme,
    invite_link: String,
    copied: bool,
    entity: Entity<InvitePeopleModal>,
) -> AnyElement {
    let button_label = if copied { "Copied" } else { "Copy" };
    let button_bg = if copied {
        theme.status_online
    } else {
        theme.tokens.button_theme_primary
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
                .child(invite_link),
        )
        .child(
            div()
                .id("invite-link-copy")
                .h_full()
                .w(px(150.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .bg(button_bg)
                .hover(|s| s.opacity(0.9))
                .text_size(px(18.))
                .font_weight(FontWeight::BOLD)
                .text_color(gpui::white())
                .on_click(move |_: &ClickEvent, _window, cx| {
                    entity.update(cx, |this, cx| this.copy_invite_link(cx));
                })
                .child(button_label),
        )
        .into_any_element()
}
