use gpui::{
    Context, Entity, FontWeight, Render, SharedString, Task, Window, div, prelude::*, px,
};
use mezon_store::{ChannelList, ClanId, Settings};
use ui::utils::{DateTimeType, format_distance_from_now};

use crate::app::shell::Shell;
use crate::components::primitives::{
    Button, ButtonVariants, Icon, IconName, Sizable, Size, h_flex, v_flex,
};
use crate::theme::{ActiveTheme, Theme};

#[derive(Clone, PartialEq, Eq)]
struct ArchivedChannelRow {
    channel_id: i64,
    channel_label: SharedString,
    channel_private: bool,
    last_active_timestamp: Option<i64>,
}

pub struct ArchivedChannelPage {
    clan_id: ClanId,
    channel_list: Entity<ChannelList>,
    settings: Entity<Settings>,
    channels: Vec<ArchivedChannelRow>,
    loading: bool,
    restoring: Option<i64>,
    _fetch_task: Option<Task<()>>,
    _restore_task: Option<Task<()>>,
}

impl ArchivedChannelPage {
    pub fn new(
        clan_id: ClanId,
        channel_list: Entity<ChannelList>,
        settings: Entity<Settings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            clan_id,
            channel_list,
            settings,
            channels: Vec::new(),
            loading: true,
            restoring: None,
            _fetch_task: None,
            _restore_task: None,
        };
        this.fetch_archived_channels(cx);
        this
    }

    pub fn release(&mut self) {
        self._fetch_task.take();
        self._restore_task.take();
    }

    fn fetch_archived_channels(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        let clan_id = self.clan_id;
        self._fetch_task = Some(cx.spawn(async move |this, cx| {
            let task = this
                .update(cx, |this, cx| {
                    this.channel_list
                        .update(cx, |store, cx| store.fetch_archived_channels(clan_id, cx))
                })
                .ok();
            let Some(task) = task else {
                return;
            };
            let fetched = task.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match fetched {
                    Ok(descs) => {
                        this.channels = descs
                            .into_iter()
                            .map(|desc| ArchivedChannelRow {
                                channel_id: desc.channel_id,
                                channel_label: desc.channel_label.into(),
                                channel_private: desc.channel_private,
                                last_active_timestamp: desc.last_active_timestamp,
                            })
                            .collect();
                    }
                    Err(err) => {
                        tracing::error!("fetch archived channels failed: {err}");
                    }
                }
                cx.notify();
            });
        }));
    }

    fn restore_channel(&mut self, channel_id: i64, cx: &mut Context<Self>) {
        if self.restoring.is_some() {
            return;
        }
        self.restoring = Some(channel_id);
        cx.notify();

        let clan_id = self.clan_id;
        let locale = self.settings.read(cx).language.clone();
        self._restore_task = Some(cx.spawn(async move |this, cx| {
            let task = this
                .update(cx, |this, cx| {
                    this.channel_list.update(cx, |store, cx| {
                        store.restore_archived_channel(clan_id, channel_id, cx)
                    })
                })
                .ok();
            let Some(task) = task else {
                return;
            };
            let result = task.await;
            let success = result.is_ok();
            let _ = this.update(cx, |this, cx| {
                this.restoring = None;
                match result {
                    Ok(()) => {
                        this.channels.retain(|row| row.channel_id != channel_id);
                        this.channel_list
                            .update(cx, |store, cx| store.refresh_clan(clan_id, cx));
                    }
                    Err(err) => {
                        tracing::error!("restore archived channel failed: {err}");
                    }
                }
                cx.notify();
            });
            if success {
                let message =
                    mezon_i18n::t(&locale, "clanSettings.archivedChannels.restoreSuccess")
                        .to_string();
                cx.update(|cx| {
                    Shell::global(cx).update(cx, |shell, cx| shell.success(message, cx));
                });
            }
        }));
    }

    fn format_archived_subtitle(timestamp_sec: Option<i64>, locale: &str) -> SharedString {
        let archived_label =
            mezon_i18n::t(locale, "clanSettings.archivedChannels.archived").to_uppercase();
        let Some(timestamp_sec) = timestamp_sec.filter(|&t| t > 0) else {
            return archived_label.into();
        };
        let now = chrono::Local::now().timestamp();
        let diff = now.saturating_sub(timestamp_sec);
        let time_ago = if diff <= 1 {
            mezon_i18n::t(locale, "common.justNow").to_string()
        } else {
            let Some(utc) = chrono::DateTime::from_timestamp(timestamp_sec, 0) else {
                return archived_label.into();
            };
            let naive = utc.with_timezone(&chrono::Local).naive_local();
            format_distance_from_now(DateTimeType::Naive(naive), false, true, false)
        };
        format!("{archived_label} {time_ago}").into()
    }

    fn render_empty_state(locale: &str, theme: &Theme) -> impl IntoElement {
        v_flex()
            .items_center()
            .justify_center()
            .py(px(48.0))
            .child(
                div()
                    .opacity(0.6)
                    .child(
                        Icon::new(IconName::Hashtag)
                            .size(px(48.0))
                            .text_color(theme.text_primary),
                    ),
            )
            .child(
                div()
                    .mt(px(12.0))
                    .text_base()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_primary)
                    .opacity(0.6)
                    .child(mezon_i18n::t(
                        locale,
                        "clanSettings.archivedChannels.emptyState",
                    )),
            )
    }

    fn render_channel_row(
        &self,
        index: usize,
        row: &ArchivedChannelRow,
        locale: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let channel_id = row.channel_id;
        let icon = if row.channel_private {
            IconName::HashtagLocked
        } else {
            IconName::Hashtag
        };
        let subtitle = Self::format_archived_subtitle(row.last_active_timestamp, locale);
        let restoring = self.restoring == Some(channel_id);

        h_flex()
            .id(("archived-channel-row", index))
            .w_full()
            .items_center()
            .gap(px(8.0))
            .px(px(16.0))
            .py(px(12.0))
            .rounded_lg()
            .bg(theme.tokens.bg_item_theme_hover)
            .shadow_sm()
            .child(
                div()
                    .flex_shrink_0()
                    .size(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    // .rounded_md()
                    // .bg(theme.tokens.bg_icon_theme_active)
                    .child(
                        Icon::new(icon)
                            .size(px(20.0))
                            .text_color(theme.tokens.bg_icon_theme),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text_primary)
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(row.channel_label.clone()),
                    )
                    .child(
                        div()
                            .mt(px(2.0))
                            .text_xs()
                            .text_color(theme.text_primary)
                            .child(subtitle),
                    ),
            )
            .child(
                Button::new(format!("archived-channel-restore-{channel_id}"))
                    .label(mezon_i18n::t(locale, "clanSettings.archivedChannels.restore"))
                    .primary()
                    .with_size(Size::Medium)
                    .disabled(restoring)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.restore_channel(channel_id, cx);
                    })),
            )
    }
}

impl Render for ArchivedChannelPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let locale = self.settings.read(cx).language.clone();

        v_flex()
            .w_full()
            .pt(px(8.0))
            .child(
                div()
                    .mb(px(24.0))
                    .max_w(px(672.0))
                    .text_base()
                    .text_color(theme.text_secondary)
                    .child(mezon_i18n::t(
                        &locale,
                        "clanSettings.archivedChannels.description",
                    )),
            )
            .when(self.loading, |el| el.child(div().h(px(48.0))))
            .when(!self.loading && self.channels.is_empty(), |el| {
                el.child(Self::render_empty_state(&locale, &theme))
            })
            .when(!self.loading && !self.channels.is_empty(), |el| {
                el.child(
                    v_flex()
                        .gap(px(12.0))
                        .children(self.channels.iter().enumerate().map(|(index, row)| {
                            self.render_channel_row(index, row, &locale, &theme, cx)
                                .into_any_element()
                        })),
                )
            })
    }
}
