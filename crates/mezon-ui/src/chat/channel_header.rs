use std::sync::Arc;

use gpui::{App, Window, div, prelude::*, px};

use crate::components::primitives::{Icon, IconName};
use crate::theme::Theme;

type ToggleHandler = Arc<dyn Fn(&mut Window, &mut App)>;

pub struct ChannelHeader {
    name: String,
    dm: bool,
    members_action: bool,
    members_active: bool,
    on_toggle_members: Option<ToggleHandler>,
}

impl ChannelHeader {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            dm: false,
            members_action: true,
            members_active: false,
            on_toggle_members: None,
        }
    }

    pub fn dm(mut self, dm: bool) -> Self {
        self.dm = dm;
        self
    }

    pub fn members_action(mut self, show: bool) -> Self {
        self.members_action = show;
        self
    }

    pub fn members_active(mut self, active: bool) -> Self {
        self.members_active = active;
        self
    }

    pub fn on_toggle_members(mut self, handler: ToggleHandler) -> Self {
        self.on_toggle_members = Some(handler);
        self
    }

    pub fn render(&self, theme: &Theme) -> impl IntoElement {
        let bg_hover = theme.bg_hover;
        let bg_active = theme.bg_tertiary;
        let icon_color = theme.text_muted;
        let icon_active = theme.text_primary;
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
            .child(
                div().flex().flex_row().items_center().gap_1().children(
                    actions
                        .into_iter()
                        .filter(move |(id, _)| *id != "hdr-members" || self.members_action)
                        .map(move |(id, icon)| {
                            let is_members = id == "hdr-members";
                            let active = is_members && self.members_active;
                            let tint = if active { icon_active } else { icon_color };
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
                                .child(Icon::new(icon).size(px(20.)).text_color(tint));
                            if active {
                                button = button.bg(bg_active);
                            }
                            if is_members && let Some(handler) = self.on_toggle_members.clone() {
                                button = button.on_click(move |_, window, cx| handler(window, cx));
                            }
                            button
                        }),
                ),
            )
    }
}
