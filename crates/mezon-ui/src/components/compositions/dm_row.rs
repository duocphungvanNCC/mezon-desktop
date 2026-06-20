use gpui::{AnyElement, ElementId, SharedString, div, prelude::*, px};
use mezon_store::DirectKind;

use crate::components::primitives::Avatar;
use crate::router::{Route, navigate};
use crate::theme::Theme;

pub struct DmRow {
    id: String,
    label: String,
    kind: DirectKind,
    selected: bool,
    online: bool,
    avatar_src: String,
    avatar_raw: String,
}

impl DmRow {
    pub fn new(id: impl Into<String>, label: impl Into<String>, kind: DirectKind) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind,
            selected: false,
            online: false,
            avatar_src: String::new(),
            avatar_raw: String::new(),
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn online(mut self, online: bool) -> Self {
        self.online = online;
        self
    }

    pub fn avatar_src(mut self, src: impl Into<String>) -> Self {
        self.avatar_src = src.into();
        self
    }

    pub fn avatar_raw(mut self, raw: impl Into<String>) -> Self {
        self.avatar_raw = raw.into();
        self
    }

    pub fn render(self, theme: &Theme) -> impl IntoElement {
        let elem_id: ElementId = SharedString::from(format!("dm-{}", self.id)).into();
        let group_name: SharedString = format!("dm-row-{}", self.id).into();
        let nav_id = self.id.clone();
        let channel_type = self.kind.channel_type();
        let label: SharedString = self.label.clone().into();
        let bg_hover = theme.bg_hover;
        let muted = theme.text_muted;
        let selected = self.selected;
        let name_color = if selected {
            theme.text_primary
        } else {
            theme.tokens.text_theme_primary
        };

        let avatar_slot = self.render_avatar(theme);

        let close_btn = div()
            .id(SharedString::from(format!("dm-close-{}", self.id)))
            .flex()
            .items_center()
            .justify_center()
            .size(px(20.))
            .text_size(px(20.))
            .opacity(0.)
            .text_color(muted)
            .cursor_pointer()
            .group_hover(group_name.clone(), |this| this.opacity(1.))
            .hover(move |this| this.text_color(gpui::rgb(0xef4444)))
            .on_click(|_, _window, cx| cx.stop_propagation())
            .child("×");

        div()
            .id(elem_id)
            .group(group_name)
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .h(px(42.))
            .w_full()
            .px_2()
            .rounded_md()
            .cursor_pointer()
            .when(selected, |this| this.bg(bg_hover))
            .hover(move |this| this.bg(bg_hover))
            .on_click(move |_, _window, cx| {
                navigate(
                    cx,
                    Route::DirectMessage {
                        direct_id: nav_id.clone(),
                        message_type: channel_type.to_string(),
                    },
                );
            })
            .child(avatar_slot)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(name_color)
                    .truncate()
                    .child(label),
            )
            .child(close_btn)
    }

    fn render_avatar(&self, theme: &Theme) -> AnyElement {
        let size = px(32.);
        let online = self.online && self.kind == DirectKind::Dm;

        let inner: AnyElement = if self.kind == DirectKind::Group && self.avatar_src.is_empty() {
            div()
                .size(size)
                .flex_shrink_0()
                .rounded_full()
                .child(
                    gpui::img(crate::util::assets::AVATAR_GROUP)
                        .size(size)
                        .rounded_full()
                        .object_fit(gpui::ObjectFit::Cover),
                )
                .into_any_element()
        } else {
            let mut avatar = Avatar::new().name(self.label.clone()).size_px(size);
            let src = self.avatar_src.clone();
            let raw = self.avatar_raw.clone();
            if !src.is_empty() {
                let proxied = src.clone();
                avatar = avatar.src(src);
                if !raw.is_empty() && raw != proxied {
                    avatar = avatar.fallback_src(raw);
                }
            } else if !raw.is_empty() {
                avatar = avatar.src(raw);
            }
            avatar.into_any_element()
        };

        div()
            .relative()
            .flex_shrink_0()
            .size(size)
            .child(inner)
            .when(online, |this| {
                this.child(
                    div()
                        .absolute()
                        .bottom(px(-1.))
                        .right(px(-1.))
                        .size(px(10.))
                        .rounded_full()
                        .border_2()
                        .border_color(theme.bg_secondary)
                        .bg(theme.status_online),
                )
            })
            .into_any_element()
    }
}
