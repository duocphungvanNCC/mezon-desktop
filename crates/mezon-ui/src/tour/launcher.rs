use gpui::{
    App, Context, FocusHandle, FontWeight, IntoElement, SharedString, Window, div, prelude::*, px,
};
use mezon_store::Settings;

use super::state::{TourState, available_tracks};
use crate::app::shell::Shell;
use crate::components::primitives::{Button, ButtonVariants, Icon, IconName, h_flex, v_flex};
use crate::theme::ActiveTheme;

pub struct TourLauncher {
    focus_handle: FocusHandle,
}

impl TourLauncher {
    pub fn open(window: &mut Window, cx: &mut App) {
        let view = cx.new(|cx| Self {
            focus_handle: cx.focus_handle(),
        });
        window.focus(&view.read(cx).focus_handle.clone(), cx);
        Shell::global(cx).update(cx, |shell, cx| shell.show_modal(view.into(), cx));
    }
}

impl Render for TourLauncher {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let settings = Settings::try_global(cx);
        let locale = settings
            .as_ref()
            .map(|settings| settings.read(cx).language.clone())
            .unwrap_or_else(|| "en".to_string());
        let done: Vec<String> = settings
            .map(|settings| settings.read(cx).tour_done_tracks.clone())
            .unwrap_or_default();
        let tracks = available_tracks(cx);

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("menu")
            .on_action(cx.listener(|_, _: &::menu::Cancel, _window, cx| {
                Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
            }))
            .w(px(460.))
            .gap_3()
            .p(px(20.))
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.bg_floating)
            .shadow_lg()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text_primary)
                    .child(SharedString::from(mezon_i18n::t(&locale, "tour.title"))),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.text_secondary)
                    .child(SharedString::from(mezon_i18n::t(&locale, "tour.subtitle"))),
            )
            .children(tracks.into_iter().map(|track| {
                let id = track.id;
                let seen = done.iter().any(|entry| entry == id);
                h_flex()
                    .id(id)
                    .items_center()
                    .gap_3()
                    .p(px(12.))
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border)
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.bg_hover))
                    .on_click(move |_, window, cx| {
                        Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
                        TourState::start_track(id, window, cx);
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text_primary)
                                    .child(SharedString::from(mezon_i18n::t(
                                        &locale,
                                        track.name_key,
                                    ))),
                            )
                            .child(div().text_xs().text_color(theme.text_muted).child(
                                SharedString::from(mezon_i18n::t(&locale, track.summary_key)),
                            )),
                    )
                    .when(seen, |el| {
                        el.child(
                            Icon::new(IconName::Check)
                                .size_4()
                                .text_color(theme.status_online),
                        )
                    })
            }))
            .child(
                h_flex().justify_end().child(
                    Button::new("tour-launcher-close")
                        .label(mezon_i18n::t(&locale, "tour.close"))
                        .ghost()
                        .on_click(|_, _window, cx| {
                            Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
                        }),
                ),
            )
    }
}
