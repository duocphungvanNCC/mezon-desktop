use std::time::Duration;

use crate::components::primitives::{
    Avatar, Button as GpuiButton, ButtonVariants, Label, Sizable, Size, h_flex, v_flex,
};
use gpui::{
    Context, Entity, FontWeight, Rgba, SharedString, Task, Window, deferred, div, prelude::*, px,
};
use mezon_store::{AccountStore, Settings};

use crate::image_cache::LruImageCache;
use crate::theme::ActiveTheme;

pub struct AccountPage {
    settings: Entity<Settings>,
    avatar_image_cache: Entity<LruImageCache>,
    banner_color: Option<Rgba>,
    banner_source: String,
    banner_task: Option<Task<()>>,
    toast_message: Option<SharedString>,
}

impl AccountPage {
    pub fn new(settings: Entity<Settings>, cx: &mut Context<Self>) -> Self {
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        cx.observe(&AccountStore::global(cx), |this, _, cx| {
            this.refresh_banner_source(cx);
            cx.notify();
        })
        .detach();
        AccountStore::global(cx).update(cx, |store, cx| store.ensure_account(cx));
        let mut page = Self {
            settings,
            avatar_image_cache: crate::image_cache::shared_avatar_cache(cx),
            banner_color: None,
            banner_source: String::new(),
            banner_task: None,
            toast_message: None,
        };
        page.refresh_banner_source(cx);
        page
    }

    fn show_toast(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast_message = Some(message.into());
        cx.notify();

        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            this.update(cx, |this, cx| {
                this.toast_message = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn refresh_banner_source(&mut self, cx: &mut Context<Self>) {
        let source = AccountStore::global(cx)
            .read(cx)
            .account
            .as_ref()
            .and_then(|account| account.avatar_url.as_deref())
            .map(|url| crate::util::imgproxy::avatar_url(cx, url))
            .unwrap_or_default();
        if source == self.banner_source {
            return;
        }
        self.banner_source = source.clone();
        self.banner_color = None;
        self.load_banner_color(source, cx);
    }

    fn load_banner_color(&mut self, avatar_url: String, cx: &mut Context<Self>) {
        if avatar_url.is_empty() {
            self.banner_task = None;
            return;
        }
        let avatar_image_cache = self.avatar_image_cache.clone();
        let resource = gpui::Resource::Uri(avatar_url.into());
        self.banner_task = Some(cx.spawn(async move |this, cx| {
            for attempt in 0..60 {
                let image = avatar_image_cache
                    .read_with(cx, |cache, _| cache.cached_render_image(&resource));
                if let Some(image) = image
                    && let Some(bytes) = image.as_bytes(0)
                    && let Some(color) = crate::chat::user_profile_modal::average_bgra_color(bytes)
                {
                    let _ = this.update(cx, |this, cx| {
                        this.banner_color = Some(color);
                        cx.notify();
                    });
                    break;
                }
                let delay_ms = if attempt < 20 { 50 } else { 200 };
                cx.background_executor()
                    .timer(Duration::from_millis(delay_ms))
                    .await;
            }
        }));
    }
}

impl Render for AccountPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let locale = self.settings.read(cx).language.clone();
        let store = AccountStore::global(cx).read(cx);

        if store.account_error {
            return v_flex()
                .gap_4()
                .child(
                    Label::new(mezon_i18n::t(&locale, "setting.account.failedToLoad"))
                        .text_color(theme.text_muted),
                )
                .into_any_element();
        }

        if store.account_loading || store.account.is_none() {
            return v_flex()
                .gap_4()
                .child(
                    Label::new(mezon_i18n::t(&locale, "setting.account.loading"))
                        .text_color(theme.text_muted),
                )
                .into_any_element();
        }

        let account = store.account.as_ref().unwrap();

        let display_name: SharedString = account.display_name.clone().into();
        let username: SharedString = account.username.clone().into();

        let email_display = match &account.email {
            Some(email) if !email.is_empty() => SharedString::from(mask_email(email)),
            _ => SharedString::from(mezon_i18n::t(&locale, "common.notSet")),
        };

        let password_label = if account.password_setted {
            SharedString::from(mezon_i18n::t(&locale, "setting.account.changePassword"))
        } else {
            SharedString::from(mezon_i18n::t(&locale, "setting.account.setPassword"))
        };

        let password_display = if account.password_setted {
            SharedString::from("*********")
        } else {
            SharedString::from(mezon_i18n::t(&locale, "setting.account.password"))
        };

        let phone_display = account
            .phone_number
            .as_ref()
            .map(|s| SharedString::from(s.as_str()))
            .unwrap_or(SharedString::from(mezon_i18n::t(&locale, "common.notSet")));

        let phone_label = if account.phone_number.is_some() {
            SharedString::from(mezon_i18n::t(&locale, "setting.account.changePhone"))
        } else {
            SharedString::from(mezon_i18n::t(&locale, "setting.account.setPhone"))
        };

        let avatar_url = account
            .avatar_url
            .as_ref()
            .map(|url| SharedString::from(crate::util::imgproxy::avatar_url(cx, url)))
            .unwrap_or_default();
        let banner_color = self
            .banner_color
            .map(gpui::Hsla::from)
            .unwrap_or(theme.tokens.bg_secondary.into());

        let account_field = |label: SharedString, value: SharedString| {
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text_muted)
                        .child(label),
                )
                .child(Label::new(value).text_color(theme.text_secondary))
        };

