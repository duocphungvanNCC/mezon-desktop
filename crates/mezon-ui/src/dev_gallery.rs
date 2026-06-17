use std::rc::Rc;

use crate::components::primitives::{Input, InputState, Select};
use gpui::{
    AnyElement, App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    ListAlignment, ListState, MouseButton, MouseDownEvent, ScrollHandle, SharedString, Window, div,
    hsla, img, list, prelude::*, px,
};
use ui::prelude::*;
use ui::{
    AnnouncementToast, Avatar, Banner, Callout, Checkbox, ContextMenu, Divider, DropdownMenu,
    Indicator, Modal, ModalFooter, ModalHeader, PopoverMenu, Section, Severity, Switch,
    ToggleState, Tooltip, right_click_menu,
};

struct PopoverContent {
    focus_handle: FocusHandle,
}

impl PopoverContent {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        cx.on_blur(&focus_handle, window, |_, _, cx| cx.emit(DismissEvent))
            .detach();
        Self { focus_handle }
    }
}

impl Focusable for PopoverContent {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DismissEvent> for PopoverContent {}

impl Render for PopoverContent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .key_context("menu")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|_, _: &::menu::Cancel, _window, cx| {
                cx.emit(DismissEvent);
            }))
            .on_mouse_down_out(cx.listener(|_, _: &MouseDownEvent, _window, cx| {
                cx.emit(DismissEvent);
            }))
            .elevation_2(cx)
            .p_3()
            .gap_1()
            .child(Label::new("Popover content"))
            .child(
                Label::new("Click outside or press Esc to close")
                    .color(Color::Muted)
                    .size(LabelSize::Small),
            )
    }
}

struct FakeMessage {
    author: SharedString,
    time: SharedString,
    text: SharedString,
    image: Option<SharedString>,
}

fn build_fake_messages(count: usize) -> Vec<FakeMessage> {
    let authors = ["Alice", "Bob", "Carol", "Dave", "Erin"];
    let texts = [
        "Hey, did you see the latest build?",
        "I pushed a fix for the rendering issue — the list is fully virtualized now and stays smooth even with thousands of items.",
        "👍 looks good to me",
        "Can you review my PR when you get a chance? It touches the theme bridge and a few of the gallery sections.",
        "Lunch in 10?",
        "The new design looks great — here's a screenshot of how attachments render inline.",
    ];
    (0..count)
        .map(|i| FakeMessage {
            author: authors[i % authors.len()].into(),
            time: format!("{:02}:{:02}", 9 + (i / 60) % 12, i % 60).into(),
            text: texts[(i * 3) % texts.len()].into(),
            image: (i % 4 == 0)
                .then(|| format!("https://picsum.photos/seed/mezon{i}/480/270").into()),
        })
        .collect()
}

fn render_fake_message(msg: &FakeMessage, cx: &App) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_start()
        .w_full()
        .px_3()
        .py_2()
        .gap_3()
        .child(
            div()
                .size_8()
                .flex_none()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(cx.theme().colors().element_background)
                .child(Label::new(
                    msg.author.chars().next().unwrap_or('?').to_string(),
                )),
        )
        .child(
            v_flex()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(Label::new(msg.author.clone()))
                        .child(
                            Label::new(msg.time.clone())
                                .color(Color::Muted)
                                .size(LabelSize::Small),
                        ),
                )
                .child(Label::new(msg.text.clone()))
                .when_some(msg.image.clone(), |this, url| {
                    this.child(img(url).w(px(360.)).h(px(200.)).rounded_md())
                }),
        )
        .into_any_element()
}

pub struct DevGallery {
    scroll: ScrollHandle,
    checkbox: ToggleState,
    switch: ToggleState,
    menu: Entity<ContextMenu>,
    modal_focus: FocusHandle,
    name_input: Entity<InputState>,
    email_input: Entity<InputState>,
    role: SharedString,
    role_menu: Entity<ContextMenu>,
    agree: ToggleState,
    form_message: Option<(bool, SharedString)>,
    select: Entity<Select>,
    msg_list: ListState,
    messages: Rc<Vec<FakeMessage>>,
    show_modal: bool,
    show_toast: bool,
}

