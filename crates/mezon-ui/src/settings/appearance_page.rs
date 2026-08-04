use crate::components::primitives::{Avatar, Icon, IconName, Label, h_flex, v_flex};
use crate::image_cache::LruImageCache;
use crate::theme::{ActiveTheme, Theme, resolve_theme, set_theme};
use gpui::{
    AnyElement, Context, Entity, FontWeight, Rgba, Window, div, linear_color_stop, linear_gradient,
    prelude::*, px, rgb,
};
use mezon_store::{AccountStore, Settings};
use ui::Tooltip;

const THEME_KEYS: &[&str] = &[
    "dark",
    "light",
    "sunrise",
    "purple_haze",
    "redDark",
    "abyss_dark",
    "berrynade",
    "cisher",
    "sunset",
];

pub struct AppearancePage {
    settings: Entity<Settings>,
    account_store: Entity<AccountStore>,
    avatar_image_cache: Entity<LruImageCache>,
}

impl AppearancePage {
    pub fn new(settings: Entity<Settings>, cx: &mut Context<Self>) -> Self {
        let account_store = AccountStore::global(cx);
        cx.observe(&account_store, |_, _, cx| cx.notify()).detach();
        account_store.update(cx, |store, cx| store.ensure_account(cx));
        Self {
            settings,
            account_store,
            avatar_image_cache: crate::image_cache::shared_avatar_cache(cx),
        }
    }
}

fn rgba(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
}

fn canonical_theme_key(key: &str) -> &str {
    match key {
        "purple" => "purple_haze",
        "abyss" => "abyss_dark",
        "red_dark" => "redDark",
        key => key,
    }
}

fn message_row(
    avatar_src: String,
    display_name: String,
    timestamp: String,
    message: String,
    theme: &Theme,
    avatar_image_cache: Entity<LruImageCache>,
) -> impl IntoElement {
    h_flex()
        .px_5()
        .gap_3()
        .items_start()
        .child(
            Avatar::new()
                .size_px(px(45.0))
                .name(display_name.clone())
                .image_cache(avatar_image_cache)
                .when(!avatar_src.is_empty(), |avatar| avatar.src(avatar_src)),
        )
        .child(
            v_flex()
                .min_w_0()
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            Label::new(display_name)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text_primary),
                        )
                        .child(Label::new(timestamp).text_xs().text_color(theme.text_muted)),
                )
                .child(Label::new(message).text_color(theme.text_secondary)),
        )
}

fn swatch_background(key: &str) -> AnyElement {
    let base = div().absolute().inset_0().rounded_full();
    match key {
        "dark" => base.bg(rgb(0x26272b)).into_any_element(),
        "light" => base.bg(rgb(0xffffff)).into_any_element(),
        "sunrise" => base
            .bg(linear_gradient(
                135.,
                linear_color_stop(rgb(0xe0c3fc), 0.),
                linear_color_stop(rgb(0xfff1eb), 1.),
            ))
            .into_any_element(),
        "purple_haze" => base
            .bg(linear_gradient(
                135.,
                linear_color_stop(rgb(0xa78bfa), 0.),
                linear_color_stop(rgb(0x60a5fa), 1.),
            ))
            .into_any_element(),
        "redDark" => base
            .bg(linear_gradient(
                135.,
                linear_color_stop(rgb(0x3b0a0a), 0.),
                linear_color_stop(rgb(0xe11d48), 1.),
            ))
            .into_any_element(),
        "abyss_dark" => base
            .bg(linear_gradient(
                135.,
                linear_color_stop(rgb(0x0f172a), 0.),
                linear_color_stop(rgb(0x6d28d9), 1.),
            ))
            .into_any_element(),
        "berrynade" => base
            .bg(linear_gradient(
                161.,
                linear_color_stop(rgb(0x52122f), 0.),
                linear_color_stop(rgb(0x6f5018), 1.),
            ))
            .into_any_element(),
        "cisher" => base
            .bg(linear_gradient(
                135.,
                linear_color_stop(rgb(0xf8dfaf), 0.),
                linear_color_stop(rgb(0xf4c7b3), 1.),
            ))
            .into_any_element(),
        "sunset" => base
            .bg(linear_gradient(
                142.,
                linear_color_stop(rgb(0x22132f), 0.),
                linear_color_stop(rgb(0x513423), 1.),
            ))
            .into_any_element(),
        _ => base.bg(resolve_theme(key).bg_primary).into_any_element(),
    }
}