        let outlined_button = |id: &'static str, label: SharedString| {
            div()
                .id(id)
                .h(px(40.0))
                .px_3()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded_md()
                .bg(theme.tokens.bg_button_secondary)
                .border_1()
                .border_color(theme.border)
                .text_color(theme.text_secondary)
                .cursor_pointer()
                .hover(move |style| style.text_color(theme.text_primary))
                .child(label)
        };

        v_flex()
            .text_sm()
            .child(
                v_flex()
                    .relative()
                    .rounded_lg()
                    .overflow_hidden()
                    .bg(theme.bg_primary)
                    .shadow_md()
                    .child(div().h(px(100.0)).w_full().bg(banner_color))
                    .child(
                        h_flex()
                            .h(px(104.))
                            .px_5()
                            .gap_4()
                            .items_center()
                            .child(
                                Label::new(display_name.clone())
                                    .relative()
                                    .top(px(-26.0))
                                    .ml(px(128.0))
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text_secondary),
                            )
                            .child(div().flex_1())
                            .child(
                                GpuiButton::new("edit-profile-btn")
                                    .label(mezon_i18n::t(
                                        &locale,
                                        "setting.account.editUserProfile",
                                    ))
                                    .relative()
                                    .top(px(-8.0))
                                    .with_size(Size::Large)
                                    .primary()
                                    .on_click(move |_, _, cx| {
                                        crate::router::replace(
                                            cx,
                                            crate::router::Route::SettingsProfile,
                                        );
                                    }),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_4()
                            .px_4()
                            .pb_4()
                            .child(
                                v_flex()
                                    .gap_4()
                                    .p_4()
                                    .rounded_md()
                                    .bg(theme.bg_secondary)
                                    .shadow_sm()
                                    .child(
                                        h_flex()
                                            .justify_between()
                                            .items_center()
                                            .child(account_field(
                                                mezon_i18n::t(
                                                    &locale,
                                                    "setting.account.displayName",
                                                )
                                                .into(),
                                                display_name.clone(),
                                            ))
                                            .child(
                                                outlined_button(
                                                    "edit-display-name-btn",
                                                    mezon_i18n::t(&locale, "accountSetting.edit")
                                                        .into(),
                                                )
                                                .on_click(move |_, _, cx| {
                                                    crate::router::replace(
                                                        cx,
                                                        crate::router::Route::SettingsProfile,
                                                    );
                                                }),
                                            ),
                                    )
                                    .child(account_field(
                                        mezon_i18n::t(&locale, "setting.account.username").into(),
                                        username,
                                    )),
                            )
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .p_4()
                                    .rounded_md()
                                    .bg(theme.bg_secondary)
                                    .shadow_sm()
                                    .child(account_field(
                                        mezon_i18n::t(&locale, "setting.account.email").into(),
                                        email_display,
                                    )),
                            )
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .p_4()
                                    .rounded_md()
                                    .bg(theme.bg_secondary)
                                    .shadow_sm()
                                    .child(account_field(
                                        mezon_i18n::t(&locale, "setting.account.password").into(),
                                        password_display,
                                    ))
                                    .child(
                                        outlined_button("password-btn", password_label).on_click(
                                            cx.listener(|this, _, _, cx| {
                                                let locale =
                                                    this.settings.read(cx).language.clone();
                                                this.show_toast(
                                                    mezon_i18n::t(
                                                        &locale,
                                                        "setting.account.passwordComingSoon",
                                                    ),
                                                    cx,
                                                );
                                            }),
                                        ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .p_4()
                                    .rounded_md()
                                    .bg(theme.bg_secondary)
                                    .shadow_sm()
                                    .child(account_field(
                                        mezon_i18n::t(&locale, "setting.account.phone").into(),
                                        phone_display,
                                    ))
                                    .child(outlined_button("phone-btn", phone_label).on_click(
                                        cx.listener(|this, _, _, cx| {
                                            let locale = this.settings.read(cx).language.clone();
                                            this.show_toast(
                                                mezon_i18n::t(
                                                    &locale,
                                                    "setting.account.phoneComingSoon",
                                                ),
                                                cx,
                                            );
                                        }),
                                    )),
                            ),
                    )
                    .child(deferred(
                        div()
                            .absolute()
                            .left(px(20.0))
                            .top(px(72.0))
                            .size(px(108.0))
                            .rounded_full()
                            .bg(theme.bg_primary)
                            .p(px(6.0))
                            .child(
                                Avatar::new()
                                    .when(!avatar_url.is_empty(), |avatar| {
                                        avatar.src(avatar_url.clone())
                                    })
                                    .name(display_name)
                                    .size_px(px(96.0))
                                    .image_cache(self.avatar_image_cache.clone()),
                            ),
                    )),
            )
            .when_some(self.toast_message.clone(), |this, msg| {
                this.child(div().text_sm().text_color(theme.text_muted).child(msg))
            })
            .into_any_element()
    }
}

fn mask_email(email: &str) -> String {
    let at = email.find('@').unwrap_or(email.len());
    if at > 1 {
        format!("{}***{}", &email[..1], &email[at..])
    } else {
        email.to_string()
    }
}