impl DevGallery {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let menu = ContextMenu::build(window, cx, |menu, _window, _cx| {
            menu.header("Account")
                .entry("Profile", None, |_, _| {})
                .entry("Settings", None, |_, _| {})
                .submenu("More", |sub, _, _| {
                    sub.entry("Preferences", None, |_, _| {}).entry(
                        "Keyboard Shortcuts",
                        None,
                        |_, _| {},
                    )
                })
                .separator()
                .entry("Log out", None, |_, _| {})
        });

        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Your name"));
        let email_input = cx.new(|cx| InputState::new(window, cx).placeholder("you@example.com"));

        let this = cx.weak_entity();
        let role_menu = ContextMenu::build(window, cx, move |mut menu, _window, _cx| {
            for role in ["Admin", "Member", "Guest"] {
                let this = this.clone();
                menu = menu.entry(role, None, move |_window, cx| {
                    this.update(cx, |this, cx| {
                        this.role = role.into();
                        cx.notify();
                    })
                    .ok();
                });
            }
            menu
        });

        let messages = Rc::new(build_fake_messages(400));
        let msg_list = ListState::new(messages.len(), ListAlignment::Top, px(200.));

        let select = cx.new(|_cx| {
            Select::new(
                "demo-select",
                vec!["Admin".into(), "Member".into(), "Guest".into()],
            )
            .placeholder("Choose a role…")
        });

        Self {
            scroll: ScrollHandle::new(),
            checkbox: ToggleState::Selected,
            switch: ToggleState::Selected,
            menu,
            modal_focus: cx.focus_handle(),
            name_input,
            email_input,
            role: "Member".into(),
            role_menu,
            agree: ToggleState::Unselected,
            form_message: None,
            select,
            msg_list,
            messages,
            show_modal: false,
            show_toast: false,
        }
    }
}

fn section(title: &str, items: Vec<gpui::AnyElement>) -> impl IntoElement {
    v_flex()
        .gap_3()
        .pb_4()
        .child(Label::new(title.to_string()).size(LabelSize::Large))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_3()
                .children(items),
        )
}

impl Render for DevGallery {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme_buttons = [
            ("Dark", "dark"),
            ("Light", "light"),
            ("Purple", "purple"),
            ("Abyss", "abyss"),
            ("Red", "red_dark"),
        ]
        .into_iter()
        .map(|(label, name)| {
            Button::new(name, label)
                .style(ButtonStyle::Tinted(ui::TintColor::Accent))
                .on_click(cx.listener(move |_this, _, window, cx| {
                    crate::theme::set_theme(crate::theme::resolve_theme(name), cx);
                    window.refresh();
                    cx.notify();
                }))
                .into_any_element()
        })
        .collect::<Vec<_>>();
        let themes = section("Theme (switch to test components live)", theme_buttons);

        let buttons = section(
            "Button (Zed ui::Button)",
            vec![
                Button::new("b-filled", "Filled")
                    .style(ButtonStyle::Filled)
                    .on_click(|_, _, _| {})
                    .into_any_element(),
                Button::new("b-subtle", "Subtle")
                    .style(ButtonStyle::Subtle)
                    .on_click(|_, _, _| {})
                    .into_any_element(),
                Button::new("b-tinted", "Accent")
                    .style(ButtonStyle::Tinted(ui::TintColor::Accent))
                    .on_click(|_, _, _| {})
                    .into_any_element(),
                Button::new("b-transparent", "Transparent")
                    .style(ButtonStyle::Transparent)
                    .on_click(|_, _, _| {})
                    .into_any_element(),
                Button::new("b-disabled", "Disabled")
                    .style(ButtonStyle::Filled)
                    .disabled(true)
                    .on_click(|_, _, _| {})
                    .into_any_element(),
                Button::new("b-danger", "Danger")
                    .style(ButtonStyle::Tinted(ui::TintColor::Error))
                    .on_click(|_, _, _| {})
                    .into_any_element(),
            ],
        );

