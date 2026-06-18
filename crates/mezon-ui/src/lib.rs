pub mod assets;
pub mod channel_sidebar;
pub mod chat;
pub mod chat_area;
pub mod chat_layout;
pub mod clan_sidebar;
pub mod components;
pub mod dev_gallery;
pub mod imgproxy;
pub mod login_view;
pub mod main_layout;
pub mod root;
pub mod router;
pub mod settings;
pub mod text_utils;
pub mod theme;
pub mod title_bar;

pub use channel_sidebar::ChannelSidebar;
pub use chat_layout::ChatLayout;
pub use clan_sidebar::ClanSidebar;
pub use dev_gallery::DevGallery;
pub use login_view::LoginView;
pub use root::RootView;
pub use router::{Route, Router};
pub use settings::SettingsScreen;
pub use theme::Theme;

gpui::actions!(mezon, [ToggleInspector]);

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
}
