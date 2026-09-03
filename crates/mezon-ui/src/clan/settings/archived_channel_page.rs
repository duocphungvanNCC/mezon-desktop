use gpui::{
    AnyElement, Context, Entity, FontWeight, MouseButton, Render, SharedString, Subscription, Task,
    Window, deferred, div, prelude::*, px,
};
use mezon_store::{ChannelList, ClanId, Settings};
use ui::utils::{DateTimeType, format_distance_from_now};

use crate::app::shell::Shell;
use crate::components::primitives::{
    Button, ButtonVariants, Icon, IconName, Input, InputEvent, InputState, Sizable, Size, h_flex,
    v_flex,
};
use crate::theme::{ActiveTheme, Theme};

#[derive(Clone, PartialEq, Eq)]
struct ArchivedChannelRow {
    channel_id: i64,
    channel_label: SharedString,
    channel_private: bool,
    category_name: SharedString,
    topic: SharedString,
    creator_name: SharedString,
    member_count: i32,
    create_timestamp: Option<i64>,
    age_restricted: bool,
    last_active_timestamp: Option<i64>,
}

const PAGE_SIZES: [usize; 3] = [10, 50, 100];

pub struct ArchivedChannelPage {
    clan_id: ClanId,
    channel_list: Entity<ChannelList>,
    settings: Entity<Settings>,
    channels: Vec<ArchivedChannelRow>,
    search: Option<Entity<InputState>>,
    _search_sub: Option<Subscription>,
    page: usize,
    page_size: usize,
    page_size_picker_open: bool,
    loading: bool,
    fetch_failed: bool,
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
            search: None,
            _search_sub: None,
            page: 0,
            page_size: PAGE_SIZES[0],
            page_size_picker_open: false,
            loading: true,
            fetch_failed: false,
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
        self.fetch_failed = false;
        cx.notify();

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
                        this.fetch_failed = false;
                        this.channels = descs
                            .into_iter()
                            .map(|desc| ArchivedChannelRow {
                                channel_id: desc.channel_id,
                                channel_label: desc.channel_label.into(),
                                channel_private: desc.channel_private,
                                category_name: desc.category_name.into(),
                                topic: desc.topic.into(),
                                creator_name: desc.creator_name.into(),
                                member_count: desc.member_count,
                                create_timestamp: desc.create_timestamp,
                                age_restricted: desc.age_restricted,
                                last_active_timestamp: desc.last_active_timestamp,
                            })
                            .collect();
                        this.page = 0;
                    }
                    Err(err) => {
                        tracing::error!("fetch archived channels failed: {err}");
                        this.channels.clear();
                        this.fetch_failed = true;
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
                if let Ok(()) = result {
                    this.channels.retain(|row| row.channel_id != channel_id);
                    let page_count = this.channels.len().div_ceil(this.page_size).max(1);
                    this.page = this.page.min(page_count - 1);
                    this.channel_list
                        .update(cx, |store, cx| store.refresh_clan(clan_id, cx));
                } else if let Err(err) = &result {
                    tracing::error!("restore archived channel failed: {err}");
                }
                cx.notify();
            });
            cx.update(|cx| {
                Shell::global(cx).update(cx, |shell, cx| {
                    if success {
                        let message =
                            mezon_i18n::t(&locale, "clanSettings.archivedChannels.restoreSuccess")
                                .to_string();
                        shell.success(message, cx);
                    } else {
                        let message =
                            mezon_i18n::t(&locale, "clanSettings.archivedChannels.restoreFailed")
                                .to_string();
                        shell.error(message, cx);
                    }
                });
            });
        }));
    }

    fn format_active_subtitle(timestamp_sec: Option<i64>, locale: &str) -> SharedString {
        let active_label =
            mezon_i18n::t(locale, "clanSettings.archivedChannels.archived").to_uppercase();
        let Some(timestamp_sec) = timestamp_sec.filter(|&t| t > 0) else {
            return active_label.into();
        };
        let now = chrono::Local::now().timestamp();
        let diff = now.saturating_sub(timestamp_sec);
        let time_ago = if diff <= 1 {
            mezon_i18n::t(locale, "common.justNow").to_string()
        } else {
            let Some(utc) = chrono::DateTime::from_timestamp(timestamp_sec, 0) else {
                return active_label.into();
            };
            let naive = utc.with_timezone(&chrono::Local).naive_local();
            format_distance_from_now(DateTimeType::Naive(naive), false, true, false)
        };
        format!("{active_label} {time_ago}").into()
    }

    fn ensure_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() {
            return;
        }
        let placeholder = mezon_i18n::t(
            &self.settings.read(cx).language,
            "clanSettings.archivedChannels.searchPlaceholder",
        )
        .to_string();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .height(px(40.0))
                .radius(px(10.0))
                .padding_x(px(16.0))
                .padding_right(px(42.0))
        });
        self._search_sub = Some(cx.subscribe(&input, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.page = 0;
                this.page_size_picker_open = false;
                cx.notify();
            }
        }));
        self.search = Some(input);
    }

    fn matches_search(row: &ArchivedChannelRow, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let query = query.to_lowercase();
        row.channel_label.to_lowercase().contains(&query)
    }

    fn format_created(timestamp_sec: Option<i64>) -> Option<String> {
        let timestamp = timestamp_sec?;
        chrono::DateTime::from_timestamp(timestamp, 0).map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%b %d, %Y")
                .to_string()
        })
    }

    fn metadata_chip(text: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
        div()
            .px(px(8.0))
            .py(px(3.0))
            .rounded_full()
            .bg(theme.bg_hover)
            .text_xs()
            .text_color(theme.text_secondary)
            .child(text.into())
    }

    fn pagination_item(current: usize, pages: usize) -> Vec<Option<usize>> {
        if pages <= 7 {
            return (0..pages).map(Some).collect();
        }
        let mut items = vec![Some(0)];
        let start = current.saturating_sub(1).max(1);
        let end = (current + 1).min(pages - 2);
        if start > 1 {
            items.push(None);
        }
        items.extend((start..=end).map(Some));
        if end < pages - 2 {
            items.push(None);
        }
        items.push(Some(pages - 1));
        items
    }

    fn page_button(
        &self,
        label: &str,
        disabled: bool,
        selected: bool,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let is_arrow = label.parse::<usize>().is_err();
        let is_left = label == "‹";
        div()
            .id(format!("archived-page-{label}-{selected}-{disabled}"))
            .w(px(40.0))
            .h(px(32.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .border_1()
            .border_color(if selected {
                cx.theme().text_primary
            } else {
                cx.theme().border
            })
            .bg(if selected {
                cx.theme().tokens.bg_active_button
            } else {
                cx.theme().brand
            })
            .text_color(if is_arrow {
                gpui::white()
            } else {
                gpui::Hsla::from(cx.theme().text_primary)
            })
            .when(disabled, |el| el.opacity(0.5))
            .when(!disabled, |el| el.cursor_pointer())
            .when(is_arrow, |el| {
                el.child(
                    Icon::new(IconName::ArrowRight)
                        .size(px(20.0))
                        .text_color(gpui::white())
                        .when(is_left, |icon| {
                            icon.with_transformation(gpui::Transformation::rotate(gpui::radians(
                                std::f32::consts::PI,
                            )))
                        }),
                )
            })
            .when(!is_arrow, |el| el.child(label.to_string()))
    }

    fn render_pagination(&self, pages: usize, cx: &mut Context<Self>) -> AnyElement {
        if pages <= 1 {
            return div().into_any_element();
        }
        let mut bar = h_flex().items_center().gap_2();
        bar = bar.child(
            self.page_button("‹", self.page == 0, false, cx)
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.page > 0 {
                        this.page -= 1;
                        cx.notify();
                    }
                })),
        );
        for item in Self::pagination_item(self.page, pages) {
            if let Some(page) = item {
                bar = bar.child(
                    self.page_button(&(page + 1).to_string(), false, page == self.page, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.page = page;
                            cx.notify();
                        })),
                );
            } else {
                bar = bar.child(
                    div()
                        .px(px(4.0))
                        .text_color(cx.theme().text_secondary)
                        .child("…"),
                );
            }
        }
        bar.child(
            self.page_button("›", self.page + 1 >= pages, false, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.page + 1 < pages {
                        this.page += 1;
                        cx.notify();
                    }
                })),
        )
        .into_any_element()
    }

    fn page_size_control(&self, locale: &str, cx: &mut Context<Self>) -> AnyElement {
        let open = self.page_size_picker_open;
        let chevron_angle = if open { std::f32::consts::PI } else { 0.0 };
        let mut control = div().relative().w(px(68.0)).child(
            div()
                .id("archived-page-size")
                .w_full()
                .h(px(32.0))
                .px_2()
                .flex()
                .items_center()
                .justify_between()
                .rounded(px(6.0))
                .border_1()
                .border_color(cx.theme().border)
                .cursor_pointer()
                .hover(|style| style.bg(cx.theme().bg_hover))
                .child(self.page_size.to_string())
                .child(
                    Icon::new(IconName::ChevronDown)
                        .size(px(14.0))
                        .text_color(cx.theme().text_secondary)
                        .with_transformation(gpui::Transformation::rotate(gpui::radians(
                            chevron_angle,
                        ))),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.page_size_picker_open = !open;
                    cx.notify();
                })),
        );
        if open {
            let mut menu = div()
                .absolute()
                .bottom(px(36.0))
                .left_0()
                .w(px(68.0))
                .p_1()
                .rounded(px(6.0))
                .bg(cx.theme().bg_floating)
                .border_1()
                .border_color(cx.theme().border)
                .shadow_lg()
                .occlude();
            for size in PAGE_SIZES {
                let selected = size == self.page_size;
                menu = menu.child(
                    div()
                        .id(format!("archived-page-size-{size}"))
                        .h(px(28.0))
                        .px_2()
                        .flex()
                        .items_center()
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .when(selected, |style| style.bg(cx.theme().bg_hover))
                        .hover(|style| style.bg(cx.theme().bg_hover))
                        .child(size.to_string())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.page_size = size;
                                this.page = 0;
                                this.page_size_picker_open = false;
                                cx.notify();
                            }),
                        ),
                );
            }
            control = control.child(deferred(menu));
        }
        h_flex()
            .id("archived-page-size-control")
            .items_center()
            .gap_2()
            .child(mezon_i18n::t(
                locale,
                "channelSetting.table.pagination.show",
            ))
            .child(control)
            .child(mezon_i18n::t(
                locale,
                "channelSetting.table.pagination.channelOf",
            ))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if this.page_size_picker_open {
                    this.page_size_picker_open = false;
                    cx.notify();
                }
            }))
            .into_any_element()
    }

    fn render_empty_state(locale: &str, theme: &Theme) -> impl IntoElement {
        v_flex()
            .items_center()
            .justify_center()
            .py(px(48.0))
            .child(
                div().opacity(0.6).child(
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

    fn render_fetch_error(locale: &str, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .items_center()
            .justify_center()
            .gap(px(12.0))
            .py(px(48.0))
            .child(
                div()
                    .text_base()
                    .text_color(theme.status_dnd)
                    .text_center()
                    .child(mezon_i18n::t(
                        locale,
                        "clanSettings.archivedChannels.fetchFailed",
                    )),
            )
            .child(
                Button::new("archived-channels-retry")
                    .label(mezon_i18n::t(locale, "channelVoice.retry"))
                    .with_size(Size::Medium)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.fetch_archived_channels(cx);
                    })),
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
        let subtitle = Self::format_active_subtitle(row.last_active_timestamp, locale);
        let created = Self::format_created(row.create_timestamp);
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
                    )
                    .when(!row.topic.is_empty(), |el| {
                        el.child(
                            div()
                                .mt(px(6.0))
                                .max_w(px(620.0))
                                .text_sm()
                                .text_color(theme.text_secondary)
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(row.topic.clone()),
                        )
                    })
                    .child(
                        h_flex()
                            .mt(px(8.0))
                            .gap(px(6.0))
                            .flex_wrap()
                            .when(!row.category_name.is_empty(), |el| {
                                el.child(Self::metadata_chip(
                                    format!("Category: {}", row.category_name),
                                    theme,
                                ))
                            })
                            .when(!row.creator_name.is_empty(), |el| {
                                el.child(Self::metadata_chip(
                                    format!("Created by {}", row.creator_name),
                                    theme,
                                ))
                            })
                            .when(row.member_count > 0, |el| {
                                el.child(Self::metadata_chip(
                                    format!("{} members", row.member_count),
                                    theme,
                                ))
                            })
                            .when_some(created, |el, created| {
                                el.child(Self::metadata_chip(format!("Created {created}"), theme))
                            })
                            .when(row.age_restricted, |el| {
                                el.child(Self::metadata_chip("Age restricted", theme))
                            }),
                    ),
            )
            .child(
                Button::new(format!("archived-channel-restore-{channel_id}"))
                    .label(mezon_i18n::t(
                        locale,
                        "clanSettings.archivedChannels.restore",
                    ))
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_search(window, cx);
        let theme = cx.theme().clone();
        let locale = self.settings.read(cx).language.clone();
        let query = self
            .search
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_string())
            .unwrap_or_default();
        let filtered = self
            .channels
            .iter()
            .filter(|row| Self::matches_search(row, &query))
            .collect::<Vec<_>>();
        let total = filtered.len();
        let pages = total.div_ceil(self.page_size).max(1);
        self.page = self.page.min(pages - 1);
        let visible = filtered
            .into_iter()
            .skip(self.page * self.page_size)
            .take(self.page_size)
            .cloned()
            .collect::<Vec<_>>();

        v_flex()
            .h_full()
            .min_h_0()
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
            .when(!self.loading && !self.fetch_failed, |el| {
                el.child(
                    div()
                        .relative()
                        .w_full()
                        .mb(px(20.0))
                        .child(Input::new(
                            self.search.as_ref().expect("search initialized"),
                        ))
                        .child(
                            div()
                                .absolute()
                                .right(px(12.0))
                                .top_0()
                                .bottom_0()
                                .flex()
                                .items_center()
                                .child(
                                    Icon::new(IconName::Search)
                                        .size(px(18.0))
                                        .text_color(theme.text_secondary),
                                ),
                        ),
                )
            })
            .when(self.loading, |el| el.child(div().h(px(48.0))))
            .when(!self.loading && self.fetch_failed, |el| {
                el.child(Self::render_fetch_error(&locale, &theme, cx))
            })
            .when(
                !self.loading && !self.fetch_failed && self.channels.is_empty(),
                |el| el.child(Self::render_empty_state(&locale, &theme)),
            )
            .when(
                !self.loading && !self.fetch_failed && !self.channels.is_empty() && total == 0,
                |el| {
                    el.child(
                        v_flex()
                            .items_center()
                            .py(px(48.0))
                            .text_color(theme.text_secondary)
                            .child(mezon_i18n::t(
                                &locale,
                                "clanSettings.archivedChannels.noSearchResults",
                            )),
                    )
                },
            )
            .when(!self.loading && !self.fetch_failed && total > 0, |el| {
                el.child(
                    div()
                        .id("archived-channel-list-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .pr(px(4.0))
                        .child(
                            v_flex()
                                .gap(px(12.0))
                                .children(visible.iter().enumerate().map(|(index, row)| {
                                    self.render_channel_row(index, row, &locale, &theme, cx)
                                        .into_any_element()
                                })),
                        ),
                )
                .child(
                    h_flex()
                        .flex_shrink_0()
                        .w_full()
                        .h(px(72.0))
                        .items_center()
                        .justify_between()
                        .border_t_1()
                        .border_color(theme.border)
                        .text_sm()
                        .text_color(theme.text_secondary)
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(self.page_size_control(&locale, cx))
                                .child(total.to_string()),
                        )
                        .child(self.render_pagination(pages, cx)),
                )
            })
    }
}
