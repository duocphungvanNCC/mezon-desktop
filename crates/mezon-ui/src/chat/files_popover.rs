use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, ClickEvent, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, ListAlignment, ListState, MouseDownEvent, SharedString, Task, Window, div, list,
    prelude::*, px, svg,
};
use mezon_store::{
    ChannelDocument, ChannelId, ClanId, ClanMembersStore, FilesStore, Settings,
    download_url_with_dialog, filename_matches_query,
};
use ui::{PopoverMenuHandle, ScrollAxes, Scrollbars, WithScrollbar};

use crate::chat::file_type_icon::file_type_icon_for;
use crate::components::primitives::{
    Button, ButtonVariants, Icon, IconName, Input, InputEvent, InputState, Sizable, Size, Spinner,
    h_flex, v_flex,
};
use crate::router::{Route, Router};
use crate::theme::{ActiveTheme, Theme};

const POPOVER_WIDTH: f32 = 540.;
const HEADER_HEIGHT: f32 = 48.;
const MIN_POPOVER_HEIGHT: f32 = 400.;
const LIST_OVERDRAW: f32 = 200.;
const MAX_VH: f32 = 0.8;
const SEARCH_WIDTH: f32 = 224.;
const SEARCH_DEBOUNCE_MS: u64 = 200;
const LIST_PAD_X: f32 = 16.;

#[derive(Clone)]
struct FileRowVm {
    url: SharedString,
    filename: SharedString,
    shared_by: SharedString,
    download_label: SharedString,
    type_badge: SharedString,
    icon: IconName,
    failed: bool,
}

impl FileRowVm {
    fn from_doc(doc: &ChannelDocument, locale: &str) -> Self {
        let time = format_file_time(doc.create_time_seconds as i64, locale);
        let username = if doc.uploader_name.is_empty() {
            "Unknown"
        } else {
            doc.uploader_name.as_ref()
        };
        let shared_by = mezon_i18n::t(locale, "channelTopbar.fileItem.sharedBy")
            .replace("{{username}}", username)
            .replace("{{time}}", &time);
        let type_badge = mezon_store::short_file_type_label_for(&doc.filetype, &doc.filename);
        let download_label = format!(
            "{} {}",
            mezon_i18n::t(locale, "channelTopbar.fileItem.download"),
            type_badge.as_ref()
        );
        Self {
            url: doc.url.clone().into(),
            filename: doc.filename.clone().into(),
            shared_by: shared_by.into(),
            download_label: download_label.into(),
            type_badge,
            icon: file_type_icon_for(&doc.filetype, &doc.filename),
            failed: doc.is_failed(),
        }
    }
}

pub struct FilesPopoverPanel {
    settings: Entity<Settings>,
    channel_id: ChannelId,
    clan_id: ClanId,
    search_input: Entity<InputState>,
    debounced_query: String,
    list_state: ListState,
    focus_handle: FocusHandle,
    rows: Vec<FileRowVm>,
    _debounce_task: Task<()>,
    _subs: Vec<gpui::Subscription>,
}

