use gpui::{
    AnyElement, AnyView, App, Bounds, Context, Entity, FocusHandle, FontWeight, Global, Hsla,
    IntoElement, MouseButton, Pixels, Point, RenderOnce, SharedString, Size, Window, deferred, div,
    hsla, prelude::*, px, relative, size, svg,
};
use mezon_store::Settings;

use super::anchor::TourAnchors;
use super::overlay::{
    BUBBLE_GAP, BUBBLE_HEIGHT, BUBBLE_WIDTH, GLOW_INSET, RING_RADIUS, SCRIM_ALPHA, Side,
    VIEWPORT_MARGIN, bands, center_origin, hole_for, place,
};
use super::tracks::{TOUR_VERSION, TRACKS, TourTrack, core_track_for, track};
use crate::app::shell::Shell;
use crate::components::primitives::{Button, ButtonVariants, ToastKind, h_flex, v_flex};
use crate::router::Router;
use crate::theme::ActiveTheme;

const CARET: Pixels = px(12.);

gpui::actions!(mezon_tour, [TourNext, TourBack]);

pub struct TourStatus {
    pub resolving: bool,
    pub hole: Option<(f32, f32, f32, f32)>,
    pub track: &'static str,
    pub index: usize,
    pub position: usize,
    pub total: usize,
    pub title_key: &'static str,
    pub anchor: Option<String>,
    pub has_hole: bool,
}

enum Phase {
    Idle,
    Arming {
        track: &'static TourTrack,
        index: usize,
        forward: bool,
    },
    Showing {
        track: &'static TourTrack,
        index: usize,
        hole: Option<Bounds<Pixels>>,
    },
}

pub struct TourState {
    phase: Phase,
    context: Option<&'static str>,
    epoch: u64,
    restore_focus: Option<FocusHandle>,
    focus_handle: FocusHandle,
}

struct GlobalTourState(Entity<TourState>);
impl Global for GlobalTourState {}

impl TourState {
    pub fn init(cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx| Self {
            phase: Phase::Idle,
            context: None,
            epoch: 0,
            restore_focus: None,
            focus_handle: cx.focus_handle(),
        });
        cx.set_global(GlobalTourState(entity.clone()));
        entity
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalTourState>()
            .map(|this| this.0.clone())
    }

    pub fn is_active(&self) -> bool {
        !matches!(self.phase, Phase::Idle)
    }

    pub fn status(&self, cx: &App) -> Option<TourStatus> {
        if let Phase::Arming { track, index, .. } = &self.phase {
            return Some(TourStatus {
                resolving: true,
                hole: None,
                track: track.id,
                index: *index,
                position: 0,
                total: track.steps.len(),
                title_key: track.steps[*index].title_key,
                anchor: track.steps[*index].anchor.map(|a| format!("{a:?}")),
                has_hole: false,
            });
        }
        let Phase::Showing { track, index, hole } = &self.phase else {
            return None;
        };
        let step = &track.steps[*index];
        let visible = self.visible_steps(track, cx);
        let position = visible
            .iter()
            .position(|candidate| candidate == index)
            .map_or(1, |offset| offset + 1);
        Some(TourStatus {
            resolving: false,
            hole: hole.map(|h| {
                (
                    h.origin.x.as_f32(),
                    h.origin.y.as_f32(),
                    h.size.width.as_f32(),
                    h.size.height.as_f32(),
                )
            }),
            track: track.id,
            index: *index,
            position,
            total: visible.len().max(position),
            title_key: step.title_key,
            anchor: step.anchor.map(|anchor| format!("{anchor:?}")),
            has_hole: hole.is_some(),
        })
    }

    pub fn advance(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if forward {
            self.next(window, cx);
        } else {
            self.back(window, cx);
        }
    }

    pub fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel(window, cx);
    }

    pub fn start_track(id: &str, window: &mut Window, cx: &mut App) {
        let Some(entity) = Self::try_global(cx) else {
            return;
        };
        let Some(track) = track(id) else {
            return;
        };
        entity.update(cx, |this, cx| this.start(track, window, cx));
    }

    fn start(&mut self, track: &'static TourTrack, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_focus = window.focused(cx);
        self.context = current_context(cx);
        TourAnchors::set_probing(cx, true);
        self.epoch = TourAnchors::begin_epoch(cx);
        self.phase = Phase::Arming {
            track,
            index: 0,
            forward: true,
        };
        window.focus(&self.focus_handle, cx);
        window.refresh();
        cx.notify();
    }

    fn show(
        &mut self,
        track: &'static TourTrack,
        index: usize,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let visible = self.visible_steps(track, cx);
        let Some(landed) = land(&visible, index, forward) else {
            return false;
        };
        let hole = track.steps[landed].anchor.and_then(|anchor| {
            TourAnchors::live(cx, anchor, self.epoch)
                .map(|target| hole_for(target, window.viewport_size()))
        });
        self.phase = Phase::Showing {
            track,
            index: landed,
            hole,
        };
        cx.notify();
        true
    }

    fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Phase::Showing { track, index, .. } = &self.phase else {
            return;
        };
        let (track, index) = (*track, *index);
        if index + 1 >= track.steps.len() || !self.show(track, index + 1, true, window, cx) {
            self.finish(window, cx);
        }
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Phase::Showing { track, index, .. } = &self.phase else {
            return;
        };
        let (track, index) = (*track, *index);
        let Some(previous) = index.checked_sub(1) else {
            return;
        };
        self.show(track, previous, false, window, cx);
    }

    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let completed = match &self.phase {
            Phase::Arming { track, .. } | Phase::Showing { track, .. } => Some(track.id),
            Phase::Idle => None,
        };
        self.phase = Phase::Idle;
        self.context = None;
        TourAnchors::set_probing(cx, false);
        if let Some(focus) = self.restore_focus.take() {
            window.focus(&focus, cx);
        }
        if let Some(id) = completed {
            mark_seen(id, cx);
        }
        cx.notify();
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Idle) {
            return;
        }
        self.phase = Phase::Idle;
        self.context = None;
        TourAnchors::set_probing(cx, false);
        if let Some(focus) = self.restore_focus.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn visible_steps(&self, track: &'static TourTrack, cx: &App) -> Vec<usize> {
        let filtered = track
            .steps
            .iter()
            .enumerate()
            .filter(|(_, step)| match step.anchor {
                None => true,
                Some(anchor) => TourAnchors::live(cx, anchor, self.epoch).is_some(),
            })
            .map(|(index, _)| index)
            .collect();
        never_empty(filtered, track.steps.len())
    }

    fn resolve(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Phase::Arming {
            track,
            index,
            forward,
        } = &self.phase
        else {
            return;
        };
        let (track, index, forward) = (*track, *index, *forward);
        if self.show(track, index, forward, window, cx) {
            return;
        }
        self.finish(window, cx);
        let message = mezon_i18n::t(&locale(cx), "tour.empty").to_string();
        Shell::global(cx).update(cx, |shell, cx| shell.toast(ToastKind::Info, message, cx));
    }
}

