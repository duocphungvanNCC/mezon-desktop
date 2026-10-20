use std::ops::Range;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Context, FontWeight, ListState, Pixels, Window, canvas, div, point,
    prelude::*, px,
};

use super::{ClanRow, ClanSidebar};
use crate::theme::ActiveTheme;

const MENTION_SCROLL_DURATION: Duration = Duration::from_millis(300);

fn schedule_mention_frame(window: &Window, callback: impl FnOnce(&mut Window, &mut App) + 'static) {
    window.on_next_frame(callback);
}

pub(super) struct MentionScroll {
    start: Pixels,
    target: Pixels,
    applied: Pixels,
    started_at: Instant,
    viewport_height: Pixels,
}

impl MentionScroll {
    fn new(state: &ListState, ix: usize, now: Instant) -> Self {
        let offset = state.logical_scroll_top();
        let row_height = px(super::CLAN_ROW_HEIGHT);
        let viewport_height = state.viewport_bounds().size.height;
        let delta = row_height * (ix as f32 - offset.item_ix as f32) - offset.offset_in_item
            + row_height * 0.5
            - viewport_height * 0.5;
        let start = state.scroll_px_offset_for_scrollbar().y;
        let target = (start - delta).clamp(-state.max_offset_for_scrollbar().y, px(0.));
        Self {
            start,
            target,
            applied: start,
            started_at: now,
            viewport_height,
        }
    }

    fn sample(&self, now: Instant) -> (Pixels, bool) {
        let progress = (now.saturating_duration_since(self.started_at).as_secs_f32()
            / MENTION_SCROLL_DURATION.as_secs_f32())
        .min(1.);
        let eased = 1. - (1. - progress).powi(3);
        (
            self.start + (self.target - self.start) * eased,
            progress >= 1.,
        )
    }

    fn advance(&mut self, list: &ListState, now: Instant) -> bool {
        if list.scroll_px_offset_for_scrollbar().y != self.applied
            || list.viewport_bounds().size.height != self.viewport_height
        {
            return false;
        }
        let (position, finished) = self.sample(now);
        list.set_offset_from_scrollbar(point(px(0.), position));
        self.applied = list.scroll_px_offset_for_scrollbar().y;
        !finished
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) struct MentionTargets {
    above: Option<usize>,
    below: Option<usize>,
}

pub(super) fn mentioned_clan_indices(rows: &[ClanRow]) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| row.shows_mention_badge())
        .map(|(ix, _)| ix)
        .collect()
}

fn mention_targets(mentions: &[usize], visible: Range<usize>) -> MentionTargets {
    let above = mentions.partition_point(|ix| *ix < visible.start);
    let below = mentions.partition_point(|ix| *ix < visible.end);
    MentionTargets {
        above: above.checked_sub(1).map(|ix| mentions[ix]),
        below: mentions.get(below).copied(),
    }
}

impl ClanSidebar {
    fn current_mention_targets(&self) -> MentionTargets {
        list_mention_targets(&self.mention_indices, &self.list_state)
    }

