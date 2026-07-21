use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, FontWeight, Hsla, ObjectFit, PathPromptOptions,
    ScrollHandle, SharedString, Subscription, Task, WeakEntity, Window, div, img, prelude::*, px,
    rgb,
};

use crate::app::shell::Shell;
use crate::components::primitives::{
    Icon, IconName, Input, InputEvent, InputState, Spinner, h_flex, v_flex,
};
use crate::image_cache::LruImageCache;
use crate::theme::{ActiveTheme, Theme};
use mezon_store::{
    AppConfig, ChannelId, ChannelMediaStore, ChannelPermissionsStore, ChannelTimeline,
    ChannelTimelineAttachment, ClanId, Settings, UpdateTimelineInput,
};

use super::media_image::{image_placeholder, render_media_image_cover};
use super::media_image_modal::open_media_image_modal;
use ui::{ScrollAxes, Scrollbars, WithScrollbar};

pub struct EventDetailView {
    focus_handle: FocusHandle,
    clan_id: ClanId,
    channel_id: ChannelId,
    event_id: i64,
    start_time_seconds: u32,
    settings: Entity<Settings>,
    scroll: ScrollHandle,
    edit_title: bool,
    edit_description: bool,
    title_input: Entity<InputState>,
    description_input: Entity<InputState>,
    saving: bool,
    uploading: bool,
    on_back: Arc<dyn Fn(&mut Window, &mut App)>,
    _store_sub: Subscription,
    _title_sub: Subscription,
    _description_sub: Subscription,
    _save_task: Option<Task<()>>,
    last_detail_error: Option<String>,
    _permissions_sub: Subscription,
    preview_image_cache: Entity<LruImageCache>,
}

impl Focusable for EventDetailView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventDetailView {
    pub fn new(
        clan_id: ClanId,
        channel_id: ChannelId,
        event_id: i64,
        start_time_seconds: u32,
        settings: Entity<Settings>,
        preview_image_cache: Entity<LruImageCache>,
        on_back: Arc<dyn Fn(&mut Window, &mut App)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        ChannelMediaStore::global(cx).update(cx, |store, cx| {
            store.fetch_detail(clan_id, channel_id, event_id, start_time_seconds, false, cx);
        });
        ChannelPermissionsStore::global(cx).update(cx, |store, cx| {
            store.ensure_loaded(clan_id, channel_id, cx);
        });
        let store = ChannelMediaStore::global(cx);
        let _store_sub = cx.observe(&store, |this, store, cx| {
            let err = store
                .read(cx)
                .detail_error(this.event_id)
                .map(str::to_string);
            if err != this.last_detail_error {
                if let Some(message) = err.clone() {
                    Shell::global(cx).update(cx, |shell, cx| shell.error(message, cx));
                }
                this.last_detail_error = err;
            }
            cx.notify();
        });
        let _permissions_sub =
            cx.observe(&ChannelPermissionsStore::global(cx), |_, _, cx| cx.notify());

        let title_input = cx.new(|cx| InputState::new(window, cx).height(px(32.)));
        let description_input =
            cx.new(|cx| InputState::new(window, cx).multi_line(true).height(px(80.)));
        let _title_sub = cx.subscribe(&title_input, |this, _, event, cx| {
            if matches!(event, InputEvent::PressEnter) && this.edit_title && !this.saving {
                this.save_title(cx);
            }
        });
        let _description_sub = cx.subscribe(&description_input, |_, _, _, cx| cx.notify());

        Self {
            focus_handle: cx.focus_handle(),
            clan_id,
            channel_id,
            event_id,
            start_time_seconds,
            settings,
            scroll: ScrollHandle::new(),
            edit_title: false,
            edit_description: false,
            title_input,
            description_input,
            saving: false,
            uploading: false,
            on_back,
            _store_sub,
            _title_sub,
            _description_sub,
            _save_task: None,
            last_detail_error: None,
            _permissions_sub,
            preview_image_cache,
        }
    }

    fn can_edit_events(&self, cx: &App) -> bool {
        ChannelPermissionsStore::global(cx).read(cx).has_permission(
            "send-message",
            self.clan_id,
            self.channel_id,
        )
    }