        let avatars = section(
            "Avatar (Zed ui::Avatar)",
            vec![
                Avatar::new("https://picsum.photos/seed/mezon-a1/64")
                    .size(px(28.))
                    .into_any_element(),
                Avatar::new("https://picsum.photos/seed/mezon-a2/64")
                    .size(px(40.))
                    .into_any_element(),
                Avatar::new("https://picsum.photos/seed/mezon-a3/64")
                    .size(px(40.))
                    .indicator(Indicator::dot().color(Color::Success))
                    .into_any_element(),
                Avatar::new("https://picsum.photos/seed/mezon-a4/64")
                    .size(px(40.))
                    .grayscale(true)
                    .into_any_element(),
            ],
        );

        let labels = section(
            "Label (Zed ui::Label)",
            vec![
                Label::new("Default").into_any_element(),
                Label::new("Muted").color(Color::Muted).into_any_element(),
                Label::new("Accent").color(Color::Accent).into_any_element(),
                Label::new("Error").color(Color::Error).into_any_element(),
                Label::new("Small")
                    .size(LabelSize::Small)
                    .into_any_element(),
            ],
        );

        let icons = section(
            "Icon (Zed ui::Icon)",
            vec![
                Icon::new(IconName::Check)
                    .color(Color::Success)
                    .into_any_element(),
                Icon::new(IconName::Close)
                    .color(Color::Error)
                    .into_any_element(),
                Icon::new(IconName::Plus).into_any_element(),
                Icon::new(IconName::Mic)
                    .color(Color::Accent)
                    .into_any_element(),
                Icon::new(IconName::Settings)
                    .color(Color::Muted)
                    .into_any_element(),
            ],
        );

        let toggles = section(
            "Checkbox / Switch (Zed ui::toggle)",
            vec![
                Checkbox::new("cb", self.checkbox)
                    .label("Enable notifications")
                    .on_click(cx.listener(|this, state: &ToggleState, _window, cx| {
                        this.checkbox = *state;
                        cx.notify();
                    }))
                    .into_any_element(),
                Switch::new("sw", self.switch)
                    .on_click(cx.listener(|this, state: &ToggleState, _window, cx| {
                        this.switch = *state;
                        cx.notify();
                    }))
                    .into_any_element(),
            ],
        );

        let dividers = section(
            "Divider (Zed ui::Divider)",
            vec![
                div()
                    .w(px(240.))
                    .child(Divider::horizontal())
                    .into_any_element(),
            ],
        );

        let tooltips = section(
            "Tooltip (Zed ui::Tooltip — hover)",
            vec![
                Button::new("tt-btn", "Hover me")
                    .style(ButtonStyle::Filled)
                    .tooltip(Tooltip::text("Saved automatically"))
                    .into_any_element(),
                div()
                    .id("tt-box")
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .child(Label::new("…or hover this box"))
                    .tooltip(Tooltip::text("Tooltips appear on hover"))
                    .into_any_element(),
            ],
        );

        let dropdown = section(
            "Dropdown w/ submenu (Zed ui::DropdownMenu + ContextMenu)",
            vec![DropdownMenu::new("dd-options", "Options", self.menu.clone()).into_any_element()],
        );

        let context_menu = section(
            "Right-click Context Menu w/ submenu (Zed ui::right_click_menu)",
            vec![
                right_click_menu::<ContextMenu>("ctx-demo")
                    .trigger(|_open, _window, cx| {
                        div()
                            .px_4()
                            .py_2()
                            .rounded_md()
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .child(Label::new("Right-click me"))
                    })
                    .menu(|window, cx| {
                        ContextMenu::build(window, cx, |menu, _, _| {
                            menu.entry("Cut", None, |_, _| {})
                                .entry("Copy", None, |_, _| {})
                                .entry("Paste", None, |_, _| {})
                                .separator()
                                .submenu("Share", |sub, _, _| {
                                    sub.entry("Copy Link", None, |_, _| {}).entry(
                                        "Email",
                                        None,
                                        |_, _| {},
                                    )
                                })
                        })
                    })
                    .into_any_element(),
            ],
        );

