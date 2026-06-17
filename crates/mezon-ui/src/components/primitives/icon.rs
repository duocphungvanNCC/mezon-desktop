use gpui::{Svg, prelude::*, px, svg};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IconName {
    ArrowDown,
    ArrowRight,
    Check,
    CircleUser,
    Close,
    Deafen,
    FolderOpen,
    GalleryVerticalEnd,
    Inbox,
    LayoutDashboard,
    Loader,
    Mic,
    Plus,
    Settings,
    Speaker,
    TriangleAlert,
    WindowMaximize,
}

impl IconName {
    pub fn path(self) -> &'static str {
        match self {
            Self::ArrowDown => "icons/arrow-down.svg",
            Self::ArrowRight => "icons/arrow-right.svg",
            Self::Check => "icons/check.svg",
            Self::CircleUser => "icons/circle-user.svg",
            Self::Close => "icons/close.svg",
            Self::Deafen => "icons/deafen.svg",
            Self::FolderOpen => "icons/folder-open.svg",
            Self::GalleryVerticalEnd => "icons/gallery-vertical-end.svg",
            Self::Inbox => "icons/inbox.svg",
            Self::LayoutDashboard => "icons/layout-dashboard.svg",
            Self::Loader => "icons/loader.svg",
            Self::Mic => "icons/mic.svg",
            Self::Plus => "icons/plus.svg",
            Self::Settings => "icons/settings.svg",
            Self::Speaker => "icons/speaker.svg",
            Self::TriangleAlert => "icons/triangle-alert.svg",
            Self::WindowMaximize => "icons/window-maximize.svg",
        }
    }
}

pub struct Icon;

impl Icon {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(name: IconName) -> Svg {
        svg().path(name.path()).size(px(16.)).flex_none()
    }
}
