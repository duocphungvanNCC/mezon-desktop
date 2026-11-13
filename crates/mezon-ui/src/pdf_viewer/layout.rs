use std::ops::Range;

use mezon_pdf::PdfPageSize;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PageAnchor {
    page: usize,
    fraction: f32,
}

pub(super) struct PageLayout {
    aspects: Vec<f32>,
    page_width: f32,
    padding: f32,
    gap: f32,
    tops: Vec<f32>,
    content_height: f32,
}

pub(super) fn page_aspect(page: &PdfPageSize) -> f32 {
    page.height.max(1.0) / page.width.max(1.0)
}

impl PageLayout {
    pub(super) fn new(aspects: Vec<f32>, page_width: f32, padding: f32, gap: f32) -> Self {
        let mut layout = Self {
            aspects,
            page_width: 0.0,
            padding,
            gap,
            tops: Vec::new(),
            content_height: 0.0,
        };
        layout.set_page_width(page_width);
        layout
    }

    pub(super) fn set_page_width(&mut self, page_width: f32) {
        self.page_width = page_width.max(1.0);
        self.tops.clear();
        let mut y = self.padding;
        for index in 0..self.aspects.len() {
            if index > 0 {
                y += self.gap;
            }
            self.tops.push(y);
            y += self.page_height(index);
        }
        self.content_height = y + self.padding;
    }

    pub(super) fn page_count(&self) -> usize {
        self.aspects.len()
    }

    pub(super) fn page_width(&self) -> f32 {
        self.page_width
    }

    pub(super) fn padding(&self) -> f32 {
        self.padding
    }

    pub(super) fn gap(&self) -> f32 {
        self.gap
    }

    pub(super) fn page_height(&self, index: usize) -> f32 {
        self.aspects
            .get(index)
            .map_or(0.0, |aspect| (self.page_width * aspect).max(1.0))
    }

    pub(super) fn page_top(&self, index: usize) -> f32 {
        self.tops
            .get(index)
            .copied()
            .unwrap_or(self.content_height - self.padding)
    }

    pub(super) fn content_width(&self) -> f32 {
        self.page_width + self.padding * 2.0
    }

    pub(super) fn content_height(&self) -> f32 {
        self.content_height
    }

    pub(super) fn max_scroll_top(&self, viewport_height: f32) -> f32 {
        (self.content_height - viewport_height.max(0.0)).max(0.0)
    }

    pub(super) fn clamp_scroll_top(&self, scroll_top: f32, viewport_height: f32) -> f32 {
        scroll_top.clamp(0.0, self.max_scroll_top(viewport_height))
    }

    pub(super) fn page_at(&self, y: f32) -> usize {
        self.tops
            .partition_point(|top| *top <= y)
            .saturating_sub(1)
            .min(self.page_count().saturating_sub(1))
    }

    pub(super) fn visible_range(&self, scroll_top: f32, viewport_height: f32) -> Range<usize> {
        let count = self.page_count();
        if count == 0 {
            return 0..0;
        }
        let mut start = self.page_at(scroll_top);
        if start + 1 < count && scroll_top >= self.page_top(start) + self.page_height(start) {
            start += 1;
        }
        let bottom = scroll_top + viewport_height.max(0.0);
        let end = self
            .tops
            .partition_point(|top| *top < bottom)
            .max(start + 1);
        start..end.min(count)
    }

    pub(super) fn current_page(&self, scroll_top: f32, viewport_height: f32) -> usize {
        let count = self.page_count();
        if count == 0 || scroll_top <= 0.0 {
            return 0;
        }
        let max = self.max_scroll_top(viewport_height);
        if max > 0.0 && scroll_top >= max - SCROLL_END_TOLERANCE {
            return count - 1;
        }
        self.page_at(scroll_top + viewport_height.max(0.0) / 2.0)
    }

    pub(super) fn scroll_top_for_page(&self, index: usize) -> f32 {
        let index = index.min(self.page_count().saturating_sub(1));
        (self.page_top(index) - self.padding).max(0.0)
    }

    pub(super) fn anchor_at(&self, scroll_top: f32) -> PageAnchor {
        let page = self.page_at(scroll_top);
        let height = self.page_height(page).max(1.0);
        let fraction = (scroll_top - self.page_top(page)) / height;
        PageAnchor { page, fraction }
    }

    pub(super) fn scroll_top_for_anchor(&self, anchor: PageAnchor) -> f32 {
        let page = anchor.page.min(self.page_count().saturating_sub(1));
        (self.page_top(page) + anchor.fraction * self.page_height(page)).max(0.0)
    }
}

const SCROLL_END_TOLERANCE: f32 = 0.5;

#[cfg(test)]
mod tests {
    use super::*;

    const PADDING: f32 = 16.0;
    const GAP: f32 = 16.0;

    fn letter_pages(count: usize) -> PageLayout {
        PageLayout::new(vec![1.25; count], 800.0, PADDING, GAP)
    }

