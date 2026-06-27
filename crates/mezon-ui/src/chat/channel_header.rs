use std::sync::Arc;

use gpui::{
    App, Context, Entity, Render, SharedString, Subscription, WeakEntity, Window, div, prelude::*,
    px,
};
use mezon_store::Settings;

use crate::app::window_controls;
use crate::components::primitives::{Icon, IconName};
use crate::theme::{ActiveTheme, Theme};

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
            .w_full()
            .h(px(window_controls::APP_HEADER_HEIGHT))
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
                                .occlude()
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

pub struct ChatHeader {
    name: SharedString,
    dm: bool,
    members_action: bool,
    members_active: bool,
    layout: WeakEntity<crate::ChatLayout>,
    _settings_observe: Subscription,
}

impl ChatHeader {
    pub fn new(
        layout: WeakEntity<crate::ChatLayout>,
        settings: &Entity<Settings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let _settings_observe = cx.observe(settings, |_, _, cx| cx.notify());
        Self {
            name: SharedString::default(),
            dm: false,
            members_action: true,
            members_active: false,
            layout,
            _settings_observe,
        }
    }

    pub fn sync(
        &mut self,
        name: Option<SharedString>,
        dm: bool,
        members_action: bool,
        members_active: bool,
        cx: &mut Context<Self>,
    ) {
        let name = name.unwrap_or_else(|| self.name.clone());
        if self.name == name
            && self.dm == dm
            && self.members_action == members_action
            && self.members_active == members_active
        {
            return;
        }
        self.name = name;
        self.dm = dm;
        self.members_action = members_action;
        self.members_active = members_active;
        cx.notify();
    }
}

impl Render for ChatHeader {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let layout = self.layout.clone();
        ChannelHeader::new(self.name.to_string())
            .dm(self.dm)
            .members_action(self.members_action)
            .members_active(self.members_active)
            .on_toggle_members(Arc::new(move |_window: &mut Window, cx: &mut App| {
                let _ = layout.update(cx, |this, cx| this.toggle_member_list(cx));
            }))
            .render(theme)
            .into_any_element()
    }
}
