use gpui::{
    Anchor, App, ClickEvent, CursorStyle, Entity, Hsla, IntoElement, RenderOnce, Window, div,
    point, prelude::*, px,
};
use mezon_store::Settings;
use ui::prelude::*;
use ui::{PopoverMenu, PopoverMenuHandle};

use crate::chat::pinned_popover::{PinnedPopoverPanel, pin_popover_on_open};
use crate::components::primitives::{
    Button, ButtonVariant, ButtonVariants, Icon, IconName, Sizable, Size,
};
use crate::theme::Theme;

pub struct ChannelHeader {
    name: String,
    dm: bool,
    pin_handle: Option<PopoverMenuHandle<PinnedPopoverPanel>>,
    settings: Option<Entity<Settings>>,
}

impl ChannelHeader {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            dm: false,
            pin_handle: None,
            settings: None,
        }
    }

    pub fn dm(mut self, dm: bool) -> Self {
        self.dm = dm;
        self
    }

    pub fn pin_popover(
        mut self,
        handle: PopoverMenuHandle<PinnedPopoverPanel>,
        settings: Entity<Settings>,
    ) -> Self {
        self.pin_handle = Some(handle);
        self.settings = Some(settings);
        self
    }

    pub fn render(
        self,
        theme: &Theme,
        _window: &mut Window,
        _cx: &mut App,
    ) -> impl IntoElement {
        let bg_hover = theme.bg_hover;
        let icon_color = theme.text_muted;
        let pin_handle = self.pin_handle;
        let settings = self.settings;
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
                    if id == "hdr-pin"
                        && let (Some(handle), Some(settings)) =
                            (pin_handle.clone(), settings.clone())
                    {
                        let menu_handle = handle.clone();
                        return PopoverMenu::new("hdr-pin-popover")
                            .with_handle(handle)
                            .anchor(Anchor::TopRight)
                            .attach(Anchor::BottomRight)
                            .offset(point(px(0.), px(9.)))
                            .on_open(pin_popover_on_open())
                            .menu({
                                let settings = settings.clone();
                                move |window, cx| {
                                    Some(cx.new(|cx| {
                                        PinnedPopoverPanel::new(
                                            settings.clone(),
                                            menu_handle.clone(),
                                            window,
                                            cx,
                                        )
                                    }))
                                }
                            })
                            .trigger(PinPopoverTrigger::new(theme, false))
                            .into_any_element();
                    }

                    div()
                        .id(id)
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(32.))
                        .h(px(32.))
                        .rounded_md()
                        .cursor_pointer()
                        .hover(move |s| s.bg(bg_hover))
                        .child(Icon::new(icon).size(px(20.)).text_color(icon_color))
                        .into_any_element()
                }),
            ))
    }
}

#[derive(IntoElement)]
struct PinPopoverTrigger {
    open: bool,
    icon_color: Hsla,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

impl PinPopoverTrigger {
    fn new(theme: &Theme, open: bool) -> Self {
        Self {
            open,
            icon_color: theme.text_muted.into(),
            on_click: None,
        }
    }
}

impl Toggleable for PinPopoverTrigger {
    fn toggle_state(mut self, selected: bool) -> Self {
        self.open = selected;
        self
    }
}

impl Clickable for PinPopoverTrigger {
    fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    fn cursor_style(self, _cursor_style: CursorStyle) -> Self {
        self
    }
}

impl RenderOnce for PinPopoverTrigger {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut button = Button::new("hdr-pin-trigger")
            .with_size(Size::Small)
            .icon(
                Icon::new(IconName::PinRight)
                    .size(px(20.))
                    .text_color(self.icon_color),
            );
        button = if self.open {
            button.with_variant(ButtonVariant::Secondary)
        } else {
            button.ghost()
        };
        if let Some(handler) = self.on_click {
            button.on_click(handler)
        } else {
            button
        }
    }
}