        let popover = section(
            "Popover (Zed ui::PopoverMenu — click-outside / Esc to close)",
            vec![
                PopoverMenu::new("popover-demo")
                    .trigger(Button::new("show-popover", "Show Popover").style(ButtonStyle::Filled))
                    .menu(|window, cx| Some(cx.new(|cx| PopoverContent::new(window, cx))))
                    .into_any_element(),
            ],
        );

        let modal = section(
            "Modal (Zed ui::Modal)",
            vec![
                Button::new("open-modal", "Open Modal")
                    .style(ButtonStyle::Filled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.show_modal = true;
                        window.focus(&this.modal_focus, cx);
                        cx.notify();
                    }))
                    .into_any_element(),
            ],
        );

        let toast = section(
            "Toast (Zed ui::AnnouncementToast)",
            vec![
                Button::new("show-toast", "Show Toast")
                    .style(ButtonStyle::Filled)
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.show_toast = true;
                        cx.notify();
                    }))
                    .into_any_element(),
            ],
        );

        let input_bg = cx.theme().colors().editor_background;
        let input_border = cx.theme().colors().border;
        let form = section(
            "Form Controls + Validation (mezon Input + Zed DropdownMenu / Checkbox)",
            vec![
                v_flex()
                    .w(px(360.))
                    .gap_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                Label::new("Name")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(
                                Input::new(&self.name_input)
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(input_border)
                                    .bg(input_bg),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                Label::new("Email")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(
                                Input::new(&self.email_input)
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(input_border)
                                    .bg(input_bg),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                Label::new("Role")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(DropdownMenu::new(
                                "role-select",
                                self.role.clone(),
                                self.role_menu.clone(),
                            )),
                    )
                    .child(
                        Checkbox::new("agree", self.agree)
                            .label("I accept the terms")
                            .on_click(cx.listener(|this, state: &ToggleState, _window, cx| {
                                this.agree = *state;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("submit-form", "Submit")
                            .style(ButtonStyle::Filled)
                            .on_click(cx.listener(|this, _, _window, cx| {
                                let name = this.name_input.read(cx).value().trim().to_string();
                                let email = this.email_input.read(cx).value().to_string();
                                let mut errors: Vec<&str> = Vec::new();
                                if name.is_empty() {
                                    errors.push("Name is required");
                                }
                                if !email.contains('@') {
                                    errors.push("A valid email is required");
                                }
                                if this.agree != ToggleState::Selected {
                                    errors.push("You must accept the terms");
                                }
                                this.form_message = Some(if errors.is_empty() {
                                    (true, "Form is valid — submitted!".into())
                                } else {
                                    (false, errors.join(" · ").into())
                                });
                                cx.notify();
                            })),
                    )
                    .when_some(self.form_message.clone(), |this, (ok, msg)| {
                        this.child(Label::new(msg).color(if ok {
                            Color::Success
                        } else {
                            Color::Error
                        }))
                    })
                    .into_any_element(),
            ],
        );

        let selected_value = self.select.read(cx).value().cloned();
        let select = section(
            "Select / Option (mezon::Select — self-contained base, emits SelectEvent)",
            vec![
                v_flex()
                    .w(px(260.))
                    .gap_2()
                    .child(self.select.clone())
                    .child(
                        Label::new(match &selected_value {
                            Some(v) => format!("Selected: {v}"),
                            None => "Nothing selected".to_string(),
                        })
                        .color(Color::Muted)
                        .size(LabelSize::Small),
                    )
                    .into_any_element(),
            ],
        );

        let status = v_flex()
            .gap_3()
            .pb_4()
            .child(
                Label::new("Status — Banner / Callout (Zed ui, Severity Success/Error)")
                    .size(LabelSize::Large),
            )
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(560.))
                    .gap_2()
                    .child(
                        Banner::new()
                            .severity(Severity::Success)
                            .child(Label::new("Changes saved successfully.")),
                    )
                    .child(
                        Banner::new()
                            .severity(Severity::Error)
                            .child(Label::new("Failed to save — please try again.")),
                    )
                    .child(
                        Banner::new()
                            .severity(Severity::Warning)
                            .child(Label::new("Your session will expire soon.")),
                    )
                    .child(
                        Banner::new()
                            .severity(Severity::Info)
                            .child(Label::new("A new update is available.")),
                    )
                    .child(
                        Callout::new()
                            .severity(Severity::Success)
                            .title("Upload complete")
                            .description("Your file has been uploaded and is ready to share."),
                    )
                    .child(
                        Callout::new()
                            .severity(Severity::Error)
                            .title("Connection lost")
                            .description(
                                "We couldn't reach the server. Check your network and retry.",
                            ),
                    ),
            );

        let list_border = cx.theme().colors().border;
        let msgs = self.messages.clone();
        let messages = v_flex()
            .gap_3()
            .pb_4()
            .child(
                Label::new(format!(
                    "Virtualized Message List (gpui::list + ListState — {} items, random content + images)",
                    self.messages.len()
                ))
                .size(LabelSize::Large),
            )
            .child(
                div()
                    .h(px(360.))
                    .w_full()
                    .max_w(px(560.))
                    .border_1()
                    .border_color(list_border)
                    .rounded_md()
                    .child(
                        list(self.msg_list.clone(), move |i, _window, cx| {
                            render_fake_message(&msgs[i], cx)
                        })
                        .size_full(),
                    ),
            );

        div()
            .size_full()
            .relative()
            .bg(cx.theme().colors().background)
            .text_color(cx.theme().colors().text)
            .child(
                div()
                    .id("dev-gallery-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        v_flex()
                            .p_8()
                            .gap_6()
                            .child(
                                Label::new("Mezon Component Gallery — verbatim Zed ui")
                                    .size(LabelSize::Large),
                            )
                            .child(themes)
                            .child(buttons)
                            .child(avatars)
                            .child(labels)
                            .child(icons)
                            .child(toggles)
                            .child(dividers)
                            .child(dropdown)
                            .child(context_menu)
                            .child(popover)
                            .child(tooltips)
                            .child(modal)
                            .child(toast)
                            .child(select)
                            .child(status)
                            .child(form)
                            .child(messages),
                    ),
            )
            .when(self.show_modal, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(hsla(0., 0., 0., 0.6))
                        .track_focus(&self.modal_focus)
                        .key_context("menu")
                        .on_action(cx.listener(|this, _: &::menu::Cancel, _window, cx| {
                            this.show_modal = false;
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                                this.show_modal = false;
                                cx.notify();
                            }),
                        )
                        .child(
                            div().w(px(440.)).elevation_3(cx).occlude().child(
                                Modal::new("demo-modal", None)
                                    .header(
                                        ModalHeader::new()
                                            .headline("Confirm action")
                                            .description("A Zed ui::Modal rendered in the gallery.")
                                            .show_dismiss_button(false)
                                            .show_back_button(false),
                                    )
                                    .section(
                                        Section::new()
                                            .child(Label::new("Modal body content goes here.")),
                                    )
                                    .footer(
                                        ModalFooter::new().end_slot(
                                            Button::new("close-modal", "Close")
                                                .style(ButtonStyle::Filled)
                                                .on_click(cx.listener(|this, _, _window, cx| {
                                                    this.show_modal = false;
                                                    cx.notify();
                                                })),
                                        ),
                                    ),
                            ),
                        ),
                )
            })
            .when(self.show_toast, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_col()
                        .justify_end()
                        .items_end()
                        .p_4()
                        .child(
                            div().w(px(360.)).occlude().child(
                                AnnouncementToast::new()
                                    .heading("Update available")
                                    .description("A new version of Mezon is ready to install.")
                                    .primary_action_label("Update now")
                                    .primary_on_click(|_, _, _| {})
                                    .secondary_action_label("Release notes")
                                    .secondary_on_click(|_, _, _| {})
                                    .dismiss_on_click(cx.listener(|this, _, _window, cx| {
                                        this.show_toast = false;
                                        cx.notify();
                                    })),
                            ),
                        ),
                )
            })
    }
}
