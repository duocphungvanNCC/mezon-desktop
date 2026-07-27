use std::collections::HashSet;

use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, MouseButton,
    SharedString, Subscription, Window, deferred, div, prelude::*, px, relative,
};
use mezon_store::{
    AccountStore, BadgeService, DirectMessageStore, FriendStore, UserId, UsersByUserStore,
    WalletStore,
};

use crate::app::shell::Shell;
use crate::components::primitives::{
    Avatar, Button, ButtonVariants, Icon, IconName, Input, InputEvent, InputState,
};
use crate::theme::ActiveTheme;

const DECIMAL_FACTOR: i128 = 1_000_000;
const MAX_CANDIDATES_SHOWN: usize = 50;

#[derive(Clone)]
struct Candidate {
    id: String,
    username: SharedString,
    avatar: SharedString,
    filter_key: String,
}

pub struct SendTokenModal {
    focus_handle: FocusHandle,
    locale: SharedString,
    search: Entity<InputState>,
    amount: Entity<InputState>,
    note: Entity<InputState>,
    candidates: Vec<Candidate>,
    visible: Vec<Candidate>,
    selected: Option<(String, SharedString)>,
    dropdown_open: bool,
    error: Option<SharedString>,
    sending: bool,
    suppress_search_change: bool,
    _search_sub: Subscription,
    _amount_sub: Subscription,
}

