use std::time::{Duration, Instant};

use gpui::{
    App, ClipboardItem, Context, ElementId, Hsla, IntoElement, MouseButton, MouseDownEvent, Pixels,
    RenderOnce, SharedString, Window, div, prelude::*, px,
};

use crate::components::primitives::{Icon, IconName};

/// React resets the copied state after 5s (`MarkdownContent.tsx`, `TripleBackticks`).
const COPIED_DURATION: Duration = Duration::from_secs(5);

pub struct CopyButtonState {
    copied_at: Option<Instant>,
}

impl CopyButtonState {
    fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self { copied_at: None }
    }

    fn is_copied(&self) -> bool {
        self.copied_at
            .is_some_and(|at| at.elapsed() < COPIED_DURATION)
    }
}

/// Copy-to-clipboard icon button. React shows `CopyIcon`, swapping to `PasteIcon`
/// (a check mark) for five seconds after a copy.
#[derive(IntoElement)]
pub struct CopyButton {
    id: ElementId,
    text: SharedString,
    color: Hsla,
    hover_bg: Hsla,
    size: Pixels,
}

impl CopyButton {
    pub fn new(
        id: impl Into<ElementId>,
        text: impl Into<SharedString>,
        color: impl Into<Hsla>,
        hover_bg: impl Into<Hsla>,
    ) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            color: color.into(),
            hover_bg: hover_bg.into(),
            size: px(16.),
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for CopyButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, CopyButtonState::new);
        let icon = if state.read(cx).is_copied() {
            IconName::PasteIcon
        } else {
            IconName::CopyIcon
        };
        let hover_bg = self.hover_bg;
        let text = self.text;

        div()
            .id(self.id)
            .flex()
            .items_center()
            .justify_center()
            .p_1()
            .rounded(px(4.))
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg))
            // Without this the press starts a text selection drag over the block.
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
            })
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                state.update(cx, |state, cx| {
                    state.copied_at = Some(Instant::now());
                    cx.notify();
                });
                let state_id = state.entity_id();
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(COPIED_DURATION).await;
                    cx.update(|cx| cx.notify(state_id))
                })
                .detach();
            })
            .child(Icon::new(icon).size(self.size).text_color(self.color))
    }
}
