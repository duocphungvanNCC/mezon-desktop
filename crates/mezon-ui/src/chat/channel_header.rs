use gpui::{AnyElement, Entity, deferred, div, prelude::*, px};

use crate::chat::layout::ChatLayout;
use crate::components::primitives::{Icon, IconName};
use crate::theme::Theme;

pub struct ChannelHeader {
    name: String,
    dm: bool,
    layout: Option<Entity<ChatLayout>>,
    pin_popover: Option<AnyElement>,
}

impl ChannelHeader {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            dm: false,
            layout: None,
            pin_popover: None,
        }
    }

    pub fn dm(mut self, dm: bool) -> Self {
        self.dm = dm;
        self
    }

    /// The owning `ChatLayout`, used to toggle the pinned-messages popover from the pin icon.
    pub fn layout(mut self, layout: Entity<ChatLayout>) -> Self {
        self.layout = Some(layout);
        self
    }

    /// The pinned-messages panel to anchor under the pin icon when open.
    pub fn pin_popover(mut self, popover: Option<AnyElement>) -> Self {
        self.pin_popover = popover;
        self
    }

    pub fn render(mut self, theme: &Theme) -> impl IntoElement {
        let bg_hover = theme.bg_hover;
        let icon_color = theme.text_muted;
        let layout = self.layout.clone();
        let mut pin_popover = self.pin_popover.take();
        let actions = [
            ("hdr-canvas", IconName::CanvasIcon),
            ("hdr-timeline", IconName::History),
            ("hdr-thread", IconName::ThreadIcon),
            ("hdr-members", IconName::MemberList),
            ("hdr-pin", IconName::PinRight),
            ("hdr-bell", IconName::Bell),
            ("hdr-gallery", IconName::ImageThumbnail),
            ("hdr-files", IconName::FileIcon),
            ("hdr-inbox", IconName::Inbox),
        ];

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .h(px(50.))
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.bg_primary)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .when(!self.dm, |this| {
                        this.child(
                            Icon::new(IconName::Hashtag)
                                .size(px(20.0))
                                .text_color(theme.text_muted),
                        )
                    })
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.text_primary)
                            .child(self.name.clone()),
                    ),
            )
            .child(div().flex_1())
            .child(div().flex().flex_row().items_center().gap_1().children(
                actions.into_iter().map(move |(id, icon)| {
                    let mut button = div()
                        .id(id)
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(32.))
                        .h(px(32.))
                        .rounded_md()
                        .cursor_pointer()
                        .hover(move |s| s.bg(bg_hover))
                        .child(Icon::new(icon).size(px(20.)).text_color(icon_color));

                    if id == "hdr-pin" {
                        button = button.relative();
                        if let Some(layout) = layout.clone() {
                            button = button.on_click(move |_, _window, cx| {
                                layout.update(cx, |layout, cx| layout.toggle_pin_popover(cx));
                            });
                        }
                        if let Some(popover) = pin_popover.take() {
                            button = button.child(deferred(
                                div()
                                    .absolute()
                                    .top_full()
                                    .right_0()
                                    .mt(px(9.))
                                    .child(popover),
                            ));
                        }
                    }

                    button
                }),
            ))
    }
}