impl FilesPopoverPanel {
    pub fn new(
        settings: Entity<Settings>,
        clan_id: ClanId,
        channel_id: ChannelId,
        _popover_handle: PopoverMenuHandle<FilesPopoverPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let locale = settings.read(cx).language.clone();
        let placeholder = mezon_i18n::t(&locale, "channelTopbar.files.searchPlaceholder");
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .embedded(true)
        });

        let mut subs = vec![
            cx.observe(&FilesStore::global(cx), |this, _, cx| {
                this.refresh_rows(cx);
            }),
            cx.observe(&ClanMembersStore::global(cx), |this, _, cx| {
                FilesStore::global(cx).update(cx, |store, cx| {
                    store.refresh_uploaders(this.channel_id, cx);
                });
                this.refresh_rows(cx);
            }),
            cx.observe(&settings, |this, _, cx| {
                this.refresh_rows(cx);
            }),
        ];
        subs.push(
            cx.subscribe(&search_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.schedule_debounced_filter(cx);
                }
            }),
        );

        let list_state = ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)).measure_all();

        let mut this = Self {
            settings,
            channel_id,
            clan_id,
            search_input,
            debounced_query: String::new(),
            list_state,
            focus_handle,
            rows: Vec::new(),
            _debounce_task: Task::ready(()),
            _subs: subs,
        };
        this.refresh_rows(cx);
        this
    }

    fn schedule_debounced_filter(&mut self, cx: &mut Context<Self>) {
        self._debounce_task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(SEARCH_DEBOUNCE_MS))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.debounced_query = this.search_input.read(cx).value().to_string();
                this.refresh_rows(cx);
            });
        });
    }

    fn refresh_rows(&mut self, cx: &mut Context<Self>) {
        self.rows = Self::compute_rows(self.channel_id, &self.settings, &self.debounced_query, cx);
        cx.notify();
    }

    fn compute_rows(
        channel_id: ChannelId,
        settings: &Entity<Settings>,
        query: &str,
        cx: &App,
    ) -> Vec<FileRowVm> {
        let locale = settings.read(cx).language.clone();
        FilesStore::global(cx)
            .read(cx)
            .documents(channel_id)
            .iter()
            .filter(|doc| filename_matches_query(&doc.filename, query))
            .map(|doc| FileRowVm::from_doc(doc, &locale))
            .collect()
    }
}

impl Focusable for FilesPopoverPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DismissEvent> for FilesPopoverPanel {}

impl Render for FilesPopoverPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let locale = self.settings.read(cx).language.clone();
        let store = FilesStore::global(cx);
        let loading = store.read(cx).is_loading(self.channel_id);
        let fetch_error = store.read(cx).fetch_error(self.channel_id);
        let rows = Rc::new(self.rows.clone());
        let tokens = &theme.tokens;
        let clan_id = self.clan_id;
        let channel_id = self.channel_id;
        let search_input = self.search_input.clone();
        let query_active = !self.debounced_query.trim().is_empty();

        ClanMembersStore::global(cx).update(cx, |members, cx| {
            members.ensure_loaded(clan_id, cx);
        });

        let current = self.list_state.item_count();
        if rows.len() > current {
            self.list_state.splice(0..0, rows.len() - current);
        } else if rows.len() < current {
            self.list_state.reset(rows.len());
        }
        let list_state = self.list_state.clone();
        let viewport_h = f32::from(window.viewport_size().height);
        let panel_max_h = (viewport_h * MAX_VH).max(MIN_POPOVER_HEIGHT);

        v_flex()
            .key_context("menu")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|_, _: &::menu::Cancel, _window, cx| {
                cx.emit(DismissEvent);
            }))
            .on_mouse_down_out(cx.listener(|_, _: &MouseDownEvent, _window, cx| {
                cx.emit(DismissEvent);
            }))
            .w(px(POPOVER_WIDTH))
            .min_h(px(MIN_POPOVER_HEIGHT))
            .max_h(px(panel_max_h))
            .overflow_hidden()
            .rounded_md()
            .border_1()
            .border_color(tokens.border_primary)
            .bg(tokens.theme_setting_primary)
            .text_color(tokens.text_theme_message)
            .child(render_header(&theme, &locale, search_input, cx))
            .child(render_body(
                rows,
                loading,
                fetch_error,
                query_active,
                theme.clone(),
                locale,
                clan_id,
                channel_id,
                list_state,
                window,
                cx,
            ))
    }
}

