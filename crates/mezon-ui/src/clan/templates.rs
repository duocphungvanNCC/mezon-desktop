use crate::components::primitives::IconName;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TemplateId {
    Gaming,
    Friends,
    StudyGroup,
    SchoolClub,
    LocalCommunity,
    ArtistsCreators,
}

pub(crate) struct TemplateEntry {
    pub(crate) id: TemplateId,
    pub(crate) icon: IconName,
    pub(crate) key: &'static str,
}

pub(crate) const TEMPLATES: &[TemplateEntry] = &[
    TemplateEntry {
        id: TemplateId::Gaming,
        icon: IconName::GamingConsoleIcon,
        key: "clan.clanTemplateModal.gamingTemplate",
    },
    TemplateEntry {
        id: TemplateId::Friends,
        icon: IconName::IconFriends,
        key: "clan.clanTemplateModal.friendsTemplate",
    },
    TemplateEntry {
        id: TemplateId::StudyGroup,
        icon: IconName::MemberList,
        key: "clan.clanTemplateModal.studyGroupTemplate",
    },
    TemplateEntry {
        id: TemplateId::SchoolClub,
        icon: IconName::School,
        key: "clan.clanTemplateModal.schoolClubTemplate",
    },
    TemplateEntry {
        id: TemplateId::LocalCommunity,
        icon: IconName::Community,
        key: "clan.clanTemplateModal.localCommunityTemplate",
    },
    TemplateEntry {
        id: TemplateId::ArtistsCreators,
        icon: IconName::PaintTray,
        key: "clan.clanTemplateModal.artistsCreatorsTemplate",
    },
];
