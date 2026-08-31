use std::cell::RefCell;
use std::collections::HashMap;

use gpui::{App, Bounds, Global, IntoElement, Pixels, Size, canvas, prelude::*};

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

fn area(bounds: Bounds<Pixels>) -> f32 {
    bounds.size.width.as_f32() * bounds.size.height.as_f32()
}

fn is_usable(bounds: Bounds<Pixels>, viewport: Size<Pixels>) -> bool {
    bounds.size.width > Pixels::ZERO
        && bounds.size.height > Pixels::ZERO
        && bounds.right() > Pixels::ZERO
        && bounds.bottom() > Pixels::ZERO
        && bounds.left() < viewport.width
        && bounds.top() < viewport.height
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
        (record.epoch == epoch).then_some(record.bounds)
    }

    fn record(cx: &App, anchor: TourAnchor, bounds: Bounds<Pixels>, viewport: Size<Pixels>) {
        let Some(this) = cx.try_global::<Self>() else {
            return;
        };
        if !this.probing || !is_usable(bounds, viewport) {
            return;
        }
        let epoch = this.epoch;
        let mut records = this.records.borrow_mut();
        if let Some(existing) = records.get(&anchor)
            && existing.epoch == epoch
            && area(existing.bounds) >= area(bounds)
        {
            return;
        }
        records.insert(anchor, AnchorRecord { bounds, epoch });
    }
}

pub fn probe(cx: &App, anchor: TourAnchor) -> Option<impl IntoElement + use<>> {
    if !TourAnchors::is_probing(cx) {
        return None;
    }
    Some(
        canvas(
            move |bounds, window, cx| {
                TourAnchors::record(cx, anchor, bounds, window.viewport_size())
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    )
}
