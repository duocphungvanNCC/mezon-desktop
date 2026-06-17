pub mod account_test_view;
pub mod assets;
pub mod channel_sidebar;
pub mod chat;
pub mod chat_area;
pub mod chat_layout;
pub mod clan_sidebar;
pub mod components;
pub mod dev_gallery;
pub mod login_view;
pub mod main_layout;
pub mod root;
pub mod router;
pub mod settings;
pub mod text_utils;
pub mod theme;
pub mod title_bar;

pub use account_test_view::AccountTestView;
pub use channel_sidebar::ChannelSidebar;
pub use chat_layout::ChatLayout;
pub use clan_sidebar::ClanSidebar;
pub use dev_gallery::DevGallery;
pub use login_view::LoginView;
pub use root::RootView;
pub use router::{Route, Router};
pub use settings::SettingsScreen;
pub use theme::Theme;

pub fn init(cx: &mut gpui::App) {
    ::theme::init(::theme::LoadThemes::JustBase, cx);
    theme::init_theme_settings_provider(cx);
    cx.bind_keys([gpui::KeyBinding::new(
        "escape",
        ::menu::Cancel,
        Some("menu"),
    )]);
    components::primitives::init_input(cx);
    router::Router::init(cx);
}