    fn open_gallery(&self, attachment_index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(event) = self.event(cx) else {
            return;
        };
        let attachments: Vec<_> = event
            .media_attachments()
            .into_iter()
            .filter(|attachment| attachment.is_uploaded())
            .collect();
        if attachment_index >= attachments.len() {
            return;
        }
        open_media_image_modal(attachments, attachment_index, window, cx);
    }

    fn event(&self, cx: &App) -> Option<ChannelTimeline> {
        ChannelMediaStore::global(cx)
            .read(cx)
            .event_detail(self.event_id)
            .cloned()
    }

    fn is_loading(&self, cx: &App) -> bool {
        self.event(cx).is_none()
            && ChannelMediaStore::global(cx)
                .read(cx)
                .is_detail_loading(self.event_id)
    }

    fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(event) = self.event(cx) {
            if !self.edit_title {
                self.title_input.update(cx, |input, cx| {
                    input.set_value(&event.title, window, cx);
                });
            }
            if !self.edit_description {
                self.description_input.update(cx, |input, cx| {
                    input.set_value(&event.description, window, cx);
                });
            }
        }
    }

    fn begin_edit_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_inputs(window, cx);
        self.edit_title = true;
        cx.notify();
        self.title_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    fn begin_edit_description(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_inputs(window, cx);
        self.edit_description = true;
        cx.notify();
        self.description_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    fn cancel_title_edit(&mut self, cx: &mut Context<Self>) {
        self.edit_title = false;
        cx.notify();
    }

    fn cancel_description_edit(&mut self, cx: &mut Context<Self>) {
        self.edit_description = false;
        cx.notify();
    }

    fn cancel_edits(&mut self, cx: &mut Context<Self>) {
        if self.edit_title {
            self.cancel_title_edit(cx);
        }
        if self.edit_description {
            self.cancel_description_edit(cx);
        }
    }

    fn save_edits(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if self.edit_title {
            self.save_title(cx);
        }
        if self.edit_description && !self.saving {
            self.save_description(cx);
        }
    }

    fn save_title(&mut self, cx: &mut Context<Self>) {
        let title = self.title_input.read(cx).value().trim().to_string();
        if title.is_empty() {
            self.cancel_title_edit(cx);
            return;
        }
        let existing = self.event(cx).map(|e| e.title.clone()).unwrap_or_default();
        if title == existing {
            self.edit_title = false;
            cx.notify();
            return;
        }
        self.run_update(
            UpdateTimelineInput {
                id: self.event_id,
                title: Some(title),
                description: None,
                start_time_seconds: self.start_time_seconds,
                attachments: None,
            },
            cx,
            move |this, cx| {
                this.edit_title = false;
                cx.notify();
            },
        );
    }

    fn save_description(&mut self, cx: &mut Context<Self>) {
        let description = self.description_input.read(cx).value().trim().to_string();
        let existing = self
            .event(cx)
            .map(|e| e.description.clone())
            .unwrap_or_default();
        if description == existing {
            self.edit_description = false;
            cx.notify();
            return;
        }
        self.run_update(
            UpdateTimelineInput {
                id: self.event_id,
                title: None,
                description: Some(description),
                start_time_seconds: self.start_time_seconds,
                attachments: None,
            },
            cx,
            move |this, cx| {
                this.edit_description = false;
                cx.notify();
            },
        );
    }

    fn run_update(
        &mut self,
        input: UpdateTimelineInput,
        cx: &mut Context<Self>,
        on_success: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        if self.saving {
            return;
        }
        self.saving = true;
        cx.notify();
        let clan_id = self.clan_id;
        let channel_id = self.channel_id;
        let task = ChannelMediaStore::global(cx).update(cx, |store, cx| {
            store.update_timeline(clan_id, channel_id, input, cx)
        });
        self._save_task = Some(cx.spawn(async move |this, cx| match task.await {
            Ok(_) => {
                this.update(cx, |this, cx| {
                    this.saving = false;
                    on_success(this, cx);
                })
                .ok();
            }
            Err(e) => {
                this.update(cx, |this, cx| {
                    this.saving = false;
                    cx.notify();
                })
                .ok();
                cx.update(|cx| {
                    Shell::global(cx).update(cx, |shell, cx| shell.error(e, cx));
                });
            }
        }));
    }

    fn pick_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.uploading || self.saving {
            return;
        }
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let media_paths: Vec<PathBuf> = paths
                .into_iter()
                .filter(|path| is_allowed_media(path))
                .collect();
            if media_paths.is_empty() {
                return;
            }
            this.update(cx, |this, cx| {
                this.uploading = true;
                cx.notify();
            })
            .ok();

            let mut uploaded = Vec::new();
            for path in media_paths {
                let upload_task = this
                    .update(cx, |_, cx| {
                        ChannelMediaStore::global(cx)
                            .update(cx, |store, cx| store.upload_attachment(&path, cx))
                    })
                    .ok();
                if let Some(task) = upload_task {
                    if let Ok(att) = task.await {
                        uploaded.push(att);
                    }
                }
            }

            if uploaded.is_empty() {
                this.update(cx, |this, cx| {
                    this.uploading = false;
                    cx.notify();
                })
                .ok();
                return;
            }

            let update_task = this.update(cx, |this, cx| {
                this.uploading = false;
                let input = UpdateTimelineInput {
                    id: this.event_id,
                    title: None,
                    description: None,
                    start_time_seconds: this.start_time_seconds,
                    attachments: Some(uploaded),
                };
                ChannelMediaStore::global(cx).update(cx, |store, cx| {
                    store.update_timeline(this.clan_id, this.channel_id, input, cx)
                })
            });
            if let Ok(task) = update_task {
                if let Err(message) = task.await {
                    this.update(cx, |_, cx| {
                        Shell::global(cx).update(cx, |shell, cx| shell.error(message, cx));
                    })
                    .ok();
                }
            }
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
    }
}

