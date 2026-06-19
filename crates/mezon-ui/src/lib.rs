pub mod assets;
pub mod channel_sidebar;
pub mod chat;
pub mod chat_area;
pub mod chat_layout;
pub mod clan_sidebar;
pub mod components;
pub mod dev_gallery;
pub mod direct_message;
pub mod imgproxy;
pub mod login_view;
pub mod root;
pub mod router;
pub mod settings;
pub mod shell;
pub mod text_utils;
pub mod theme;
pub mod title_bar;

pub use channel_sidebar::ChannelSidebar;
pub use chat_layout::ChatLayout;
pub use clan_sidebar::ClanSidebar;
pub use dev_gallery::DevGallery;
pub use direct_message::DirectMessageScreen;
pub use login_view::LoginView;
pub use root::RootView;
pub use router::{Route, Router};
pub use settings::SettingsScreen;
pub use shell::Shell;
pub use theme::Theme;

gpui::actions!(mezon, [ToggleInspector, Quit]);

#[macro_export]
macro_rules! trace_render {
    ($name:expr) => {
        $crate::trace_render!("{}", $name)
    };
    ($fmt:expr, $($arg:tt)+) => {{
        #[cfg(debug_assertions)]
        {
            static __RENDER_N: ::std::sync::atomic::AtomicU64 = ::std::sync::atomic::AtomicU64::new(0);
            ::tracing::trace!(
                target: "render",
                "{} #{}",
                ::std::format_args!($fmt, $($arg)+),
                __RENDER_N.fetch_add(1, ::std::sync::atomic::Ordering::Relaxed)
            );
        }
    }};
}

pub fn init(cx: &mut gpui::App) {
    ::theme::init(::theme::LoadThemes::JustBase, cx);
    theme::init_theme_settings_provider(cx);
    cx.bind_keys([gpui::KeyBinding::new(
        "escape",
        ::menu::Cancel,
        Some("menu"),
    )]);
    #[cfg(debug_assertions)]
    cx.bind_keys([gpui::KeyBinding::new("cmd-alt-i", ToggleInspector, None)]);
    components::primitives::init_input(cx);
    router::Router::init(cx);
    init_menus(cx);
}

/// macOS menu bar + Cmd+Q. The Edit items reuse the input component's own clipboard actions, so
/// the menu drives the same handlers as the in-input keybindings (cf. Zed's app menus).
fn init_menus(cx: &mut gpui::App) {
    use crate::components::primitives::input::{Copy, Cut, Paste, SelectAll};
    use gpui::{Menu, MenuItem, OsAction};

    cx.on_action(|_: &Quit, cx: &mut gpui::App| cx.quit());
    cx.bind_keys([gpui::KeyBinding::new("cmd-q", Quit, None)]);

    cx.set_menus(vec![
        Menu::new("Mezon").items([MenuItem::action("Quit Mezon", Quit)]),
        Menu::new("Edit").items([
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::separator(),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ]),
    ]);
}
