use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    Bounds, Context, Div, Modifiers, MouseButton, MouseDownEvent, Pixels, Point, Render, Stateful,
    TestAppContext, Window, canvas, deferred, div, point, prelude::*, px, size,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Gifs,
    Stickers,
}

struct Probe {
    open: Option<Tab>,
    toggle_bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl Probe {
    fn new() -> Self {
        Self {
            open: None,
            toggle_bounds: Rc::new(Cell::new(Bounds::default())),
        }
    }

    fn toggle(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.open = match self.open {
            Some(open) if open == tab => None,
            _ => Some(tab),
        };
        cx.notify();
    }

    fn button(&self, id: &'static str, tab: Tab, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(id)
            .size(px(20.))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(tab, cx)))
    }
}

impl Render for Probe {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let toggle_bounds = self.toggle_bounds.clone();
        let popup = self.open.map(|_| {
            deferred(
                div()
                    .absolute()
                    .bottom_full()
                    .right_0()
                    .mb(px(10.))
                    .occlude()
                    .on_mouse_down_out(cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        if toggle_bounds.get().contains(&event.position) {
                            return;
                        }
                        this.open = None;
                        cx.notify();
                    }))
                    .w(px(300.))
                    .h(px(200.)),
            )
        });

        div().size_full().flex().flex_col().justify_end().child(
            div()
                .relative()
                .w_full()
                .h(px(60.))
                .child(
                    div()
                        .absolute()
                        .right(px(12.))
                        .top(px(20.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            canvas(
                                {
                                    let toggle_bounds = self.toggle_bounds.clone();
                                    move |bounds, _, _| toggle_bounds.set(bounds)
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .size_full(),
                        )
                        .child(self.button("gif-toggle", Tab::Gifs, cx))
                        .child(self.button("sticker-toggle", Tab::Stickers, cx)),
                )
                .when_some(popup, |this, popup| this.child(popup)),
        )
    }
}

const GIF_BUTTON: Point<Pixels> = Point {
    x: px(450.),
    y: px(370.),
};
const STICKER_BUTTON: Point<Pixels> = Point {
    x: px(478.),
    y: px(370.),
};

fn click(cx: &mut gpui::VisualTestContext, at: Point<Pixels>) {
    cx.simulate_mouse_move(at, None, Modifiers::none());
    cx.run_until_parked();
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
}

fn open_probe(cx: &mut TestAppContext) -> (gpui::Entity<Probe>, &mut gpui::VisualTestContext) {
    let (probe, cx) = cx.add_window_view(|_, _| Probe::new());
    cx.simulate_resize(size(px(500.), px(400.)));
    cx.run_until_parked();
    (probe, cx)
}

#[gpui::test]
fn pressing_the_same_button_twice_closes_the_popup(cx: &mut TestAppContext) {
    let (probe, cx) = open_probe(cx);

    click(cx, GIF_BUTTON);
    probe.update(cx, |probe, _| {
        assert_eq!(probe.open, Some(Tab::Gifs), "first click opens the popup");
    });

    click(cx, GIF_BUTTON);
    probe.update(cx, |probe, _| {
        assert_eq!(
            probe.open, None,
            "the outside-dismiss must not swallow a press on the toggle itself, \
             otherwise the click reopens what it just closed"
        );
    });
}

#[gpui::test]
fn pressing_another_button_switches_tab_and_stays_open(cx: &mut TestAppContext) {
    let (probe, cx) = open_probe(cx);

    click(cx, GIF_BUTTON);
    click(cx, STICKER_BUTTON);
    probe.update(cx, |probe, _| {
        assert_eq!(
            probe.open,
            Some(Tab::Stickers),
            "clicking a different toggle switches tabs and stays open"
        );
    });
}

#[gpui::test]
fn pressing_outside_dismisses_the_popup(cx: &mut TestAppContext) {
    let (probe, cx) = open_probe(cx);

    click(cx, GIF_BUTTON);
    cx.simulate_mouse_down(
        point(px(40.), px(40.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.run_until_parked();
    probe.update(cx, |probe, _| {
        assert_eq!(
            probe.open, None,
            "a press outside still dismisses the popup"
        );
    });
}