    pub(super) fn mention_layout_observer(&self, cx: &Context<Self>) -> AnyElement {
        let view = cx.entity().downgrade();
        canvas(
            move |_, _, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                let targets = view.read(cx).current_mention_targets();
                if targets != view.read(cx).mention_targets {
                    cx.defer(move |cx| {
                        view.update(cx, |this, cx| {
                            let targets = this.current_mention_targets();
                            if targets != this.mention_targets {
                                this.mention_targets = targets;
                                cx.notify();
                            }
                        });
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
        .into_any_element()
    }

    pub(super) fn render_mention_indicators(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let targets = self.current_mention_targets();
        [true, false]
            .into_iter()
            .filter_map(|above| {
                let target = if above { targets.above } else { targets.below }?;
                self.rows
                    .get(target)
                    .filter(|row| row.shows_mention_badge())?;
                Some(
                    div()
                        .absolute()
                        .left_0()
                        .w_full()
                        .flex()
                        .justify_center()
                        .when(above, |el| el.top_0())
                        .when(!above, |el| el.bottom_0())
                        .child(self.render_mention_button(above, cx))
                        .into_any_element(),
                )
            })
            .collect()
    }

    fn render_mention_button(&self, above: bool, cx: &Context<Self>) -> AnyElement {
        let locale = &self.settings.read(cx).language;
        let label = mezon_i18n::t(locale, "common.newMessage").to_uppercase();
        div()
            .id(if above {
                "clan-mention-above"
            } else {
                "clan-mention-below"
            })
            .occlude()
            .max_w_full()
            .px_1p5()
            .py_0p5()
            .rounded_full()
            .bg(cx.theme().mention_badge)
            .text_color(gpui::white())
            .text_size(px(9.))
            .font_weight(FontWeight::BOLD)
            .shadow_lg()
            .cursor_pointer()
            .hover(|style| style.opacity(0.9))
            .child(div().min_w_0().truncate().child(label))
            .on_click(cx.listener(move |this, _, window, cx| {
                let targets = this.current_mention_targets();
                let target = if above { targets.above } else { targets.below };
                if let Some(ix) = target {
                    this.start_mention_scroll(ix, window, cx);
                }
            }))
            .into_any_element()
    }

    fn start_mention_scroll(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let position = self.list_state.scroll_px_offset_for_scrollbar();
        self.list_state.set_offset_from_scrollbar(position);
        self.mention_scroll = Some(MentionScroll::new(&self.list_state, ix, Instant::now()));
        self.schedule_mention_scroll(window, cx);
    }

    fn schedule_mention_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mention_scroll_queued {
            return;
        }
        self.mention_scroll_queued = true;
        let view = cx.entity().downgrade();
        schedule_mention_frame(window, move |window, cx| {
            let _ = view.update(cx, |this, cx| {
                this.mention_scroll_queued = false;
                this.advance_mention_scroll(window, cx);
            });
        });
    }

    fn advance_mention_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut animation) = self.mention_scroll.take() else {
            return;
        };
        if animation.advance(&self.list_state, Instant::now()) {
            self.mention_scroll = Some(animation);
            self.schedule_mention_scroll(window, cx);
        }
        cx.notify();
    }
}

fn list_mention_targets(mentions: &[usize], list: &ListState) -> MentionTargets {
    let viewport = list.viewport_bounds();
    if viewport.size.height <= px(0.) {
        return MentionTargets::default();
    }
    let mut visible = list.visible_range();
    if visible.len() > 1 {
        if let Some(bounds) = list.bounds_for_item(visible.start)
            && bounds.top() < viewport.top()
        {
            visible.start += 1;
        }
        if let Some(bounds) = list.bounds_for_item(visible.end - 1)
            && bounds.bottom() > viewport.bottom()
        {
            visible.end -= 1;
        }
    }
    mention_targets(mentions, visible)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestClanRail(ListState);

    impl gpui::Render for TestClanRail {
        fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::list(self.0.clone(), |_, _, _| {
                div()
                    .w_full()
                    .h(px(super::super::CLAN_ROW_HEIGHT))
                    .into_any_element()
            })
            .size_full()
        }
    }

    fn draw_rail(cx: &mut gpui::VisualTestContext, state: &ListState, height: f32) {
        let state = state.clone();
        cx.draw(
            gpui::point(px(0.), px(0.)),
            gpui::size(px(72.), px(height)),
            move |_, cx| cx.new(|_| TestClanRail(state)).into_any_element(),
        );
    }

    #[gpui::test]
    fn mention_scroll_moves_through_intermediate_positions_and_stops(
        cx: &mut gpui::TestAppContext,
    ) {
        let cx = cx.add_empty_window();
        let state = ListState::new(20, gpui::ListAlignment::Top, px(0.)).measure_all();
        draw_rail(cx, &state, 224.);
        let now = Instant::now();
        let start = state.scroll_px_offset_for_scrollbar().y;
        let mut animation = MentionScroll::new(&state, 15, now);
        assert_eq!(state.scroll_px_offset_for_scrollbar().y, start);
        assert!(animation.advance(&state, now + MENTION_SCROLL_DURATION / 2));
        let middle = state.scroll_px_offset_for_scrollbar().y;
        assert!(middle < start && middle > animation.target);
        draw_rail(cx, &state, 224.);
        assert!(!animation.advance(&state, now + MENTION_SCROLL_DURATION));
        assert_eq!(state.scroll_px_offset_for_scrollbar().y, animation.target);
        draw_rail(cx, &state, 224.);
        assert!(state.visible_range().contains(&15));
        let mut above = MentionScroll::new(&state, 1, now);
        assert!(above.advance(&state, now + MENTION_SCROLL_DURATION / 2));
        assert!(state.scroll_px_offset_for_scrollbar().y > middle);
        state.scroll_by(px(20.));
        let manual = state.scroll_px_offset_for_scrollbar().y;
        assert!(!above.advance(&state, now + MENTION_SCROLL_DURATION));
        assert_eq!(state.scroll_px_offset_for_scrollbar().y, manual);
        let mut resized = MentionScroll::new(&state, 15, now);
        draw_rail(cx, &state, 112.);
        let after_resize = state.scroll_px_offset_for_scrollbar().y;
        assert!(!resized.advance(&state, now + MENTION_SCROLL_DURATION / 2));
        assert_eq!(state.scroll_px_offset_for_scrollbar().y, after_resize);
    }