impl Render for EventDetailView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_inputs(window, cx);
        let theme = cx.theme().clone();
        let locale = self.settings.read(cx).language.clone();
        let config = AppConfig::global(cx).clone();
        let loading = self.is_loading(cx);
        let event = self.event(cx);
        let can_edit = self.can_edit_events(cx);
        let on_back = self.on_back.clone();
        let edit_title = self.edit_title;
        let date_label = format_event_date_label(event.as_ref(), self.start_time_seconds, &locale);
        let title_text = event
            .as_ref()
            .and_then(|e| {
                if e.title.is_empty() {
                    None
                } else {
                    Some(e.title.clone())
                }
            })
            .unwrap_or_else(|| {
                mezon_i18n::t(&locale, "channelCreator.fields.eventDetail.defaultTitle").to_string()
            });

        let header = h_flex()
            .px_6()
            .py_4()
            .border_b_1()
            .border_color(theme.border)
            .gap_3()
            .items_center()
            .child(
                div()
                    .id("event-detail-back")
                    .cursor_pointer()
                    .rounded_lg()
                    .p_1()
                    .hover(|el| el.bg(theme.bg_hover))
                    .on_click(move |_, window, cx| on_back(window, cx))
                    .child(
                        Icon::new(IconName::ArrowLeft)
                            .size(px(20.))
                            .text_color(theme.text_primary),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .when(loading, |col| {
                        col.child(
                            div()
                                .h(px(24.))
                                .w(px(192.))
                                .rounded_md()
                                .bg(theme.bg_tertiary),
                        )
                    })
                    .when(!loading, |col| {
                        if self.edit_title {
                            col.child(Input::new(&self.title_input))
                        } else if can_edit {
                            col.child(
                                div()
                                    .id("event-detail-title")
                                    .cursor_pointer()
                                    .hover(|el| el.opacity(0.85))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.begin_edit_title(window, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_lg()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.text_primary)
                                            .hover(|el| el.text_color(theme.brand))
                                            .truncate()
                                            .child(title_text),
                                    ),
                            )
                        } else {
                            col.child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.text_primary)
                                    .truncate()
                                    .child(title_text),
                            )
                        }
                        .when(!date_label.is_empty(), |col| {
                            col.child(
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(
                                        Icon::new(IconName::History)
                                            .size(px(14.))
                                            .text_color(theme.text_secondary),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.text_secondary)
                                            .child(date_label.clone()),
                                    ),
                            )
                        })
                    }),
            )
            .when(!loading && can_edit, |row| {
                row.child(
                    div()
                        .id("event-detail-upload")
                        .cursor_pointer()
                        .rounded_lg()
                        .p_2()
                        .when(self.uploading, |el| el.opacity(0.5))
                        .when(!self.uploading, |el| el.hover(|el| el.bg(theme.bg_hover)))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.pick_files(window, cx);
                        }))
                        .child(
                            Icon::new(IconName::ImageThumbnail)
                                .size(px(20.))
                                .text_color(theme.text_primary),
                        ),
                )
            })
            .into_any_element();

        let scroll_child = if loading {
            div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .child(Spinner::new())
                .into_any_element()
        } else if let Some(event) = event {
            render_detail_body(
                &theme,
                &locale,
                &event,
                &config,
                can_edit,
                self.edit_description,
                edit_title,
                &self.description_input,
                self.saving,
                self.preview_image_cache.clone(),
                cx,
            )
            .into_any_element()
        } else {
            div().into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.bg_primary)
            .child(header)
            .child(
                div()
                    .id("event-detail-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("event-detail-scroll-inner")
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .size_full()
                            .child(
                                div()
                                    .w_full()
                                    .flex_none()
                                    .image_cache(self.preview_image_cache.clone())
                                    .child(scroll_child),
                            ),
                    )
                    .custom_scrollbars(
                        Scrollbars::always_visible(ScrollAxes::Vertical)
                            .tracked_scroll_handle(&self.scroll),
                        window,
                        cx,
                    ),
            )
    }
}

