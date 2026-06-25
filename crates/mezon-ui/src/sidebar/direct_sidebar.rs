use gpui::{
    Context, Entity, FontWeight, UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};
use mezon_store::{DirectMessageStore, Settings};

use crate::components::compositions::DmRow;
use crate::components::primitives::{Icon, IconName};
use crate::router::{Route, Router, navigate};
use crate::theme::{ActiveTheme, Theme};

pub struct DirectSidebar {
    direct_store: Entity<DirectMessageStore>,
    settings: Entity<Settings>,
    list_scroll: UniformListScrollHandle,
}

impl DirectSidebar {
    pub fn new(settings: Entity<Settings>, cx: &mut Context<Self>) -> Self {
        let direct_store = DirectMessageStore::global(cx);
        cx.observe(&direct_store, |_, _, cx| cx.notify()).detach();
        cx.observe(&Router::global(cx), |_, _, cx| cx.notify())
            .detach();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        Self {
            direct_store,
            settings,
            list_scroll: UniformListScrollHandle::new(),
        }
    }

    fn render_search(&self, theme: &Theme, locale: &str) -> impl IntoElement {
        div()
            .w_full()
            .h(px(50.))
            .px_3()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .w_full()
                    .h(px(36.))
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .rounded_lg()
                    .bg(theme.tokens.bg_tertiary)
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.text_muted)
                            .child(mezon_i18n::t(locale, "clan.findOrStartConversation")),
                    ),
            )
    }

    fn render_friends_button(&self, theme: &Theme, locale: &str, active: bool) -> impl IntoElement {
        let bg_hover = theme.bg_hover;
        div()
            .id("dm-friends")
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .py_2()
            .px_3()
            .rounded_lg()
            .cursor_pointer()
            .when(active, |this| this.bg(bg_hover))
            .hover(move |this| this.bg(bg_hover))
            .on_click(|_, _window, cx| navigate(cx, Route::Friends))
            .child(
                Icon::new(IconName::IconFriends)
                    .size(px(20.))
                    .text_color(theme.text_secondary),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_primary)
                    .child(mezon_i18n::t(locale, "directMessage.friends")),
            )
    }

    fn render_section_header(&self, theme: &Theme, locale: &str) -> impl IntoElement {
        let bg_hover = theme.bg_hover;
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_4()
            .pt_4()
            .pb_1()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text_muted)
                    .child(mezon_i18n::t(locale, "directMessage.directMessages")),
            )
            .child(
                div()
                    .id("dm-create")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(20.))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(move |this| this.bg(bg_hover))
                    .child(
                        Icon::new(IconName::Plus)
                            .size(px(16.))
                            .text_color(theme.text_muted),
                    ),
            )
    }
}

impl Render for DirectSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::trace_render!("DirectSidebar");
        let theme = cx.theme();
        let locale = self.settings.read(cx).language.clone();

        let count = self.direct_store.read(cx).channels().len();
        let active_id = match Router::global(cx).read(cx).route() {
            Route::DirectMessage { direct_id, .. } => Some(direct_id),
            _ => None,
        };
        let store = self.direct_store.clone();

        let list = uniform_list("dm-list", count, move |range, _window, cx| {
            let theme = cx.theme().clone();
            let active_id = active_id;
            let store = store.read(cx);
            range
                .map(|ix| match store.channels().get(ix) {
                    Some(channel) => {
                        let selected = active_id == Some(channel.id);
                        let avatar_src = crate::util::imgproxy::avatar_url(cx, &channel.avatar);
                        DmRow::new(channel.id.to_string(), channel.label.clone(), channel.kind)
                            .selected(selected)
                            .online(channel.online)
                            .avatar_src(avatar_src)
                            .avatar_raw(channel.avatar.clone())
                            .render(&theme)
                            .into_any_element()
                    }
                    None => div().into_any_element(),
                })
                .collect::<Vec<_>>()
        })
        .track_scroll(&self.list_scroll)
        .flex_1()
        .min_h_0()
        .px_2();

        let on_friends = matches!(Router::global(cx).read(cx).route(), Route::Friends);

        div()
            .flex()
            .flex_col()
            .size_full()
            .pb(px(68.))
            .bg(theme.bg_secondary)
            .child(self.render_search(theme, &locale))
            .child(
                div()
                    .px_2()
                    .pt_2()
                    .child(self.render_friends_button(theme, &locale, on_friends)),
            )
            .child(self.render_section_header(theme, &locale))
            .child(list)
    }
}