fn render_header(
    theme: &Theme,
    locale: &str,
    search_input: Entity<InputState>,
    cx: &mut Context<FilesPopoverPanel>,
) -> impl IntoElement {
    let tokens = &theme.tokens;
    h_flex()
        .w_full()
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .gap_3()
        .px(px(16.))
        .h(px(HEADER_HEIGHT))
        .border_b_1()
        .border_color(tokens.border_theme_primary)
        .bg(tokens.theme_setting_nav)
        .child(
            h_flex()
                .items_center()
                .gap_4()
                .pr(px(16.))
                .flex_shrink_0()
                .border_r_1()
                .border_color(tokens.border_theme_primary)
                .child(
                    Icon::new(IconName::FileIcon)
                        .size(px(20.))
                        .text_color(tokens.bg_icon_theme),
                )
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(tokens.text_theme_message)
                        .child(mezon_i18n::t(locale, "channelTopbar.modals.files.title")),
                ),
        )
        .child(
            div()
                .relative()
                .w(px(SEARCH_WIDTH))
                .flex_shrink_0()
                .child(
                    h_flex()
                        .w_full()
                        .h(px(24.))
                        .pl_4()
                        .pr_2()
                        .rounded_md()
                        .bg(tokens.theme_input)
                        .items_center()
                        .child(
                            Input::new(&search_input)
                                .flex_1()
                                .text_sm()
                                .text_color(tokens.text_theme_primary),
                        ),
                )
                .child(
                    h_flex()
                        .absolute()
                        .right(px(4.))
                        .top_0()
                        .bottom_0()
                        .items_center()
                        .pl_1()
                        .child(
                            Icon::new(IconName::Search)
                                .size_4()
                                .text_color(tokens.text_theme_primary),
                        ),
                ),
        )
        .child(
            Button::new("files-close")
                .ghost()
                .with_size(Size::Small)
                .icon(
                    Icon::new(IconName::Close)
                        .size(px(16.))
                        .text_color(tokens.text_theme_primary),
                )
                .on_click(cx.listener(|_, _: &ClickEvent, _window, cx| {
                    cx.emit(DismissEvent);
                })),
        )
}