impl Focusable for SendTokenModal {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl SendTokenModal {
    pub fn open(
        locale: SharedString,
        prefill: Option<(String, String)>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if Shell::global(cx).read(cx).has_modal() {
            return;
        }
        let candidates = Self::build_candidates(cx);
        let view = cx.new(|cx| {
            let tr = |key: &'static str| mezon_i18n::t(&locale, key).to_string();
            let search = cx.new(|cx| {
                InputState::new(window, cx).placeholder(tr(
                    "userProfile.statusProfile.sendTokenModal.placeholders.searchUsers",
                ))
            });
            let amount = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(tr(
                        "userProfile.statusProfile.sendTokenModal.placeholders.amountPlaceholder",
                    ))
                    .validate(|value, _cx| value.chars().all(|c| c.is_ascii_digit()))
            });
            let note = cx.new(|cx| {
                InputState::new(window, cx).placeholder(tr(
                    "userProfile.statusProfile.sendTokenModal.placeholders.notePlaceholder",
                ))
            });
            let default_note = tr("common.transferFunds");
            let search_sub = cx.subscribe(
                &search,
                |this: &mut Self, _input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        if this.suppress_search_change {
                            this.suppress_search_change = false;
                        } else {
                            this.selected = None;
                            this.dropdown_open = true;
                        }
                        this.recompute_visible(cx);
                        cx.notify();
                    }
                },
            );
            let amount_sub = cx.subscribe(
                &amount,
                |this: &mut Self, _input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.error = None;
                        cx.notify();
                    }
                },
            );

            let mut this = Self {
                focus_handle: cx.focus_handle(),
                locale,
                search,
                amount,
                note,
                candidates,
                visible: Vec::new(),
                selected: None,
                dropdown_open: false,
                error: None,
                sending: false,
                suppress_search_change: false,
                _search_sub: search_sub,
                _amount_sub: amount_sub,
            };
            this.note
                .update(cx, |input, cx| input.set_value(default_note, window, cx));
            this.recompute_visible(cx);
            if let Some((id, username)) = prefill {
                this.select_recipient(id, username.into(), window, cx);
            }
            this
        });
        let focus_handle = view.read(cx).focus_handle.clone();
        window.focus(&focus_handle, cx);
        Shell::global(cx).update(cx, |shell, cx| shell.show_modal(view.into(), cx));
    }

    fn build_candidates(cx: &App) -> Vec<Candidate> {
        let me = BadgeService::try_global(cx).and_then(|b| b.read(cx).current_user_id(cx));
        let me_id = me.map(|id| id.0.to_string());
        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<Candidate> = Vec::new();

        if let Some(store) = FriendStore::try_global(cx) {
            for friend in store.read(cx).friends() {
                let id = friend.id.0.to_string();
                Self::push_candidate(
                    &mut out,
                    &mut seen,
                    me_id.as_deref(),
                    cx,
                    id,
                    &friend.username,
                    &friend.avatar_url,
                );
            }
        }
        if let Some(store) = UsersByUserStore::try_global(cx) {
            for user in store.read(cx).users() {
                let id = user.id.0.to_string();
                Self::push_candidate(
                    &mut out,
                    &mut seen,
                    me_id.as_deref(),
                    cx,
                    id,
                    &user.username,
                    &user.avatar_url,
                );
            }
        }
        out
    }

    fn push_candidate(
        out: &mut Vec<Candidate>,
        seen: &mut HashSet<String>,
        me_id: Option<&str>,
        cx: &App,
        id: String,
        username: &str,
        avatar_url: &str,
    ) {
        if id.is_empty() || id == "0" || Some(id.as_str()) == me_id || username.is_empty() {
            return;
        }
        if !seen.insert(id.clone()) {
            return;
        }
        let avatar = if avatar_url.is_empty() {
            SharedString::default()
        } else {
            SharedString::from(crate::util::imgproxy::avatar_url(cx, avatar_url))
        };
        out.push(Candidate {
            id,
            username: SharedString::from(username.to_string()),
            avatar,
            filter_key: username.to_lowercase(),
        });
    }

    fn close(cx: &mut App) {
        Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
    }

    fn select_recipient(
        &mut self,
        id: String,
        username: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.suppress_search_change = true;
        self.search.update(cx, |input, cx| {
            input.set_value(username.clone(), window, cx)
        });
        self.selected = Some((id, username));
        self.dropdown_open = false;
        cx.notify();
    }

    fn recompute_visible(&mut self, cx: &App) {
        let needle = self.search.read(cx).value().trim().to_lowercase();
        let visible: Vec<Candidate> = self
            .candidates
            .iter()
            .filter(|candidate| needle.is_empty() || candidate.filter_key.contains(&needle))
            .take(MAX_CANDIDATES_SHOWN)
            .cloned()
            .collect();
        self.visible = visible;
    }

    fn amount_value(&self, cx: &App) -> i64 {
        self.amount
            .read(cx)
            .value()
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(0)
    }

    fn exceeds_balance(&self, cx: &App) -> bool {
        let amount = self.amount_value(cx);
        if amount <= 0 {
            return false;
        }
        let balance =
            WalletStore::try_global(cx).and_then(|w| w.read(cx).balance().map(|b| b.to_string()));
        let Some(balance) = balance else {
            return false;
        };
        let scaled = (amount as i128).saturating_mul(DECIMAL_FACTOR);
        let available: i128 = balance.trim().parse().unwrap_or(0);
        scaled > available
    }

    fn can_send(&self, cx: &App) -> bool {
        !self.sending
            && self.selected.is_some()
            && self.amount_value(cx) > 0
            && !self.exceeds_balance(cx)
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        if !self.can_send(cx) {
            return;
        }
        let Some((recipient, recipient_username)) = self.selected.clone() else {
            return;
        };
        let amount = self.amount_value(cx);
        let note = {
            let text = self.note.read(cx).value().trim().to_string();
            (!text.is_empty()).then_some(text)
        };
        let card_note = note
            .clone()
            .unwrap_or_else(|| mezon_i18n::t(&self.locale, "common.transferFunds").to_string());
        let card_text = format!(
            "Funds Transferred: {}₫ | {card_note}",
            format_thousands(amount)
        );
        let recipient_user_id = recipient.parse::<i64>().ok().map(UserId);
        let Some(sender_id) =
            BadgeService::try_global(cx).and_then(|b| b.read(cx).current_user_id(cx))
        else {
            self.error = Some("Wallet is not available".into());
            cx.notify();
            return;
        };
        let sender = sender_id.0.to_string();
        let sender_username = AccountStore::try_global(cx)
            .and_then(|a| {
                a.read(cx)
                    .account
                    .as_ref()
                    .map(|acct| acct.username.clone())
            })
            .unwrap_or_default();

        self.sending = true;
        self.error = None;
        cx.notify();

        let task = WalletStore::global(cx).update(cx, |wallet, cx| {
            wallet.send_token(
                sender,
                sender_username,
                recipient,
                amount,
                note,
                None,
                false,
                cx,
            )
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.sending = false;
                match result {
                    Ok(_) => {
                        if let Some(user_id) = recipient_user_id {
                            let card = DirectMessageStore::global(cx).update(cx, |store, cx| {
                                store.create_dm_and_send_token_card(
                                    user_id,
                                    recipient_username.to_string(),
                                    card_text.clone(),
                                    cx,
                                )
                            });
                            card.detach();
                        }
                        let message =
                            mezon_i18n::t(&this.locale, "token.toast.success.sendSuccess")
                                .to_string();
                        Shell::global(cx).update(cx, |shell, cx| shell.success(message, cx));
                        Self::close(cx);
                    }
                    Err(error) => {
                        let message: SharedString = error.into();
                        Shell::global(cx).update(cx, |shell, cx| shell.error(message.clone(), cx));
                        this.error = Some(message);
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }
}

impl Render for SendTokenModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let entity = cx.entity();
        let locale = self.locale.clone();
        let tk = |key: &'static str| mezon_i18n::t(&locale, key).to_string();
        let can_send = self.can_send(cx);
        let exceeds = self.exceeds_balance(cx);

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .p_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.tokens.text_theme_primary)
                            .child(tk("userProfile.statusProfile.sendTokenModal.title")),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.tokens.text_secondary)
                            .child(tk("userProfile.statusProfile.sendTokenModal.description")),
                    ),
            )
            .child(
                div()
                    .id("send-token-close")
                    .size_6()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.tokens.bg_item_hover))
                    .on_click(|_: &ClickEvent, _window, cx| Self::close(cx))
                    .child(
                        Icon::new(IconName::Close)
                            .size(px(18.))
                            .text_color(theme.tokens.text_theme_primary),
                    ),
            );

        let mut recipient_field = div().relative().child(
            div()
                .id("send-token-search")
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.dropdown_open = true;
                    this.recompute_visible(cx);
                    cx.notify();
                }))
                .child(
                    Input::new(&self.search)
                        .w_full()
                        .text_color(theme.tokens.text_secondary),
                ),
        );
        if self.dropdown_open {
            let mut menu = div()
                .id("send-token-dropdown")
                .absolute()
                .top_full()
                .left_0()
                .right_0()
                .mt_1()
                .flex()
                .flex_col()
                .min_h_0()
                .max_h(px(240.))
                .overflow_y_scroll()
                .rounded_md()
                .border_1()
                .border_color(theme.tokens.border_primary)
                .bg(theme.tokens.theme_setting_primary)
                .shadow_lg()
                .occlude()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if this.dropdown_open {
                        this.dropdown_open = false;
                        cx.notify();
                    }
                }));
            if self.visible.is_empty() {
                menu = menu.child(
                    div()
                        .px_3()
                        .py_3()
                        .text_sm()
                        .text_color(theme.tokens.text_secondary)
                        .child(tk("userProfile.statusProfile.sendTokenModal.noUsersFound")),
                );
            }
            for candidate in &self.visible {
                let id = candidate.id.clone();
                let username = candidate.username.clone();
                menu = menu.child(
                    div()
                        .id(SharedString::from(format!(
                            "send-token-user-{}",
                            candidate.id
                        )))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .px_3()
                        .py_2()
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.tokens.bg_item_hover))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                this.select_recipient(id.clone(), username.clone(), window, cx);
                            }),
                        )
                        .child(
                            Avatar::new()
                                .name(candidate.username.clone())
                                .src(candidate.avatar.clone())
                                .size_px(px(28.)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.tokens.text_secondary)
                                .child(candidate.username.clone()),
                        ),
                );
            }
            recipient_field = recipient_field.child(deferred(menu));
        }

        let recipient_section = section(
            theme,
            tk("userProfile.statusProfile.sendTokenModal.fields.to"),
        )
        .child(recipient_field);

        let amount_section = section(
            theme,
            tk("userProfile.statusProfile.sendTokenModal.fields.amount"),
        )
        .child(
            div()
                .relative()
                .child(
                    Input::new(&self.amount)
                        .w_full()
                        .text_color(theme.tokens.text_secondary),
                )
                .child(
                    div()
                        .absolute()
                        .right_3()
                        .top_0()
                        .bottom_0()
                        .flex()
                        .items_center()
                        .text_color(theme.tokens.text_theme_primary)
                        .child(tk("userProfile.statusProfile.sendTokenModal.currency")),
                ),
        )
        .when(exceeds, |d| {
            d.child(div().text_xs().text_color(gpui::rgb(0xef4444)).child(tk(
                "userProfile.statusProfile.sendTokenModal.errors.exceedWalletBalance",
            )))
        });

        let note_section = section(
            theme,
            tk("userProfile.statusProfile.sendTokenModal.fields.note"),
        )
        .child(
            Input::new(&self.note)
                .w_full()
                .text_color(theme.tokens.text_secondary),
        );

        let mut body = div()
            .flex()
            .flex_col()
            .gap_4()
            .px_4()
            .pb_2()
            .child(recipient_section)
            .child(amount_section)
            .child(note_section);
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(gpui::rgb(0xef4444))
                    .child(error.clone()),
            );
        }

        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .p_4()
            .child(
                Button::new("send-token-cancel")
                    .label(tk(
                        "userProfile.statusProfile.sendTokenModal.buttons.cancel",
                    ))
                    .on_click(|_: &ClickEvent, _window, cx| Self::close(cx)),
            )
            .child(
                Button::new("send-token-submit")
                    .primary()
                    .label(tk(
                        "userProfile.statusProfile.sendTokenModal.buttons.sendTokens",
                    ))
                    .disabled(!can_send)
                    .on_click(move |_: &ClickEvent, _window, cx| {
                        entity.update(cx, |this, cx| this.send(cx));
                    }),
            );

        div()
            .track_focus(&self.focus_handle)
            .key_context("menu")
            .on_action(cx.listener(|this, _: &::menu::Cancel, _window, cx| {
                if this.dropdown_open {
                    this.dropdown_open = false;
                    cx.notify();
                    return;
                }
                Self::close(cx);
            }))
            .occlude()
            .w(px(480.))
            .max_h(relative(0.85))
            .flex()
            .flex_col()
            .rounded(px(12.))
            .bg(theme.tokens.theme_setting_primary)
            .shadow_lg()
            .child(header)
            .child(
                div()
                    .id("send-token-body-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
            .child(footer)
    }
}

fn format_thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let bytes = digits.as_bytes();
    let mut out = String::new();
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 && (bytes.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*byte as char);
    }
    if n < 0 { format!("-{out}") } else { out }
}

fn section(theme: &crate::theme::Theme, label: impl Into<SharedString>) -> gpui::Div {
    div().flex().flex_col().gap_2().child(
        div()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.tokens.text_theme_primary)
            .child(label.into()),
    )
}