    #[test]
    fn stacks_pages_with_padding_and_gap() {
        let layout = letter_pages(3);
        assert_eq!(layout.page_height(0), 1000.0);
        assert_eq!(layout.page_top(0), 16.0);
        assert_eq!(layout.page_top(1), 1032.0);
        assert_eq!(layout.page_top(2), 2048.0);
        assert_eq!(layout.content_height(), 3064.0);
        assert_eq!(layout.content_width(), 832.0);
    }

    #[test]
    fn mixed_page_sizes_keep_their_own_aspect() {
        let layout = PageLayout::new(vec![1.25, 0.5, 2.0], 400.0, PADDING, GAP);
        assert_eq!(layout.page_height(0), 500.0);
        assert_eq!(layout.page_height(1), 200.0);
        assert_eq!(layout.page_height(2), 800.0);
        assert_eq!(layout.page_top(2), 16.0 + 500.0 + 16.0 + 200.0 + 16.0);
    }

    #[test]
    fn page_aspect_tolerates_degenerate_sizes() {
        let aspect = page_aspect(&PdfPageSize {
            width: 0.0,
            height: 0.0,
        });
        assert_eq!(aspect, 1.0);
    }

    #[test]
    fn page_at_maps_gaps_to_the_page_above() {
        let layout = letter_pages(3);
        assert_eq!(layout.page_at(0.0), 0);
        assert_eq!(layout.page_at(1020.0), 0);
        assert_eq!(layout.page_at(1032.0), 1);
        assert_eq!(layout.page_at(1_000_000.0), 2);
    }

    #[test]
    fn visible_range_covers_partially_visible_pages() {
        let layout = letter_pages(5);
        assert_eq!(layout.visible_range(0.0, 600.0), 0..1);
        assert_eq!(layout.visible_range(900.0, 600.0), 0..2);
        assert_eq!(layout.visible_range(1020.0, 600.0), 1..2);
        assert_eq!(layout.visible_range(0.0, 10_000.0), 0..5);
    }

    #[test]
    fn visible_range_is_never_empty_for_unknown_viewport() {
        let layout = letter_pages(3);
        assert_eq!(layout.visible_range(1500.0, 0.0), 1..2);
        assert_eq!(layout.visible_range(1_000_000.0, 600.0), 2..3);
    }

    #[test]
    fn current_page_follows_viewport_center() {
        let layout = letter_pages(4);
        assert_eq!(layout.current_page(0.0, 600.0), 0);
        assert_eq!(layout.current_page(500.0, 600.0), 0);
        assert_eq!(layout.current_page(800.0, 600.0), 1);
    }

    #[test]
    fn current_page_reaches_short_last_page_at_scroll_end() {
        let layout = PageLayout::new(vec![1.25, 1.25, 0.1], 800.0, PADDING, GAP);
        let viewport = 900.0;
        let end = layout.max_scroll_top(viewport);
        assert_eq!(layout.current_page(end, viewport), 2);
        assert_eq!(layout.current_page(end - 50.0, viewport), 1);
    }

    #[test]
    fn content_shorter_than_viewport_stays_on_first_page() {
        let layout = letter_pages(2);
        assert_eq!(layout.max_scroll_top(10_000.0), 0.0);
        assert_eq!(layout.current_page(0.0, 10_000.0), 0);
        assert_eq!(layout.clamp_scroll_top(500.0, 10_000.0), 0.0);
    }

    #[test]
    fn scroll_top_for_page_aligns_page_under_padding() {
        let layout = letter_pages(3);
        assert_eq!(layout.scroll_top_for_page(0), 0.0);
        assert_eq!(layout.scroll_top_for_page(1), 1016.0);
        assert_eq!(
            layout.scroll_top_for_page(99),
            layout.scroll_top_for_page(2)
        );
    }

    #[test]
    fn navigating_to_page_makes_it_current() {
        let layout = letter_pages(6);
        let viewport = 700.0;
        for page in 0..layout.page_count() {
            let top = layout.clamp_scroll_top(layout.scroll_top_for_page(page), viewport);
            assert_eq!(layout.current_page(top, viewport), page);
        }
    }

    #[test]
    fn zoom_keeps_reading_position_through_anchor() {
        let mut layout = letter_pages(5);
        let scroll_top = layout.page_top(2) + 250.0;
        let anchor = layout.anchor_at(scroll_top);
        layout.set_page_width(1600.0);
        let restored = layout.scroll_top_for_anchor(anchor);
        assert_eq!(layout.page_at(restored), 2);
        assert_eq!(restored, layout.page_top(2) + 500.0);
    }

    #[test]
    fn empty_layout_is_safe() {
        let layout = PageLayout::new(Vec::new(), 800.0, PADDING, GAP);
        assert_eq!(layout.page_count(), 0);
        assert_eq!(layout.visible_range(0.0, 600.0), 0..0);
        assert_eq!(layout.current_page(100.0, 600.0), 0);
        assert_eq!(layout.scroll_top_for_page(3), 0.0);
    }
}