fn never_empty(visible: Vec<usize>, step_count: usize) -> Vec<usize> {
    if visible.is_empty() {
        (0..step_count).collect()
    } else {
        visible
    }
}

fn land(visible: &[usize], index: usize, forward: bool) -> Option<usize> {
    if forward {
        visible
            .iter()
            .copied()
            .find(|candidate| *candidate >= index)
    } else {
        visible
            .iter()
            .copied()
            .rev()
            .find(|candidate| *candidate <= index)
    }
}

fn current_context(cx: &App) -> Option<&'static str> {
    let router = Router::global(cx);
    core_track_for(router.read(cx).route_ref()).map(|track| track.id)
}

fn locale(cx: &App) -> String {
    Settings::try_global(cx)
        .map(|settings| settings.read(cx).language.clone())
        .unwrap_or_else(|| "en".to_string())
}

fn mark_seen(track_id: &str, cx: &mut App) {
    let Some(settings) = Settings::try_global(cx) else {
        return;
    };
    let changed = settings.update(cx, |settings, cx| {
        let mut changed = false;
        if settings.tour_seen_version < TOUR_VERSION {
            settings.tour_seen_version = TOUR_VERSION;
            settings.tour_done_tracks.clear();
            changed = true;
        }
        if !settings.tour_done_tracks.iter().any(|id| id == track_id) {
            settings.tour_done_tracks.push(track_id.to_string());
            changed = true;
        }
        if changed {
            cx.notify();
        }
        changed
    });
    if changed {
        mezon_store::schedule_settings_save(&settings, cx);
    }
}

pub const ALWAYS_AUTO_START_IN_DEBUG: bool = false;

fn track_done(track_id: &str, cx: &App) -> bool {
    Settings::try_global(cx).is_some_and(|settings| {
        let settings = settings.read(cx);
        settings.tour_seen_version >= TOUR_VERSION
            && settings.tour_done_tracks.iter().any(|id| id == track_id)
    })
}

