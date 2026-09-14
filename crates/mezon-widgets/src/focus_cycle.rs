//! Tab / Shift-Tab movement between the fields of one form.
//!
//! GPUI ships a window-wide tab-stop map (`Window::focus_next`), but a mezon window keeps
//! background surfaces rendered behind a modal — the composer, a search box, an embed form in
//! the message list — so a window-wide cycle would hand focus to fields the user cannot see.
//! Each form instead declares its own ordered list of fields and Tab cycles inside that list.
//!
//! A form opts in with a single call on its container element:
//!
//! ```ignore
//! div().focus_cycle(vec![email.focus_handle(cx), password.focus_handle(cx)])
//! ```

use std::rc::Rc;

use gpui::{App, FocusHandle, InteractiveElement, KeyBinding, Window, actions};

/// Key context a form container declares to opt into tab navigation.
pub const FORM_KEY_CONTEXT: &str = "MezonForm";

actions!(mezon_form, [FocusNextField, FocusPrevField]);

pub fn init(cx: &mut App) {
    cx.bind_keys(form_bindings());
}

fn form_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("tab", FocusNextField, Some(FORM_KEY_CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevField, Some(FORM_KEY_CONTEXT)),
    ]
}

/// The field Tab lands on, wrapping at both ends. `current` is `None` when focus sits somewhere
/// else in the form (a button, the container itself) — Tab then enters at the matching end.
fn next_field(current: Option<usize>, len: usize, forward: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match (current, forward) {
        (Some(index), true) => (index + 1) % len,
        (Some(index), false) => (index + len - 1) % len,
        (None, true) => 0,
        (None, false) => len - 1,
    })
}

/// Move focus to the next (or previous) field of `fields`.
pub fn cycle_focus(fields: &[FocusHandle], forward: bool, window: &mut Window, cx: &mut App) {
    let current = fields.iter().position(|field| field.is_focused(window));
    let Some(next) = next_field(current, fields.len(), forward) else {
        return;
    };
    window.focus(&fields[next], cx);
}

/// Give a form container Tab / Shift-Tab movement across `fields`, in the order listed.
///
/// A form with fewer than two fields is left untouched: Tab would land back where it started,
/// and this runs from `render` — the embed-form caller renders inside `gpui::list`, so the
/// cheapest thing to do for the many surfaces with nothing to cycle is nothing at all.
pub trait FocusCycle: InteractiveElement + Sized {
    fn focus_cycle(self, fields: Vec<FocusHandle>) -> Self {
        if fields.len() < 2 {
            return self;
        }
        // One allocation shared by both listeners, instead of a `Vec` clone per listener.
        let forward: Rc<[FocusHandle]> = fields.into();
        let backward = forward.clone();
        self.key_context(FORM_KEY_CONTEXT)
            .on_action(move |_: &FocusNextField, window, cx| {
                cycle_focus(&forward, true, window, cx);
            })
            .on_action(move |_: &FocusPrevField, window, cx| {
                cycle_focus(&backward, false, window, cx);
            })
    }
}

impl<E: InteractiveElement> FocusCycle for E {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::InputState;
    use gpui::{
        Context, Entity, Focusable, KeyContext, Render, TestAppContext, VisualTestContext, div,
        prelude::*,
    };

    #[test]
    fn tab_walks_the_fields_and_wraps_at_the_end() {
        assert_eq!(next_field(Some(0), 3, true), Some(1));
        assert_eq!(next_field(Some(2), 3, true), Some(0));
        assert_eq!(next_field(Some(0), 3, false), Some(2));
        assert_eq!(next_field(Some(1), 3, false), Some(0));
    }

    #[test]
    fn tab_enters_the_form_at_the_matching_end() {
        assert_eq!(next_field(None, 3, true), Some(0));
        assert_eq!(next_field(None, 3, false), Some(2));
    }

    #[test]
    fn a_form_without_fields_stays_put() {
        assert_eq!(next_field(None, 0, true), None);
        assert_eq!(next_field(Some(0), 0, false), None);
    }

    #[test]
    fn a_single_field_form_keeps_focus_where_it_is() {
        assert_eq!(next_field(Some(0), 1, true), Some(0));
        assert_eq!(next_field(Some(0), 1, false), Some(0));
    }

    /// A form with two text fields, the shape every wired surface has.
    struct TestForm {
        first: Entity<InputState>,
        second: Entity<InputState>,
    }

    impl Render for TestForm {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .focus_cycle(vec![
                    self.first.focus_handle(cx),
                    self.second.focus_handle(cx),
                ])
                .child(self.first.clone())
                .child(self.second.clone())
        }
    }

    #[gpui::test]
    fn tab_walks_a_drawn_form_and_shift_tab_walks_back(cx: &mut TestAppContext) {
        cx.update(|cx| {
            mezon_theme::set_theme(mezon_theme::resolve_theme("dark"), cx);
            crate::text_actions::init(cx);
            init(cx);
        });

        let window = cx.add_window(|window, cx| TestForm {
            first: cx.new(|cx| InputState::new(window, cx)),
            second: cx.new(|cx| InputState::new(window, cx)),
        });
        let (first, second) = window
            .update(cx, |form, _, cx| {
                (form.first.focus_handle(cx), form.second.focus_handle(cx))
            })
            .unwrap();
        let cx = &mut VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();

        cx.update(|window, cx| window.focus(&first, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("tab");
        assert!(
            cx.update(|window, _| second.is_focused(window)),
            "tab must move focus to the second field"
        );

        cx.simulate_keystrokes("shift-tab");
        assert!(
            cx.update(|window, _| first.is_focused(window)),
            "shift-tab must move focus back to the first field"
        );

        // The last field wraps round to the first, the way a browser form does.
        cx.update(|window, cx| window.focus(&second, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("tab");
        assert!(
            cx.update(|window, _| first.is_focused(window)),
            "tab must wrap from the last field to the first"
        );
    }

    #[test]
    fn tab_only_fires_inside_a_form() {
        let mut inside = KeyContext::default();
        inside.add(FORM_KEY_CONTEXT);
        let outside = KeyContext::default();

        for binding in form_bindings() {
            let name = binding.action().name().to_string();
            let predicate = binding
                .predicate()
                .unwrap_or_else(|| panic!("{name} must be scoped to a context"));
            assert!(
                predicate.eval(std::slice::from_ref(&inside)),
                "{name} must fire inside {FORM_KEY_CONTEXT}"
            );
            assert!(
                !predicate.eval(std::slice::from_ref(&outside)),
                "{name} must not fire outside {FORM_KEY_CONTEXT}"
            );
        }
    }
}
