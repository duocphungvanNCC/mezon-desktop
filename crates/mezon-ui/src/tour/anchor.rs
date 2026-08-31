use std::cell::RefCell;
use std::collections::HashMap;

use gpui::{App, Bounds, Global, IntoElement, Pixels, canvas, prelude::*};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TourAnchor {
    ClanRail,
    ClanHeader,
    ChannelList,
    DirectList,
    UserInfoBar,
    ChannelHeaderTools,
    MessageTimeline,
    Composer,
    ComposerTools,
    MemberList,
    VoiceControls,
    ClanSettingsNav,
    ChannelHeaderSearch,
    CreateChannel,
    FriendsButton,
    FriendsPage,
    ClanMembersRow,
    AddFriendButton,
    ClanSettingsRow(&'static str),
}

#[derive(Debug, Clone, Copy)]
struct AnchorRecord {
    bounds: Bounds<Pixels>,
    epoch: u64,
}

#[derive(Default)]
pub struct TourAnchors {
    records: RefCell<HashMap<TourAnchor, AnchorRecord>>,
    epoch: u64,
    probing: bool,
}

impl Global for TourAnchors {}

impl TourAnchors {
    pub fn is_probing(cx: &App) -> bool {
        cx.try_global::<Self>().is_some_and(|this| this.probing)
    }

    pub fn set_probing(cx: &mut App, probing: bool) {
        let this = cx.default_global::<Self>();
        this.probing = probing;
        if !probing {
            this.records.borrow_mut().clear();
        }
    }

    pub fn begin_epoch(cx: &mut App) -> u64 {
        let this = cx.default_global::<Self>();
        this.epoch = this.epoch.wrapping_add(1);
        this.epoch
    }

    pub fn live(cx: &App, anchor: TourAnchor, epoch: u64) -> Option<Bounds<Pixels>> {
        let this = cx.try_global::<Self>()?;
        let records = this.records.borrow();
        let record = records.get(&anchor)?;
        (record.epoch == epoch && record.bounds.size.width > Pixels::ZERO).then_some(record.bounds)
    }

    fn record(cx: &App, anchor: TourAnchor, bounds: Bounds<Pixels>) {
        let Some(this) = cx.try_global::<Self>() else {
            return;
        };
        let epoch = this.epoch;
        this.records
            .borrow_mut()
            .insert(anchor, AnchorRecord { bounds, epoch });
    }
}

pub fn probe(cx: &App, anchor: TourAnchor) -> Option<impl IntoElement + use<>> {
    if !TourAnchors::is_probing(cx) {
        return None;
    }
    Some(
        canvas(
            move |bounds, _, cx| TourAnchors::record(cx, anchor, bounds),
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    )
}
