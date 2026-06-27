use gpui::{Entity, SharedString};

use crate::image_cache::LruImageCache;
use crate::theme::Theme;

/// Per-frame rendering context shared by every message row and part, so we do
/// not thread a long argument list through the whole render tree. Mirrors the
/// props React passes down from `ChannelMessages` to `MessageWithUser`.
pub struct RowCtx<'a> {
    pub theme: &'a Theme,
    pub locale: &'a str,
    /// Id of the signed-in user (used for "own message" affordances).
    pub current_user_id: &'a str,
    /// True while the list is scrolling — hover backgrounds are suppressed to
    /// avoid flicker (cf. `toggleDisableHover` in React).
    pub suppress_hover: bool,
    pub avatar_cache: Entity<LruImageCache>,
    /// Id of the first unread message; the "New messages" break is rendered
    /// immediately above it (React `UnreadMessageBreak`).
    pub unread_boundary_id: Option<SharedString>,
    /// Id of a message to briefly highlight after a jump-to-message (React jump
    /// highlight). The matching row gets a tinted background.
    pub highlight_id: Option<SharedString>,
}

/// Default display-name color when a member has no role color, matching the
/// React `DEFAULT_MESSAGE_CREATOR_NAME_DISPLAY_COLOR` (`#17ac86`).
pub const DEFAULT_DISPLAY_NAME_COLOR: u32 = 0x17_ac_86;
/// Reply quote username color (`#84ADFF` in React).
pub const REPLY_USERNAME_COLOR: u32 = 0x84_ad_ff;

/// Left inset of a message's content column: 16px avatar gutter + 40px avatar
/// + 16px gap = 72px (React `pl-[72px]`).
pub const CONTENT_INSET: f32 = 72.0;
/// Avatar diameter (React 40px).
pub const AVATAR_SIZE: f32 = 40.0;
/// Avatar left offset within the row (React `left-[16px]`).
pub const AVATAR_LEFT: f32 = 16.0;
/// Right padding reserved for the hover action bar (React `pr-12`).
pub const CONTENT_RIGHT_PAD: f32 = 48.0;
/// Reply strip left indent (React `ml-9`).
pub const REPLY_INSET: f32 = 36.0;
