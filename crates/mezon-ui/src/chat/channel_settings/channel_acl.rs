use std::sync::Arc;

use gpui::{App, Global};
use mezon_client::AppApi;
use mezon_store::{RoleId, UserId};

struct GlobalChannelAclApi(Arc<AppApi>);

impl Global for GlobalChannelAclApi {}

pub fn init(api: Arc<AppApi>, cx: &mut App) {
    cx.set_global(GlobalChannelAclApi(api));
}

pub fn api(cx: &App) -> Option<Arc<AppApi>> {
    cx.try_global::<GlobalChannelAclApi>()
        .map(|global| global.0.clone())
}

pub fn channel_private_payload(private_enabled: bool) -> i32 {
    if private_enabled { 0 } else { 1 }
}

pub fn acl_user_ids(
    private_enabled: bool,
    selected: &[UserId],
    creator_id: Option<UserId>,
) -> Vec<i64> {
    let mut ids: Vec<i64> = Vec::new();
    if private_enabled {
        for user_id in selected {
            if user_id.is_zero() || ids.contains(&user_id.get()) {
                continue;
            }
            ids.push(user_id.get());
        }
    }
    if let Some(creator_id) = creator_id
        && !creator_id.is_zero()
        && !ids.contains(&creator_id.get())
    {
        ids.push(creator_id.get());
    }
    ids
}

pub fn acl_role_ids(private_enabled: bool, selected: &[RoleId]) -> Vec<i64> {
    if !private_enabled {
        return Vec::new();
    }
    let mut ids: Vec<i64> = Vec::new();
    for role_id in selected {
        if role_id.is_zero() || ids.contains(&role_id.get()) {
            continue;
        }
        ids.push(role_id.get());
    }
    ids
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    RolesAndMembers,
    MembersOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    pub mode: SearchMode,
    pub needle: String,
}

pub fn parse_search(input: &str) -> SearchQuery {
    let normalized = input.trim().to_lowercase();
    match normalized.strip_prefix('@') {
        Some(rest) => SearchQuery {
            mode: SearchMode::MembersOnly,
            needle: rest.to_string(),
        },
        None => SearchQuery {
            mode: SearchMode::RolesAndMembers,
            needle: normalized,
        },
    }
}

pub fn member_matches(clan_nick: &str, display_name: &str, username: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    clan_nick.to_lowercase().contains(needle)
        || display_name.to_lowercase().contains(needle)
        || username.to_lowercase().contains(needle)
}

pub fn role_matches(title: &str, needle: &str) -> bool {
    title.trim().to_lowercase().contains(needle.trim())
}

pub fn is_system_role(creator_id: UserId) -> bool {
    creator_id.is_zero()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_toggle_is_inverted_at_the_api_boundary() {
        assert_eq!(channel_private_payload(true), 0);
        assert_eq!(channel_private_payload(false), 1);
    }

    #[test]
    fn creator_is_force_merged_when_private() {
        let ids = acl_user_ids(true, &[UserId(2), UserId(3)], Some(UserId(7)));
        assert_eq!(ids, vec![2, 3, 7]);
    }

    #[test]
    fn creator_is_not_duplicated_when_already_selected() {
        let ids = acl_user_ids(true, &[UserId(7), UserId(2), UserId(7)], Some(UserId(7)));
        assert_eq!(ids, vec![7, 2]);
    }

    #[test]
    fn going_public_keeps_only_the_creator() {
        let ids = acl_user_ids(false, &[UserId(2), UserId(3)], Some(UserId(7)));
        assert_eq!(ids, vec![7]);
    }

    #[test]
    fn missing_creator_yields_empty_public_payload() {
        assert!(acl_user_ids(false, &[UserId(2)], None).is_empty());
        assert!(acl_user_ids(false, &[UserId(2)], Some(UserId(0))).is_empty());
    }

    #[test]
    fn roles_are_sent_only_while_private() {
        assert_eq!(
            acl_role_ids(true, &[RoleId(4), RoleId(4), RoleId(5)]),
            vec![4, 5]
        );
        assert!(acl_role_ids(false, &[RoleId(4)]).is_empty());
    }

    #[test]
    fn at_prefix_switches_to_members_only() {
        assert_eq!(
            parse_search("  @Wumpus "),
            SearchQuery {
                mode: SearchMode::MembersOnly,
                needle: "wumpus".to_string(),
            }
        );
        assert_eq!(
            parse_search("Mods"),
            SearchQuery {
                mode: SearchMode::RolesAndMembers,
                needle: "mods".to_string(),
            }
        );
        assert_eq!(
            parse_search("@"),
            SearchQuery {
                mode: SearchMode::MembersOnly,
                needle: String::new(),
            }
        );
    }

    #[test]
    fn member_match_covers_all_three_name_fields() {
        assert!(member_matches("Nick", "Display", "username", "nic"));
        assert!(member_matches("Nick", "Display", "username", "spla"));
        assert!(member_matches("Nick", "Display", "username", "sern"));
        assert!(!member_matches("Nick", "Display", "username", "zzz"));
        assert!(member_matches("", "", "", ""));
    }

    #[test]
    fn system_roles_are_excluded_from_candidates() {
        assert!(is_system_role(UserId(0)));
        assert!(!is_system_role(UserId(12)));
    }
}
