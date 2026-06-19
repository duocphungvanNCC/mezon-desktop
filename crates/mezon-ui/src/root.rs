use crate::components::primitives::Sizable;
use gpui::{
    AnyView, App, ClickEvent, Context, Entity, FontWeight, MouseButton, NavigationDirection,
    StyleRefinement, Window, div, prelude::*,
};
use mezon_store::{AuthState, ClanList, Settings};

use crate::chat_layout::ChatLayout;
use crate::components::primitives::{Button, Icon, IconName, Size, Spinner};
use crate::direct_message::DirectMessageScreen;
use crate::login_view::LoginView;
use crate::router::{Route, Router};
use crate::settings::SettingsScreen;
use crate::theme::{ActiveTheme, Theme, resolve_theme};
use crate::title_bar::TitleBar;

pub struct RootView {
    title_bar: Entity<TitleBar>,
    auth_state: Entity<AuthState>,
    login_view: Entity<LoginView>,
    chat_layout: Entity<ChatLayout>,
    settings_screen: Entity<SettingsScreen>,
    direct_message_screen: Entity<DirectMessageScreen>,
}

impl RootView {
    pub fn new(
        title_bar: Entity<TitleBar>,
        auth_state: Entity<AuthState>,
        settings: Entity<Settings>,
        cx: &mut Context<Self>,
    ) -> Self {
        // App shell: owns the cross-cutting overlay layers (toasts + modal). Init before child
        // views so any of them can surface a toast/modal via `Shell::global`.
        let shell = crate::shell::Shell::init(cx);
        cx.observe(&shell, |_, _, cx| cx.notify()).detach();

        cx.observe(&settings, |_, settings, cx| {
            let name = settings.read(cx).theme.clone();
            crate::theme::set_theme(resolve_theme(&name), cx);
            cx.notify();
        })
        .detach();

        let login_view = cx.new({
            let auth_state = auth_state.clone();
            let settings = settings.clone();
            move |cx| LoginView::new(auth_state, settings, cx)
        });

        cx.observe(&Router::global(cx), |this, _, cx| {
            this.sync_settings_page(cx);
            cx.notify();
        })
        .detach();

        let clan_list: Entity<ClanList> = ClanList::global(cx);

        let clan_list_for_chat = clan_list.clone();
        let auth_state_for_chat = auth_state.clone();
        let settings_for_chat = settings.clone();
        let chat_layout = cx.new({
            let settings = settings_for_chat;
            move |cx| {
                ChatLayout::new(
                    clan_list_for_chat.clone(),
                    auth_state_for_chat.clone(),
                    settings.clone(),
                    cx,
                )
            }
        });

        let auth_state_for_settings = auth_state.clone();
        let clan_list_for_settings = clan_list.clone();
        let settings_screen = cx.new({
            let settings = settings.clone();
            move |cx| {
                SettingsScreen::new(
                    auth_state_for_settings.clone(),
                    settings.clone(),
                    clan_list_for_settings.clone(),
                    cx,
                )
            }
        });

        let direct_message_screen = cx.new({
            let clan_list = clan_list.clone();
            let auth_state = auth_state.clone();
            let settings = settings.clone();
            move |cx| DirectMessageScreen::new(clan_list, auth_state, settings, cx)
        });

        Self {
            title_bar,
            auth_state,
            login_view,
            chat_layout,
            settings_screen,
            direct_message_screen,
        }
    }