fn render_detail_body(
    theme: &Theme,
    locale: &str,
    event: &ChannelTimeline,
    config: &AppConfig,
    can_edit: bool,
    edit_description: bool,
    edit_title: bool,
    description_input: &Entity<InputState>,
    saving: bool,
    preview_image_cache: Entity<LruImageCache>,
    cx: &mut Context<EventDetailView>,
) -> impl IntoElement {
    let attachments = event.media_attachments();
    let featured = attachments.first().cloned();
    let grid: Vec<(usize, ChannelTimelineAttachment)> =
        attachments.into_iter().enumerate().skip(1).collect();
    let view = cx.weak_entity();

    v_flex()
        .px_6()
        .py_4()
        .gap_4()
        .flex_none()
        .w_full()
        .child(if edit_description {
            Input::new(description_input).into_any_element()
        } else if !event.description.is_empty() {
            div()
                .id("event-detail-description")
                .w_full()
                .p_3()
                .rounded_lg()
                .bg(theme.bg_primary)
                .border_1()
                .border_color(theme.border)
                .when(can_edit, |el| {
                    el.cursor_pointer().hover(|el| el.bg(theme.bg_hover))
                })
                .when(can_edit, |el| {
                    el.on_click(cx.listener(|this, _, window, cx| {
                        this.begin_edit_description(window, cx);
                    }))
                })
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.text_secondary)
                        .child(event.description.clone()),
                )
                .into_any_element()
        } else if can_edit {
            div()
                .id("event-detail-add-description")
                .w_full()
                .p_3()
                .border_2()
                .border_dashed()
                .border_color(theme.border)
                .rounded_lg()
                .flex()
                .items_center()
                .justify_center()
                .gap_2()
                .cursor_pointer()
                .text_color(theme.text_secondary)
                .hover(|el| {
                    el.border_color(Hsla::from(theme.brand).opacity(0.5))
                        .text_color(theme.brand)
                })
                .on_click(cx.listener(|this, _, window, cx| {
                    this.begin_edit_description(window, cx);
                }))
                .child(
                    Icon::new(IconName::PenEdit)
                        .size(px(16.))
                        .text_color(theme.text_secondary),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.text_secondary)
                        .child(mezon_i18n::t(
                            locale,
                            "channelCreator.fields.eventDetail.addDescription",
                        )),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        })
        .when((edit_title || edit_description) && can_edit, |col| {
            let primary = theme.tokens.bg_button_primary;
            let cancel_hover_bg = Hsla::from(theme.status_dnd).opacity(0.1);
            col.child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        div()
                            .id("event-detail-cancel")
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .px_4()
                            .py_2()
                            .rounded_lg()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text_secondary)
                            .when(!saving, |el| {
                                el.cursor_pointer()
                                    .hover(|el| el.text_color(theme.status_dnd).bg(cancel_hover_bg))
                            })
                            .when(saving, |el| el.opacity(0.5))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.saving {
                                    return;
                                }
                                this.cancel_edits(cx);
                            }))
                            .child(
                                Icon::new(IconName::CloseIcon)
                                    .size(px(16.))
                                    .text_color(theme.text_secondary),
                            )
                            .child(mezon_i18n::t(
                                locale,
                                "channelCreator.fields.eventDetail.cancel",
                            )),
                    )
                    .child(
                        div()
                            .id("event-detail-save")
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .px_4()
                            .py_2()
                            .rounded_lg()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(0xffffff))
                            .bg(primary)
                            .when(saving, |el| el.opacity(0.5))
                            .when(!saving, |el| {
                                el.cursor_pointer()
                                    .hover(|el| el.bg(theme.tokens.bg_button_primary_hover))
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.saving {
                                    return;
                                }
                                this.save_edits(cx);
                            }))
                            .child(if saving {
                                Spinner::new().into_any_element()
                            } else {
                                Icon::new(IconName::CheckIcon)
                                    .size(px(16.))
                                    .text_color(rgb(0xffffff))
                                    .into_any_element()
                            })
                            .child(mezon_i18n::t(
                                locale,
                                "channelCreator.fields.eventDetail.save",
                            )),
                    ),
            )
        })
        .when_some(featured, |col, att| {
            col.child(render_featured_image(
                theme,
                locale,
                0,
                &att,
                config,
                preview_image_cache,
                view.clone(),
            ))
        })
        .child(render_media_grid(theme, &grid, config, can_edit, view, cx))
}