pub fn pending_core_track(cx: &App) -> Option<&'static str> {
    let router = Router::global(cx);
    let track = core_track_for(router.read(cx).route_ref())?;
    if !(cfg!(debug_assertions) && ALWAYS_AUTO_START_IN_DEBUG) && track_done(track.id, cx) {
        return None;
    }
    Some(track.id)
}

pub fn available_tracks(cx: &App) -> Vec<&'static TourTrack> {
    let router = Router::global(cx);
    let route = router.read(cx).route_ref();
    TRACKS
        .iter()
        .filter(|track| track.precondition.is_met(route))
        .collect()
}

pub fn auto_start_core(window: &mut Window, cx: &mut App) {
    let Some(id) = pending_core_track(cx) else {
        return;
    };
    if TourState::try_global(cx).is_some_and(|entity| entity.read(cx).is_active()) {
        return;
    }
    TourState::start_track(id, window, cx);
}

pub fn layer(cx: &App) -> Option<AnyView> {
    TourState::try_global(cx).map(AnyView::from)
}

#[derive(IntoElement)]
struct Scrim {
    hole: Option<Bounds<Pixels>>,
    viewport: Size<Pixels>,
    ring: Hsla,
}

impl RenderOnce for Scrim {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let dim = hsla(0., 0., 0., SCRIM_ALPHA);
        let ring = self.ring;

        let Some(hole) = self.hole else {
            return div().absolute().top_0().left_0().size_full().bg(dim);
        };

        let mut root = div().absolute().top_0().left_0().size_full();
        for band in bands(hole, self.viewport) {
            root = root.child(
                div()
                    .absolute()
                    .left(band.origin.x)
                    .top(band.origin.y)
                    .w(band.size.width)
                    .h(band.size.height)
                    .bg(dim),
            );
        }
        root.child(
            div()
                .absolute()
                .left(hole.origin.x - GLOW_INSET)
                .top(hole.origin.y - GLOW_INSET)
                .w(hole.size.width + GLOW_INSET * 2.)
                .h(hole.size.height + GLOW_INSET * 2.)
                .rounded(RING_RADIUS + GLOW_INSET)
                .border_1()
                .border_color(hsla(ring.h, ring.s, ring.l, 0.3)),
        )
        .child(
            div()
                .absolute()
                .left(hole.origin.x)
                .top(hole.origin.y)
                .w(hole.size.width)
                .h(hole.size.height)
                .rounded(RING_RADIUS)
                .border_2()
                .border_color(ring),
        )
    }
}

impl Render for TourState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !matches!(self.phase, Phase::Idle) && self.context != current_context(cx) {
            cx.defer_in(window, |this, window, cx| this.cancel(window, cx));
            return div();
        }

        if matches!(self.phase, Phase::Arming { .. }) {
            cx.defer_in(window, |this, window, cx| this.resolve(window, cx));
            return div();
        }

        let Phase::Showing { track, index, hole } = &self.phase else {
            return div();
        };
        let (track, index, hole) = (*track, *index, *hole);

        let viewport = window.viewport_size();
        let theme = cx.theme();
        let ring: Hsla = theme.tokens.bg_button_primary.into();
        let locale = locale(cx);
        let step = &track.steps[index];
        let visible = self.visible_steps(track, cx);
        let position = visible
            .iter()
            .position(|candidate| *candidate == index)
            .map_or(1, |offset| offset + 1);
        let total = visible.len().max(position);
        let is_last = position >= total;

        let (origin, side) = match hole {
            Some(hole) => place(hole, size(BUBBLE_WIDTH, BUBBLE_HEIGHT), viewport),
            None => (
                center_origin(size(BUBBLE_WIDTH, BUBBLE_HEIGHT), viewport),
                Side::Center,
            ),
        };

        let scrim = Scrim {
            hole,
            viewport,
            ring,
        };

        let bubble = v_flex()
            .absolute()
            .left(origin.x)
            .top(origin.y)
            .w(BUBBLE_WIDTH)
            .h(BUBBLE_HEIGHT)
            .p(px(16.))
            .gap(px(8.))
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.bg_floating)
            .shadow_lg()
            .occlude()
            .child(
                div()
                    .flex_none()
                    .w(px(56.))
                    .px(px(8.))
                    .py(px(2.))
                    .rounded_md()
                    .bg(theme.bg_hover)
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(format!("{position} / {total}")),
            )
            .child(
                div()
                    .flex_none()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text_primary)
                    .child(SharedString::from(mezon_i18n::t(&locale, step.title_key))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(theme.text_secondary)
                    .child(SharedString::from(mezon_i18n::t(&locale, step.body_key))),
            )
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .h(px(3.))
                    .rounded_full()
                    .bg(theme.bg_hover)
                    .child(
                        div()
                            .h_full()
                            .w(relative(position as f32 / total as f32))
                            .rounded_full()
                            .bg(ring),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("tour-skip")
                            .label(mezon_i18n::t(&locale, "tour.skip"))
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| this.finish(window, cx))),
                    )
                    .child(div().flex_1())
                    .when(position > 1, |el| {
                        el.child(
                            Button::new("tour-back")
                                .label(mezon_i18n::t(&locale, "tour.back"))
                                .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
                        )
                    })
                    .child(
                        Button::new("tour-next")
                            .label(mezon_i18n::t(
                                &locale,
                                if is_last { "tour.done" } else { "tour.next" },
                            ))
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| this.next(window, cx))),
                    ),
            );

        let caret = hole.and_then(|hole| caret_for(side, hole, origin, theme.bg_floating.into()));

        div().child(deferred(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .track_focus(&self.focus_handle)
                .key_context("tour")
                .on_action(
                    cx.listener(|this, _: &::menu::Cancel, window, cx| this.finish(window, cx)),
                )
                .on_action(cx.listener(|this, _: &TourNext, window, cx| this.next(window, cx)))
                .on_action(cx.listener(|this, _: &TourBack, window, cx| this.back(window, cx)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.next(window, cx)),
                )
                .child(scrim)
                .children(caret)
                .child(bubble),
        ))
    }
}