    fn sync_settings_page(&mut self, cx: &mut Context<Self>) {
        let page = match Router::global(cx).read(cx).route() {
            Route::SettingsProfile => crate::settings::SettingsPage::Profile,
            Route::SettingsDevices => crate::settings::SettingsPage::Device,
            Route::SettingsAppearance => crate::settings::SettingsPage::Appearance,
            Route::SettingsActivity => crate::settings::SettingsPage::Activity,
            Route::SettingsNotifications => crate::settings::SettingsPage::Notifications,
            Route::SettingsLanguage => crate::settings::SettingsPage::Language,
            Route::SettingsVoice => crate::settings::SettingsPage::Voice,
            Route::SettingsAdvanced => crate::settings::SettingsPage::Advanced,
            Route::SettingsAccount => crate::settings::SettingsPage::Account,
            _ => return,
        };
        self.settings_screen.update(cx, |s, _| s.set_page(page));
    }
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::trace_render!("RootView");
        let theme = cx.theme();
        let state = self.auth_state.read(cx).clone();

        let content: gpui::AnyElement = match state {
            AuthState::NotAuthenticated | AuthState::OtpRequested { .. } => {
                self.login_view.clone().into_any_element()
            }
            AuthState::AwaitingCallback => render_awaiting_callback(theme),
            AuthState::Connecting(_) => render_connecting(theme),
            AuthState::Authenticated(_) => {
                let route = Router::global(cx).read(cx).route();
                match route {
                    Route::SettingsAccount
                    | Route::SettingsProfile
                    | Route::SettingsDevices
                    | Route::SettingsAppearance
                    | Route::SettingsActivity
                    | Route::SettingsNotifications
                    | Route::SettingsLanguage
                    | Route::SettingsVoice
                    | Route::SettingsAdvanced => self.settings_screen.clone().into_any_element(),
                    Route::NotFound { .. } => render_not_found(theme),
                    Route::Direct | Route::DirectMessage { .. } => {
                        self.direct_message_screen.clone().into_any_element()
                    }
                    _ => self.chat_layout.clone().into_any_element(),
                }
            }
        };

        let overlay = crate::shell::Shell::global(cx).read(cx).render_overlay();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.bg_primary)
            .text_color(theme.text_primary)
            .on_action(cx.listener(|_, _: &crate::ToggleInspector, window, cx| {
                window.toggle_inspector(cx);
            }))
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Back),
                |_, _, cx| crate::router::go_back(cx),
            )
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Forward),
                |_, _, cx| crate::router::go_forward(cx),
            )
            .child(
                AnyView::from(self.title_bar.clone())
                    .cached(StyleRefinement::default().w_full().h_8()),
            )
            .child(content)
            .child(overlay)
    }
}

fn render_awaiting_callback(theme: &Theme) -> gpui::AnyElement {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .flex_col()
        .gap_4()
        .child(div().size_16().bg(theme.brand).rounded_lg())
        .child(
            div()
                .text_xl()
                .font_weight(FontWeight::BOLD)
                .text_color(theme.text_primary)
                .child("Mezon"),
        )
        .child(
            div()
                .text_sm()
                .text_color(theme.text_secondary)
                .child("Connecting - complete sign-in in your browser..."),
        )
        .into_any_element()
}

fn render_connecting(theme: &Theme) -> gpui::AnyElement {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .flex_col()
        .gap_4()
        .child(
            Spinner::new()
                .with_size(Size::Large)
                .color(theme.brand.into()),
        )
        .child(
            div()
                .text_xl()
                .font_weight(FontWeight::BOLD)
                .text_color(theme.text_primary)
                .child("Loading..."),
        )
        .into_any_element()
}

fn render_not_found(theme: &Theme) -> gpui::AnyElement {
    let back_btn = Button::new("back-to-chat").label("Back to Chat").on_click(
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            crate::router::go_back(cx);
        },
    );

    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .flex_col()
        .gap_4()
        .child(
            Icon::new(IconName::TriangleAlert)
                .size_8()
                .text_color(theme.text_muted),
        )
        .child(
            div()
                .text_xl()
                .font_weight(FontWeight::BOLD)
                .text_color(theme.text_primary)
                .child("Page Not Found"),
        )
        .child(
            div()
                .text_sm()
                .text_color(theme.text_secondary)
                .child("This path is not registered in the local Mezon router."),
        )
        .child(back_btn)
        .into_any_element()
}