#[allow(clippy::too_many_arguments)]
fn render_body(
    rows: Rc<Vec<FileRowVm>>,
    loading: bool,
    fetch_error: bool,
    query_active: bool,
    theme: Arc<Theme>,
    locale: String,
    clan_id: ClanId,
    channel_id: ChannelId,
    list_state: ListState,
    window: &mut Window,
    cx: &mut Context<FilesPopoverPanel>,
) -> impl IntoElement {
    let tokens = &theme.tokens;

    let body: gpui::AnyElement = if rows.is_empty() {
        if loading && !query_active {
            div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .child(Spinner::new().with_size(Size::Small))
                .into_any_element()
        } else if fetch_error && !query_active {
            render_error_body(&locale, &theme, clan_id, channel_id).into_any_element()
        } else {
            render_empty_body(&locale, &theme).into_any_element()
        }
    } else {
        let rows_for_list = rows.clone();
        let theme_for_list = theme.clone();
        let locale_for_list = locale.clone();
        div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .when(fetch_error, |this| {
                this.child(
                    div()
                        .px(px(LIST_PAD_X))
                        .pt(px(8.))
                        .child(render_retry_banner(&locale, &theme, clan_id, channel_id)),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .pl(px(LIST_PAD_X))
                    .pr(px(LIST_PAD_X))
                    .py(px(8.))
                    .child(
                        list(list_state.clone(), move |ix, _window, _cx| {
                            let Some(vm) = rows_for_list.get(ix) else {
                                return div().into_any_element();
                            };
                            div()
                                .w_full()
                                .pb(px(8.))
                                .child(file_row(ix, vm, &theme_for_list, &locale_for_list))
                                .into_any_element()
                        })
                        .size_full(),
                    )
                    .custom_scrollbars(
                        Scrollbars::always_visible(ScrollAxes::Vertical)
                            .tracked_scroll_handle(&list_state),
                        window,
                        cx,
                    ),
            )
            .into_any_element()
    };

    div()
        .flex_1()
        .min_h_0()
        .w_full()
        .overflow_hidden()
        .bg(tokens.theme_setting_primary)
        .child(body)
}

fn render_empty_body(locale: &str, theme: &Theme) -> impl IntoElement {
    let tokens = &theme.tokens;
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .p(px(48.))
        .gap_2()
        .child(
            div()
                .relative()
                .mb_4()
                .p(px(22.))
                .rounded_full()
                .bg(tokens.bg_item_theme_hover)
                .child(
                    Icon::new(IconName::FileIcon)
                        .size(px(36.))
                        .text_color(tokens.text_theme_primary),
                )
                .child(
                    svg()
                        .path(IconName::EmptyUnreadStyle.path())
                        .absolute()
                        .top_0()
                        .left(px(-10.))
                        .w(px(104.))
                        .h(px(80.)),
                ),
        )
        .child(
            div()
                .text_size(px(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(tokens.text_theme_message)
                .child(mezon_i18n::t(locale, "channelTopbar.files.emptyTitle")),
        )
        .child(
            div()
                .text_base()
                .text_color(tokens.text_theme_primary)
                .text_center()
                .child(mezon_i18n::t(
                    locale,
                    "channelTopbar.files.emptyDescription",
                )),
        )
}

fn render_error_body(
    locale: &str,
    theme: &Theme,
    clan_id: ClanId,
    channel_id: ChannelId,
) -> impl IntoElement {
    let tokens = &theme.tokens;
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .p(px(24.))
        .child(
            div()
                .text_sm()
                .text_color(tokens.text_secondary)
                .child(mezon_i18n::t(locale, "common.failedToLoad")),
        )
        .child(
            Button::new("files-retry")
                .label(mezon_i18n::t(locale, "channelVoice.retry"))
                .ghost()
                .with_size(Size::Small)
                .on_click(move |_: &ClickEvent, _window, cx| {
                    FilesStore::global(cx).update(cx, |store, cx| {
                        store.refresh(clan_id, channel_id, cx);
                    });
                }),
        )
}

fn render_retry_banner(
    locale: &str,
    theme: &Theme,
    clan_id: ClanId,
    channel_id: ChannelId,
) -> impl IntoElement {
    let tokens = &theme.tokens;
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .px(px(8.))
        .py(px(8.))
        .mb(px(4.))
        .rounded_md()
        .bg(tokens.bg_item_theme_hover)
        .child(
            div()
                .text_xs()
                .text_color(tokens.text_secondary)
                .child(mezon_i18n::t(locale, "common.failedToLoad")),
        )
        .child(
            Button::new("files-retry-banner")
                .label(mezon_i18n::t(locale, "channelVoice.retry"))
                .ghost()
                .with_size(Size::XSmall)
                .on_click(move |_: &ClickEvent, _window, cx| {
                    FilesStore::global(cx).update(cx, |store, cx| {
                        store.refresh(clan_id, channel_id, cx);
                    });
                }),
        )
}

fn file_row(index: usize, vm: &FileRowVm, theme: &Theme, locale: &str) -> gpui::AnyElement {
    let tokens = &theme.tokens;
    let group_name = SharedString::from(format!("file-row-{index}"));

    if vm.failed {
        return h_flex()
            .id(("file-item", index))
            .w_full()
            .gap_3()
            .py(px(12.))
            .px(px(12.))
            .rounded_lg()
            .border_1()
            .border_color(tokens.border_theme_primary)
            .bg(tokens.theme_setting_nav)
            .child(
                div()
                    .flex_shrink_0()
                    .w(px(32.))
                    .h(px(40.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        svg()
                            .path(IconName::FileThumbEmpty.path())
                            .size(px(32.))
                            .text_color(tokens.text_theme_primary),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.status_dnd)
                    .child(mezon_i18n::t(
                        locale,
                        "channelTopbar.fileItem.attachmentFailed",
                    )),
            )
            .into_any_element();
    }

    let url = vm.url.clone();
    let filename = vm.filename.clone();
    let download_url = url.clone();
    let download_name = filename.clone();

    h_flex()
        .id(("file-item", index))
        .group(group_name.clone())
        .relative()
        .w_full()
        .gap_3()
        .py(px(12.))
        .pl(px(12.))
        .pr(px(12.))
        .rounded_lg()
        .border_1()
        .border_color(tokens.border_theme_primary)
        .bg(tokens.theme_setting_nav)
        .cursor_pointer()
        .on_click(move |_: &ClickEvent, _window, cx| {
            download_url_with_dialog(url.clone(), filename.clone(), cx);
        })
        .child(
            v_flex()
                .flex_shrink_0()
                .w(px(36.))
                .items_center()
                .gap_0p5()
                .child(
                    svg()
                        .path(vm.icon.path())
                        .w(px(32.))
                        .h(px(40.))
                        .text_color(tokens.text_theme_primary),
                )
                .child(
                    div()
                        .text_size(px(9.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(tokens.text_secondary)
                        .child(vm.type_badge.clone()),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .pr(px(40.))
                .gap_1()
                .child(
                    div()
                        .truncate()
                        .whitespace_nowrap()
                        .text_sm()
                        .text_color(theme.text_link)
                        .hover(|s| s.underline())
                        .child(vm.filename.clone()),
                )
                .child(
                    div()
                        .relative()
                        .w_full()
                        .h(px(16.))
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .truncate()
                                .whitespace_nowrap()
                                .text_xs()
                                .text_color(tokens.text_secondary)
                                .opacity(1.)
                                .group_hover(group_name.clone(), |s| s.opacity(0.))
                                .child(vm.shared_by.clone()),
                        )
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .truncate()
                                .whitespace_nowrap()
                                .text_xs()
                                .text_color(tokens.text_secondary)
                                .opacity(0.)
                                .group_hover(group_name.clone(), |s| s.opacity(1.))
                                .child(vm.download_label.clone()),
                        ),
                ),
        )
        .child(
            div()
                .absolute()
                .right(px(16.))
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .opacity(0.)
                .group_hover(group_name, |s| s.opacity(1.))
                .child(
                    div()
                        .id(("file-dl", index))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(32.))
                        .rounded_md()
                        .border_1()
                        .border_color(tokens.border_theme_primary)
                        .bg(tokens.theme_input)
                        .cursor_pointer()
                        .on_click(move |_: &ClickEvent, _window, cx| {
                            download_url_with_dialog(
                                download_url.clone(),
                                download_name.clone(),
                                cx,
                            );
                        })
                        .child(
                            Icon::new(IconName::Download)
                                .size(px(16.))
                                .text_color(tokens.text_theme_primary),
                        ),
                ),
        )
        .into_any_element()
}

fn format_file_time(create_time: i64, locale: &str) -> String {
    let Some(utc) = chrono::DateTime::from_timestamp(create_time, 0) else {
        return String::new();
    };
    let local = utc.with_timezone(&chrono::Local);
    let date = local.date_naive();
    let today = chrono::Local::now().date_naive();
    let time = local.format("%H:%M").to_string();

    if date == today {
        format!("{} {}", mezon_i18n::t(locale, "common.todayAt"), time)
    } else if Some(date) == today.pred_opt() {
        format!("{} {}", mezon_i18n::t(locale, "common.yesterdayAt"), time)
    } else {
        local.format("%d/%m/%Y, %H:%M").to_string()
    }
}

pub fn active_files_channel(cx: &App) -> Option<(ClanId, ChannelId)> {
    match Router::global(cx).read(cx).route() {
        Route::Channel {
            clan_id,
            channel_id,
        }
        | Route::Thread {
            clan_id,
            channel_id,
            ..
        }
        | Route::Canvas {
            clan_id,
            channel_id,
            ..
        } => {
            if clan_id.0 == 0 {
                None
            } else {
                Some((clan_id, channel_id))
            }
        }
        _ => None,
    }
}

pub fn files_popover_on_open() -> Rc<dyn Fn(&mut Window, &mut App)> {
    Rc::new(|_window, cx| {
        let Some((clan_id, channel_id)) = active_files_channel(cx) else {
            return;
        };
        FilesStore::global(cx).update(cx, |store, cx| {
            store.refresh(clan_id, channel_id, cx);
        });
        ClanMembersStore::global(cx).update(cx, |members, cx| {
            members.ensure_loaded(clan_id, cx);
        });
    })
}
