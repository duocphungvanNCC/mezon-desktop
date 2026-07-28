use gpui::{App, Hsla, SharedString, rgb};
use mezon_store::{ClanId, ProfileContext, RolesStore, UserId};

use crate::chat::message::DEFAULT_DISPLAY_NAME_COLOR;

pub const ROLE_FALLBACK_COLOR: u32 = 0x99_aa_b5;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoleStyleView {
    pub color: Option<Hsla>,
    pub icon: Option<SharedString>,
}

pub fn role_fallback_color() -> Hsla {
    Hsla::from(rgb(ROLE_FALLBACK_COLOR))
}

pub fn direct_name_color() -> Hsla {
    Hsla::from(rgb(DEFAULT_DISPLAY_NAME_COLOR))
}

pub fn role_scope(profile_context: Option<ProfileContext>) -> Option<ClanId> {
    match profile_context {
        Some(ProfileContext::Clan(clan_id)) => Some(clan_id),
        _ => None,
    }
}

pub fn role_style_view(clan_id: ClanId, user_id: UserId, cx: &App) -> RoleStyleView {
    let Some(store) = RolesStore::try_global(cx) else {
        return RoleStyleView::default();
    };
    let Some(style) = store.read(cx).role_style(clan_id, user_id) else {
        return RoleStyleView::default();
    };
    RoleStyleView {
        color: style.color.map(Hsla::from),
        icon: style.icon.clone(),
    }
}

pub fn role_color_in(roles: Option<&RolesStore>, clan_id: ClanId, user_id: UserId) -> Hsla {
    roles
        .and_then(|roles| roles.role_style(clan_id, user_id))
        .and_then(|style| style.color)
        .map(Hsla::from)
        .unwrap_or_else(role_fallback_color)
}

pub fn clan_role_color(clan_id: ClanId, user_id: UserId, cx: &App) -> Hsla {
    let roles = RolesStore::try_global(cx);
    let roles = roles.as_ref().map(|roles| roles.read(cx));
    role_color_in(roles, clan_id, user_id)
}

pub fn name_color(clan_scoped: bool, role_color: Option<Hsla>) -> Hsla {
    if clan_scoped {
        role_color.unwrap_or_else(role_fallback_color)
    } else {
        direct_name_color()
    }
}

pub fn message_sender_color(clan_id: Option<ClanId>, user_id: Option<UserId>, cx: &App) -> Hsla {
    match (clan_id, user_id) {
        (Some(clan_id), Some(user_id)) => clan_role_color(clan_id, user_id, cx),
        (Some(_), None) => role_fallback_color(),
        (None, _) => direct_name_color(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mezon_store::ChannelId;

    #[test]
    fn clan_scope_without_role_falls_back_to_default_role_color() {
        assert_eq!(name_color(true, None), role_fallback_color());
        assert_eq!(name_color(true, None), Hsla::from(rgb(0x99_aa_b5)));
    }

    #[test]
    fn direct_scope_ignores_role_color() {
        let role = Hsla::from(rgb(0xff_00_00));
        assert_eq!(name_color(false, Some(role)), direct_name_color());
        assert_eq!(name_color(false, None), Hsla::from(rgb(0x17_ac_86)));
    }

    #[test]
    fn clan_scope_prefers_role_color() {
        let role = Hsla::from(rgb(0xff_00_00));
        assert_eq!(name_color(true, Some(role)), role);
    }

    #[test]
    fn role_scope_only_matches_clan_context() {
        let clan_id = ClanId(7);
        assert_eq!(
            role_scope(Some(ProfileContext::Clan(clan_id))),
            Some(clan_id)
        );
        assert_eq!(role_scope(Some(ProfileContext::Direct(ChannelId(3)))), None);
        assert_eq!(role_scope(None), None);
    }
}