fn theme_swatch(
    key: String,
    label: String,
    is_selected: bool,
    theme: &Theme,
    settings: Entity<Settings>,
) -> impl IntoElement {
    let click_key = key.clone();
    div()
        .id(key.clone())
        .relative()
        .size(px(60.0))
        .rounded_full()
        .border_color(if is_selected {
            theme.brand
        } else {
            theme.border
        })
        .when(is_selected, |el| el.border_2().shadow_lg())
        .when(!is_selected, |el| el.border_1())
        .cursor_pointer()
        .hover(|el| el.opacity(0.9))
        .tooltip(Tooltip::text(label))
        .on_click(move |_, _, cx| {
            set_theme(resolve_theme(&click_key), cx);
            settings.update(cx, |s, cx| {
                s.theme = click_key.clone();
                cx.notify();
            });
            mezon_store::schedule_settings_save(&settings, cx);
        })
        .child(swatch_background(&key))
        .when(is_selected, |el| {
            el.child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .rounded_full()
                    .bg(theme.brand)
                    .p(px(2.0))
                    .child(
                        Icon::new(IconName::Check)
                            .size_3()
                            .text_color(rgba(255, 255, 255, 1.0)),
                    ),
            )
        })
}

impl Render for AppearancePage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current_theme = self.settings.read(cx).theme.clone();
        let locale = self.settings.read(cx).language.clone();
        let theme = cx.theme();
        let settings = self.settings.clone();

        let account = self.account_store.read(cx).account.as_ref();
        let display_name = account
            .map(|account| {
                if account.display_name.is_empty() {
                    account.username.clone()
                } else {
                    account.display_name.clone()
                }
            })
            .unwrap_or_else(|| "Mezon".to_string());
        let avatar_src = account
            .and_then(|account| account.avatar_url.as_deref())
            .map(|url| crate::util::imgproxy::profile_url(cx, url))
            .unwrap_or_default();
        let time = chrono::Local::now().format("%I:%M %p").to_string();
        let timestamp = format!(
            "{} {}",
            mezon_i18n::t(&locale, "common.todayAt"),
            time.trim_start_matches('0')
        );
        let sample_message = mezon_i18n::t(&locale, "common.sampleMessage").to_string();

        let themes = [
            ("dark", "appThemeSetting.fields.dark"),
            ("light", "appThemeSetting.fields.light"),
            ("sunrise", "appThemeSetting.fields.sunrise"),
            ("purple_haze", "appThemeSetting.fields.purpleHaze"),
            ("redDark", "appThemeSetting.fields.redDark"),
            ("abyss_dark", "appThemeSetting.fields.abyssDark"),
            ("berrynade", "appThemeSetting.fields.berrynade"),
            ("cisher", "appThemeSetting.fields.cisher"),
            ("sunset", "appThemeSetting.fields.sunset"),
        ];

        v_flex()
            .child(
                v_flex()
                    .h(px(150.0))
                    .rounded_lg()
                    .bg(theme.bg_secondary)
                    .border_1()
                    .border_color(theme.border)
                    .pb_5()
                    .overflow_hidden()
                    .child(div().mt(px(-15.0)).child(message_row(
                        avatar_src.clone(),
                        display_name.clone(),
                        timestamp.clone(),
                        sample_message.clone(),
                        theme,
                        self.avatar_image_cache.clone(),
                    )))
                    .child(div().mt_5().child(message_row(
                        avatar_src.clone(),
                        display_name.clone(),
                        timestamp.clone(),
                        sample_message.clone(),
                        theme,
                        self.avatar_image_cache.clone(),
                    )))
                    .child(div().mt_5().child(message_row(
                        avatar_src,
                        display_name,
                        timestamp,
                        sample_message,
                        theme,
                        self.avatar_image_cache.clone(),
                    ))),
            )
            .child(
                v_flex()
                    .mt(px(40.0))
                    .p_5()
                    .gap_2()
                    .rounded_lg()
                    .bg(theme.bg_secondary)
                    .child(
                        Label::new(mezon_i18n::t(&locale, "setting.appearance.theme"))
                            .text_color(theme.text_primary)
                            .font_weight(FontWeight::BOLD),
                    )
                    .child(
                        h_flex()
                            .mt_5()
                            .flex_wrap()
                            .gap_x(px(30.0))
                            .gap_y_4()
                            .children(THEME_KEYS.iter().zip(themes.iter()).map(
                                |(key, (_, label_key))| {
                                    theme_swatch(
                                        (*key).to_string(),
                                        mezon_i18n::t(&locale, label_key).to_string(),
                                        canonical_theme_key(&current_theme) == *key,
                                        theme,
                                        settings.clone(),
                                    )
                                },
                            )),
                    ),
            )
    }
}