fn caret_for(
    side: Side,
    hole: Bounds<Pixels>,
    origin: Point<Pixels>,
    color: Hsla,
) -> Option<AnyElement> {
    let horizontal = (hole.center().x - CARET / 2.)
        .max(origin.x + BUBBLE_GAP)
        .min(origin.x + BUBBLE_WIDTH - BUBBLE_GAP - CARET);
    let vertical = (hole.center().y - CARET / 2.)
        .max(origin.y + VIEWPORT_MARGIN)
        .min(origin.y + BUBBLE_HEIGHT - VIEWPORT_MARGIN - CARET);

    let (path, left, top, width, height) = match side {
        Side::Below => (
            "icons/tour-caret-up.svg",
            horizontal,
            origin.y - CARET / 2.,
            CARET,
            CARET / 2.,
        ),
        Side::Above => (
            "icons/tour-caret-down.svg",
            horizontal,
            origin.y + BUBBLE_HEIGHT,
            CARET,
            CARET / 2.,
        ),
        Side::Right => (
            "icons/tour-caret-left.svg",
            origin.x - CARET / 2.,
            vertical,
            CARET / 2.,
            CARET,
        ),
        Side::Left => (
            "icons/tour-caret-right.svg",
            origin.x + BUBBLE_WIDTH,
            vertical,
            CARET / 2.,
            CARET,
        ),
        Side::Center => return None,
    };

    Some(
        svg()
            .absolute()
            .left(left)
            .top(top)
            .w(width)
            .h(height)
            .path(path)
            .text_color(color)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::{land, never_empty};

    #[test]
    fn filtering_every_step_away_falls_back_to_the_whole_track() {
        assert_eq!(never_empty(Vec::new(), 4), vec![0, 1, 2, 3]);
    }

    #[test]
    fn a_non_empty_filter_is_left_alone() {
        assert_eq!(never_empty(vec![1, 3], 4), vec![1, 3]);
    }

    #[test]
    fn an_empty_track_stays_empty() {
        assert!(never_empty(Vec::new(), 0).is_empty());
    }

    #[test]
    fn forward_lands_on_the_requested_step_when_it_is_visible() {
        assert_eq!(land(&[0, 1, 2, 3], 2, true), Some(2));
    }

    #[test]
    fn forward_skips_steps_whose_anchor_is_off_screen() {
        assert_eq!(land(&[0, 3, 5], 1, true), Some(3));
        assert_eq!(land(&[0, 3, 5], 4, true), Some(5));
    }

    #[test]
    fn forward_past_the_last_visible_step_ends_the_track() {
        assert_eq!(land(&[0, 3], 4, true), None);
    }

    #[test]
    fn back_skips_backwards_over_missing_steps() {
        assert_eq!(land(&[0, 3, 5], 4, false), Some(3));
        assert_eq!(land(&[2, 4], 3, false), Some(2));
    }

    #[test]
    fn back_before_the_first_visible_step_ends_the_track() {
        assert_eq!(land(&[2, 4], 1, false), None);
    }

    #[test]
    fn an_empty_visible_set_never_lands() {
        assert_eq!(land(&[], 0, true), None);
        assert_eq!(land(&[], 0, false), None);
    }
}
