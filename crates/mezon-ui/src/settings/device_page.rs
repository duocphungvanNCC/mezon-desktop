use crate::components::primitives::{Icon, IconName, Label, h_flex, v_flex};
use gpui::{ClickEvent, Context, Entity, FontWeight, Window, div, prelude::*};
use mezon_store::{AccountStore, Settings};

use crate::theme::ActiveTheme;

pub struct DevicePage {
    fetch_started: bool,
}

impl DevicePage {
    pub fn new(settings: Entity<Settings>, cx: &mut Context<Self>) -> Self {
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        cx.observe(&AccountStore::global(cx), |_, _, cx| cx.notify())
            .detach();
        Self {
            fetch_started: false,
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        AccountStore::global(cx).update(cx, |store, cx| store.fetch_devices(cx));
    }
}

impl Render for DevicePage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.fetch_started {
            self.fetch_started = true;
            AccountStore::global(cx).update(cx, |store, cx| store.fetch_devices(cx));
        }

        let theme = cx.theme();
        let store = AccountStore::global(cx).read(cx);

        v_flex()
            .gap_4()
            .child(
                Label::new("Devices")
                    .text_xl()
                    .text_color(theme.text_primary)
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .child(
                Label::new("Manage the devices that have access to your account.")
                    .text_sm()
                    .text_color(theme.text_muted),
            )
            .child(if let Some(error) = &store.devices_error {
                div()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child(error.clone())
                    .into_any_element()
            } else if store.devices_loading {
                div()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child("Loading devices...")
                    .into_any_element()
            } else if store.devices.is_empty() {
                div()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child("No devices found.")
                    .into_any_element()
            } else {
                let current: Vec<_> = store.devices.iter().filter(|d| d.is_current).collect();
                let others: Vec<_> = store.devices.iter().filter(|d| !d.is_current).collect();

                v_flex()
                    .gap_6()
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                Label::new("CURRENT DEVICE")
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text_muted),
                            )
                            .children(current.iter().map(|device| {
                                h_flex()
                                    .items_center()
                                    .gap_3()
                                    .px_4()
                                    .py_3()
                                    .rounded_lg()
                                    .bg(theme.bg_primary)
                                    .child(
                                        Icon::new(IconName::WindowMaximize)
                                            .size_5()
                                            .text_color(theme.status_online),
                                    )
                                    .child(
                                        v_flex()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(theme.text_primary)
                                                    .child(device.device_name.clone()),
                                            )
                                            .child(
                                                div().text_xs().text_color(theme.text_muted).child(
                                                    format!(
                                                        "{} · {} · Active now",
                                                        device.platform, device.ip
                                                    ),
                                                ),
                                            ),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.status_online)
                                            .child("Active now"),
                                    )
                                    .into_any_element()
                            })),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                Label::new("OTHER DEVICES")
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text_muted),
                            )
                            .when(others.is_empty(), |el| {
                                el.child(
                                    div()
                                        .text_sm()
                                        .text_color(theme.text_muted)
                                        .px_4()
                                        .child("No other devices."),
                                )
                            })
                            .children(others.iter().map(|device| {
                                let last_active = device.last_active_label.clone();
                                let device_id = device.device_id.clone();
                                h_flex()
                                    .items_center()
                                    .gap_3()
                                    .px_4()
                                    .py_3()
                                    .rounded_lg()
                                    .bg(theme.bg_secondary)
                                    .child(
                                        Icon::new(IconName::Speaker)
                                            .size_5()
                                            .text_color(theme.text_secondary),
                                    )
                                    .child(
                                        v_flex()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(theme.text_primary)
                                                    .child(device.device_name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme.text_muted)
                                                    .child(format!(
                                                        "{} · {}",
                                                        device.platform, device.location
                                                    )),
                                            ),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.text_muted)
                                            .child(last_active),
                                    )
                                    .child(
                                        div()
                                            .id(format!("remove-device-{}", device_id))
                                            .cursor_pointer()
                                            .text_color(theme.status_dnd)
                                            .child("✕")
                                            .on_click(
                                                move |_: &ClickEvent,
                                                      _: &mut Window,
                                                      _cx: &mut gpui::App| {
                                                    tracing::info!("Remove device: {}", device_id);
                                                },
                                            ),
                                    )
                                    .into_any_element()
                            })),
                    )
                    .into_any_element()
            })
    }
}