const MEDIA_GRID_COLS: usize = 4;

fn render_media_grid(
    theme: &Theme,
    grid: &[(usize, ChannelTimelineAttachment)],
    config: &AppConfig,
    can_edit: bool,
    view: WeakEntity<EventDetailView>,
    cx: &mut Context<EventDetailView>,
) -> impl IntoElement {
    let mut cells: Vec<gpui::AnyElement> = grid
        .iter()
        .map(|(index, att)| {
            render_grid_image(theme, *index, att, config, view.clone()).into_any_element()
        })
        .collect();
    if can_edit {
        cells.push(render_add_media_tile(theme, cx).into_any_element());
    }

    let mut rows: Vec<gpui::AnyElement> = Vec::new();
    while !cells.is_empty() {
        let take = MEDIA_GRID_COLS.min(cells.len());
        let row: Vec<gpui::AnyElement> = cells.drain(..take).collect();
        let filler_count = MEDIA_GRID_COLS - row.len();
        rows.push(
            h_flex()
                .gap_3()
                .w_full()
                .flex_none()
                .children(row)
                .children((0..filler_count).map(|_| div().flex_1().min_w_0().into_any_element()))
                .into_any_element(),
        );
    }

    v_flex().gap_3().w_full().flex_none().children(rows)
}

fn render_add_media_tile(theme: &Theme, cx: &mut Context<EventDetailView>) -> impl IntoElement {
    let hover_border = Hsla::from(theme.brand).opacity(0.5);
    div().flex_1().min_w_0().child(
        div()
            .id("event-detail-add-media")
            .w_full()
            .aspect_square()
            .rounded_xl()
            .border_2()
            .border_dashed()
            .border_color(theme.border)
            .bg(theme.bg_primary)
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(theme.text_secondary)
            .hover(|el| el.border_color(hover_border).text_color(theme.brand))
            .on_click(cx.listener(|this, _, window, cx| {
                this.pick_files(window, cx);
            }))
            .child(
                Icon::new(IconName::PlusIcon)
                    .size(px(32.))
                    .text_color(theme.text_secondary),
            ),
    )
}

