use gpui::{Context, ElementId, EventEmitter, SharedString, Window, prelude::*};

use super::dropdown::Dropdown;

pub enum SelectEvent {
    Change(usize),
}

pub struct Select {
    id: ElementId,
    items: Vec<SharedString>,
    selected: Option<usize>,
    open: bool,
    placeholder: SharedString,
}

impl EventEmitter<SelectEvent> for Select {}

impl Select {
    pub fn new(id: impl Into<ElementId>, items: Vec<SharedString>) -> Self {
        Self {
            id: id.into(),
            items,
            selected: None,
            open: false,
            placeholder: "Select…".into(),
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn value(&self) -> Option<&SharedString> {
        self.selected.and_then(|i| self.items.get(i))
    }

    pub fn set_selected(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        self.selected = index;
        cx.notify();
    }
}

impl Render for Select {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        Dropdown::new(self.id.clone())
            .items(self.items.clone())
            .selected(self.selected)
            .open(self.open)
            .placeholder(self.placeholder.clone())
            .on_toggle({
                let this = this.clone();
                move |_window, cx| {
                    this.update(cx, |select, cx| {
                        select.open = !select.open;
                        cx.notify();
                    })
                    .ok();
                }
            })
            .on_select({
                let this = this.clone();
                move |index, _window, cx| {
                    this.update(cx, |select, cx| {
                        select.selected = Some(index);
                        select.open = false;
                        cx.emit(SelectEvent::Change(index));
                        cx.notify();
                    })
                    .ok();
                }
            })
    }
}