    fn row(id: i64, badge_count: u32, muted: bool) -> ClanRow {
        ClanRow {
            id: id.to_string().into(),
            id_num: mezon_store::ClanId(id),
            row_id: format!("clan-{id}").into(),
            group_name: format!("clan-group-{id}").into(),
            name: "Clan".into(),
            proxied_avatar_url: None,
            avatar_id: format!("clan-avatar-{id}").into(),
            badge_count,
            has_unread: true,
            muted,
            active: false,
        }
    }

    #[test]
    fn targets_follow_badges_mute_read_reorder_and_removal() {
        let mut rows = vec![row(1, 0, false), row(2, 2, true), row(3, 1, false)];
        assert_eq!(mentioned_clan_indices(&rows), vec![2]);
        rows[1].muted = false;
        assert_eq!(mentioned_clan_indices(&rows), vec![1, 2]);
        rows.swap(0, 2);
        assert_eq!(mentioned_clan_indices(&rows), vec![0, 1]);
        rows[0].badge_count = 0;
        assert_eq!(mentioned_clan_indices(&rows), vec![1]);
        rows.remove(1);
        assert!(mentioned_clan_indices(&rows).is_empty());
        rows[1].badge_count = 1;
        assert_eq!(mentioned_clan_indices(&rows), vec![1]);
        rows.clear();
        assert!(mentioned_clan_indices(&rows).is_empty());
    }

    #[test]
    fn mentions_outside_viewport_have_directional_targets() {
        assert_eq!(
            mention_targets(&[0, 2, 4, 6, 9], 3..6),
            MentionTargets {
                above: Some(2),
                below: Some(6)
            },
        );
    }

    #[test]
    fn visible_mentions_do_not_hide_mentions_further_down() {
        assert_eq!(
            mention_targets(&[2, 9], 1..4),
            MentionTargets {
                above: None,
                below: Some(9)
            },
        );
    }

    #[test]
    fn empty_or_fully_visible_lists_have_no_indicators() {
        assert_eq!(mention_targets(&[], 0..5), MentionTargets::default());
        assert_eq!(mention_targets(&[1, 2], 0..3), MentionTargets::default());
    }

    #[gpui::test]
    fn layout_resize_and_reveal_update_the_production_targets(cx: &mut gpui::TestAppContext) {
        use gpui::ListAlignment;

        let cx = cx.add_empty_window();
        let state = ListState::new(20, ListAlignment::Top, px(0.)).measure_all();
        assert_eq!(
            list_mention_targets(&[15], &state),
            MentionTargets::default()
        );

        draw_rail(cx, &state, 196.);
        assert_eq!(list_mention_targets(&[3], &state).below, Some(3));
        state.scroll_to(gpui::ListOffset {
            item_ix: 0,
            offset_in_item: px(28.),
        });
        draw_rail(cx, &state, 224.);
        assert_eq!(list_mention_targets(&[0], &state).above, Some(0));
        assert_eq!(list_mention_targets(&[1, 15], &state).below, Some(15));
        state.scroll_to_center_item(15);
        draw_rail(cx, &state, 224.);
        assert_eq!(list_mention_targets(&[1, 15], &state).below, None);
        assert_eq!(list_mention_targets(&[1, 15], &state).above, Some(1));
        assert_eq!(list_mention_targets(&[], &state), MentionTargets::default());
        state.scroll_to_reveal_item(0);
        draw_rail(cx, &state, 1120.);
        assert_eq!(
            list_mention_targets(&[1, 15], &state),
            MentionTargets::default()
        );
    }
}