fn render_featured_image(
    theme: &Theme,
    locale: &str,
    attachment_index: usize,
    att: &ChannelTimelineAttachment,
    config: &AppConfig,
    preview_image_cache: Entity<LruImageCache>,
    view: WeakEntity<EventDetailView>,
) -> impl IntoElement {
    let url = featured_proxy_url(att, config);
    let featured_h = px(256.);
    div()
        .id(SharedString::from(format!(
            "event-featured-{attachment_index}"
        )))
        .relative()
        .w_full()
        .h(featured_h)
        .flex_none()
        .rounded_xl()
        .overflow_hidden()
        .when(att.is_uploaded(), |el| {
            el.cursor_pointer()
                .hover(|el| el.opacity(0.92))
                .on_click(move |_, window, cx| {
                    view.update(cx, |this, cx| {
                        this.open_gallery(attachment_index, window, cx)
                    })
                    .ok();
                })
        })
        .child(render_media_image_cover(
            theme.bg_tertiary,
            theme.text_muted,
            url,
            featured_h,
            preview_image_cache,
        ))
        .child(
            div()
                .absolute()
                .top_3()
                .left_3()
                .px_2()
                .py_1()
                .rounded_md()
                .bg(theme.tokens.bg_button_primary)
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0xffffff))
                .child(mezon_i18n::t(
                    locale,
                    "channelCreator.fields.eventDetail.featured",
                )),
        )
        .when(!att.is_uploaded(), |el| {
            el.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui::hsla(0., 0., 0., 0.4))
                    .child(Spinner::new()),
            )
        })
}

fn render_grid_image(
    theme: &Theme,
    attachment_index: usize,
    att: &ChannelTimelineAttachment,
    config: &AppConfig,
    view: WeakEntity<EventDetailView>,
) -> impl IntoElement {
    let url = proxy_url(att, config);
    let bg = theme.bg_tertiary;
    let muted = theme.text_muted;
    div().flex_1().min_w_0().child(
        div()
            .id(SharedString::from(format!("event-grid-{attachment_index}")))
            .relative()
            .w_full()
            .aspect_square()
            .rounded_xl()
            .overflow_hidden()
            .when(att.is_uploaded(), |el| {
                el.cursor_pointer()
                    .hover(|el| el.opacity(0.92))
                    .on_click(move |_, window, cx| {
                        view.update(cx, |this, cx| {
                            this.open_gallery(attachment_index, window, cx)
                        })
                        .ok();
                    })
            })
            .child(
                img(url)
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .with_loading(move || image_placeholder(bg, muted, px(300.), px(300.)))
                    .with_fallback(move || image_placeholder(bg, muted, px(300.), px(300.))),
            )
            .when(!att.is_uploaded(), |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(gpui::hsla(0., 0., 0., 0.4))
                        .child(Spinner::new()),
                )
            }),
    )
}

fn proxy_url(att: &ChannelTimelineAttachment, config: &AppConfig) -> SharedString {
    let url = att.display_url();
    if att.is_uploaded() && url.starts_with("http") {
        config.event_detail_grid_proxy(url).into()
    } else {
        url.into()
    }
}

fn featured_proxy_url(att: &ChannelTimelineAttachment, config: &AppConfig) -> SharedString {
    let url = att.display_url();
    if att.is_uploaded() && url.starts_with("http") {
        config.event_detail_featured_proxy(url).into()
    } else {
        url.into()
    }
}

fn format_event_date_label(
    event: Option<&ChannelTimeline>,
    start_time_seconds: u32,
    locale: &str,
) -> String {
    let seconds = event
        .map(|e| e.start_time_seconds)
        .filter(|s| *s > 0)
        .unwrap_or(start_time_seconds);
    if seconds == 0 {
        return String::new();
    }
    super::timeline_view::format_event_date_label(seconds, locale)
}

fn is_allowed_media(path: &PathBuf) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "mp4" | "webm" | "mov"
    )
}
