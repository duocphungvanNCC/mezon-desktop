use gpui::App;
use mezon_store::{ChannelUserProfile, ClanId, ClanMember, ClanMembersStore, MentionSearchStore};

use super::permissions_tab::{MemberRow, member_row_from_clan_member};
use crate::util::text_utils::normalize_search_string;

pub(super) fn search(owner: u64, clan_id: ClanId, needle: &str, cx: &mut App) {
    if !ClanMembersStore::global(cx).read(cx).roster_capped(clan_id) {
        return;
    }
    if let Some(store) = MentionSearchStore::try_global(cx) {
        store.update(cx, |store, cx| {
            store.search(owner, clan_id, None, needle, cx)
        });
    }
}

pub(super) fn end_session(owner: u64, cx: &mut App) {
    if let Some(store) = MentionSearchStore::try_global(cx) {
        store.update(cx, |store, cx| store.end_session(owner, cx));
    }
}

pub(super) fn is_searching(owner: u64, clan_id: ClanId, needle: &str, cx: &App) -> bool {
    MentionSearchStore::try_global(cx)
        .is_some_and(|store| store.read(cx).is_searching(owner, clan_id, None, needle))
}

pub(super) fn hits(clan_id: ClanId, needle: &str, cx: &App) -> Vec<MemberRow> {
    if needle.trim().is_empty() {
        return Vec::new();
    }
    let Some(store) = MentionSearchStore::try_global(cx) else {
        return Vec::new();
    };
    let Some((answered, _, members)) = store.read(cx).results(clan_id, None, needle) else {
        return Vec::new();
    };
    let folded = normalize_search_string(needle.trim());
    let answers_needle = normalize_search_string(answered) == folded;
    members
        .iter()
        .filter(|member| answers_needle || folded_match(member, &folded))
        .map(member_row_from_clan_member)
        .collect()
}

pub(super) fn profile(row: &MemberRow) -> ChannelUserProfile {
    ChannelUserProfile {
        username: row.username.to_string(),
        display_name: row.name.to_string(),
        avatar: row.avatar.to_string(),
    }
}

fn folded_match(member: &ClanMember, folded: &str) -> bool {
    [
        member.clan_nick.as_str(),
        member.user.display_name.as_str(),
        member.user.username.as_str(),
    ]
    .into_iter()
    .any(|field| normalize_search_string(field).contains(folded))
}

#[cfg(test)]
mod tests {
    use mezon_store::{User, UserId};

    use super::*;

    fn member(nick: &str, display_name: &str, username: &str) -> ClanMember {
        ClanMember {
            user: User {
                id: UserId(7),
                username: username.into(),
                display_name: display_name.into(),
                ..User::default()
            },
            clan_nick: nick.into(),
            ..ClanMember::default()
        }
    }

    #[test]
    fn a_reused_answer_is_narrowed_ignoring_case_and_accents() {
        let folded = normalize_search_string("chau gi");
        assert!(folded_match(&member("", "Châu Giang", "cg"), &folded));
        assert!(folded_match(&member("CHAU GIANG", "", "cg"), &folded));
        assert!(!folded_match(&member("", "Châu Ngọc", "chau"), &folded));
    }

    #[test]
    fn the_listed_name_follows_the_roster_order() {
        let row = member_row_from_clan_member(&member("Nick", "Display", "user"));
        assert_eq!(row.name.as_ref(), "Nick");
        assert_eq!(profile(&row).display_name, "Nick");
        assert_eq!(profile(&row).username, "user");
    }
}
