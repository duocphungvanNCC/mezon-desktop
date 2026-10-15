use std::collections::HashMap;

use gpui::App;
use mezon_store::{
    ChannelUserProfile, ClanId, ClanMember, ClanMembersStore, MENTION_SEARCH_MIN_CHARS,
    MentionSearchStore, UserId,
};

use super::permissions_tab::{MemberRow, member_row, member_row_from_clan_member};
use crate::util::text_utils::normalize_search_string;

pub(super) fn searchable(needle: &str) -> bool {
    needle.trim().chars().filter(|ch| !ch.is_control()).count() >= MENTION_SEARCH_MIN_CHARS
}

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

pub(super) fn answer_seq(clan_id: ClanId, needle: &str, cx: &App) -> Option<u64> {
    if needle.trim().is_empty() {
        return None;
    }
    let store = MentionSearchStore::try_global(cx)?;
    store
        .read(cx)
        .results(clan_id, None, needle)
        .map(|(_, seq, _)| seq)
}

pub(super) fn hits(clan_id: ClanId, needle: &str, cx: &App) -> Vec<ClanMember> {
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
        .cloned()
        .collect()
}

pub(super) fn profile(member: &ClanMember) -> ChannelUserProfile {
    ChannelUserProfile {
        username: member.user.username.clone(),
        display_name: member.user.display_name.clone(),
        avatar: member.user.avatar_url.clone(),
    }
}

pub(super) fn picker_row(
    clan_id: ClanId,
    user_id: UserId,
    remote: &HashMap<UserId, ClanMember>,
    cx: &App,
) -> MemberRow {
    let roster = ClanMembersStore::global(cx);
    let roster = roster.read(cx);
    known_row(roster.member(clan_id, user_id), remote.get(&user_id))
        .unwrap_or_else(|| member_row(clan_id, user_id, cx))
}

fn known_row(roster: Option<&ClanMember>, remote: Option<&ClanMember>) -> Option<MemberRow> {
    roster.or(remote).map(member_row_from_clan_member)
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
    use mezon_store::User;

    use super::*;

    fn member(nick: &str, display_name: &str, username: &str) -> ClanMember {
        ClanMember {
            user: User {
                id: UserId(7),
                username: username.into(),
                display_name: display_name.into(),
                avatar_url: "https://cdn/user.png".into(),
                ..User::default()
            },
            clan_nick: nick.into(),
            clan_avatar: "https://cdn/clan.png".into(),
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
    fn the_stored_profile_matches_what_the_channel_listing_returns() {
        let picked = member("Nick", "Display", "user");
        let stored = profile(&picked);
        assert_eq!(stored.display_name, "Display");
        assert_eq!(stored.username, "user");
        assert_eq!(stored.avatar, "https://cdn/user.png");
        assert_eq!(member_row_from_clan_member(&picked).name.as_ref(), "Nick");
    }

    #[test]
    fn the_roster_wins_over_a_remembered_server_row() {
        let roster = member("Renamed", "Display", "user");
        let remembered = member("Old Nick", "Display", "user");
        let row = known_row(Some(&roster), Some(&remembered)).expect("a row");
        assert_eq!(row.name.as_ref(), "Renamed");
        let row = known_row(None, Some(&remembered)).expect("a row");
        assert_eq!(row.name.as_ref(), "Old Nick");
        assert!(known_row(None, None).is_none());
    }

    #[test]
    fn a_single_character_is_not_searched() {
        assert!(!searchable(""));
        assert!(!searchable(" g "));
        assert!(searchable("gi"));
    }
}
