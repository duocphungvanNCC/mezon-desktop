use crate::app::shell::Shell;
use crate::components::primitives::{
    Button as GpuiButton, ButtonVariants, Icon, IconName, Input, InputEvent, InputState, h_flex,
    v_flex,
};
use crate::theme::ActiveTheme;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, FontWeight, SharedString, Subscription, Task,
    Window, div, prelude::*, px,
};
use mezon_store::AccountStore;

#[derive(Clone, Copy)]
enum PasswordValidationError {
    Characters,
    Uppercase,
    Lowercase,
    Number,
    Symbol,
}

fn validate_password(password: &str) -> Option<PasswordValidationError> {
    if password.chars().count() < 8 {
        Some(PasswordValidationError::Characters)
    } else if !password.chars().any(char::is_uppercase) {
        Some(PasswordValidationError::Uppercase)
    } else if !password.chars().any(char::is_lowercase) {
        Some(PasswordValidationError::Lowercase)
    } else if !password.chars().any(|c| c.is_ascii_digit()) {
        Some(PasswordValidationError::Number)
    } else if !password.chars().any(|c| !c.is_alphanumeric()) {
        Some(PasswordValidationError::Symbol)
    } else {
        None
    }
}

pub(super) struct PasswordModal {
    focus_handle: FocusHandle,
    locale: String,
    email: String,
    has_password: bool,
    current_input: Option<Entity<InputState>>,
    password_input: Entity<InputState>,
    confirm_input: Entity<InputState>,
    password_error: Option<PasswordValidationError>,
    confirm_mismatch: bool,
    current_error: bool,
    submitted: bool,
    saving: bool,
    _subscriptions: Vec<Subscription>,
    _save_task: Option<Task<()>>,
}

impl Focusable for PasswordModal {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl PasswordModal {
    pub(super) fn open(
        locale: String,
        email: String,
        has_password: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let modal = cx.new(|cx| Self::new(locale, email, has_password, window, cx));
        let focus = modal.read(cx).focus_handle.clone();
        Shell::global(cx).update(cx, |shell, cx| shell.show_modal(modal.into(), cx));
        window.focus(&focus, cx);
    }

    fn new(
        locale: String,
        email: String,
        has_password: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut make_password_input = |placeholder_key: &'static str, cx: &mut Context<Self>| {
            let placeholder: SharedString = mezon_i18n::t(&locale, placeholder_key).into();
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .height(px(52.))
            });
            input.update(cx, |state, cx| state.set_masked(true, window, cx));
            input
        };
        let current_input = has_password.then(|| {
            make_password_input(
                "accountSetting.setPasswordAccount.placeholder.currentPassword",
                cx,
            )
        });
        let password_input =
            make_password_input("accountSetting.setPasswordAccount.placeholder.password", cx);
        let confirm_input = make_password_input(
            "accountSetting.setPasswordAccount.placeholder.confirmPassword",
            cx,
        );

        let mut subscriptions = Vec::new();
        subscriptions.push(
            cx.subscribe(&password_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.revalidate(cx);
                }
            }),
        );
        subscriptions.push(
            cx.subscribe(&confirm_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.revalidate(cx);
                }
            }),
        );
        if let Some(input) = &current_input {
            subscriptions.push(cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.current_error = false;
                    cx.notify();
                }
            }));
        }

        Self {
            focus_handle: cx.focus_handle(),
            locale,
            email,
            has_password,
            current_input,
            password_input,
            confirm_input,
            password_error: None,
            confirm_mismatch: false,
            current_error: false,
            submitted: false,
            saving: false,
            _subscriptions: subscriptions,
            _save_task: None,
        }
    }

    fn revalidate(&mut self, cx: &mut Context<Self>) {
        let password = self.password_input.read(cx).value();
        let confirm = self.confirm_input.read(cx).value();
        self.password_error = (!password.is_empty() || self.submitted)
            .then(|| validate_password(password))
            .flatten();
        self.confirm_mismatch = (!confirm.is_empty() || self.submitted) && confirm != password;
        cx.notify();
    }

    fn can_save(&self, cx: &App) -> bool {
        let password = self.password_input.read(cx).value();
        let confirm = self.confirm_input.read(cx).value();
        let current_ok = self
            .current_input
            .as_ref()
            .is_none_or(|input| !input.read(cx).value().is_empty());
        !self.saving
            && !self.email.is_empty()
            && current_ok
            && !password.is_empty()
            && password == confirm
            && validate_password(password).is_none()
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.submitted = true;
        self.revalidate(cx);
        let same_password = self
            .current_input
            .as_ref()
            .is_some_and(|input| input.read(cx).value() == self.password_input.read(cx).value());
        if same_password {
            Shell::global(cx).update(cx, |shell, cx| {
                shell.error(
                    mezon_i18n::t(
                        &self.locale,
                        "accountSetting.setPasswordAccount.error.samePass",
                    ),
                    cx,
                )
            });
            return;
        }
        if !self.can_save(cx) {
            return;
        }

        let email = self.email.clone();
        let password = self.password_input.read(cx).value().to_string();
        let old_password = self
            .current_input
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        let has_password = self.has_password;
        let task = AccountStore::global(cx).update(cx, |store, cx| {
            store.save_password(email, password, old_password, cx)
        });
        self.saving = true;
        self._save_task = Some(cx.spawn(async move |this, cx| match task.await {
            Ok(()) => {
                let locale = this
                    .update(cx, |this, _| this.locale.clone())
                    .unwrap_or_default();
                cx.update(|cx| {
                    Shell::global(cx).update(cx, |shell, cx| {
                        shell.close_modal(cx);
                        shell.success(
                            mezon_i18n::t(
                                &locale,
                                "accountSetting.setPasswordAccount.toast.success",
                            ),
                            cx,
                        );
                    });
                })
            }
            Err(error) => {
                let state = this.update(cx, |this, cx| {
                    this.saving = false;
                    this.current_error = has_password && error.to_string().contains("code=3");
                    cx.notify();
                    (this.locale.clone(), this.current_error)
                });
                let Ok((locale, current_error)) = state else {
                    return;
                };
                cx.update(|cx| {
                    let key = if current_error {
                        "accountSetting.setPasswordAccount.error.incorrectCurrent"
                    } else if has_password {
                        "accountSetting.setPasswordAccount.error.updateFail"
                    } else {
                        "accountSetting.setPasswordAccount.error.createFail"
                    };
                    Shell::global(cx)
                        .update(cx, |shell, cx| shell.error(mezon_i18n::t(&locale, key), cx));
                });
            }
        }));
    }

    fn error_text(&self, error: PasswordValidationError) -> SharedString {
        let key = match error {
            PasswordValidationError::Characters => {
                "accountSetting.setPasswordAccount.error.characters"
            }
            PasswordValidationError::Uppercase => {
                "accountSetting.setPasswordAccount.error.uppercase"
            }
            PasswordValidationError::Lowercase => {
                "accountSetting.setPasswordAccount.error.lowercase"
            }
            PasswordValidationError::Number => "accountSetting.setPasswordAccount.error.number",
            PasswordValidationError::Symbol => "accountSetting.setPasswordAccount.error.symbol",
        };
        mezon_i18n::t(&self.locale, key).into()
    }
}

impl Render for PasswordModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let can_save = self.can_save(cx);
        let field =
            |label: SharedString, input: Entity<InputState>, error: Option<SharedString>| {
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text_secondary)
                            .child(label)
                            .child(div().text_color(theme.danger_text).child("*")),
                    )
                    .child(Input::new(&input).mask_toggle())
                    .when_some(error, |el, error| {
                        el.child(div().text_sm().text_color(theme.danger_text).child(error))
                    })
            };
        let current_error = self.current_error.then(|| {
            mezon_i18n::t(
                &self.locale,
                "accountSetting.setPasswordAccount.error.incorrectCurrent",
            )
            .into()
        });
        let password_error = self.password_error.map(|error| self.error_text(error));
        let confirm_error = self.confirm_mismatch.then(|| {
            mezon_i18n::t(
                &self.locale,
                "accountSetting.setPasswordAccount.error.notEqual",
            )
            .into()
        });

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("menu")
            .on_action(cx.listener(|_, _: &::menu::Cancel, _, cx| {
                Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
            }))
            .w(px(560.))
            .max_w(px(560.))
            .rounded_lg()
            .overflow_hidden()
            .bg(theme.tokens.theme_setting_primary)
            .shadow_lg()
            .child(
                v_flex()
                    .relative()
                    .gap_1()
                    .p_6()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text_primary)
                            .child(mezon_i18n::t(
                                &self.locale,
                                if self.has_password {
                                    "accountSetting.setPasswordAccount.changePassword"
                                } else {
                                    "accountSetting.setPasswordModal.title"
                                },
                            )),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.text_secondary)
                            .child(mezon_i18n::t(
                                &self.locale,
                                "accountSetting.setPasswordModal.description",
                            )),
                    )
                    .child(
                        div()
                            .id("password-modal-close")
                            .absolute()
                            .top_4()
                            .right_4()
                            .size(px(32.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(theme.text_primary)
                            .hover(|style| style.bg(theme.tokens.bg_item_hover))
                            .child(
                                Icon::new(IconName::Close)
                                    .size(px(20.))
                                    .text_color(theme.text_primary),
                            )
                            .on_click(|_, _, cx| {
                                Shell::global(cx).update(cx, |shell, cx| shell.close_modal(cx));
                            }),
                    ),
            )
            .child(
                v_flex()
                    .gap_4()
                    .p_6()
                    .child(
                        v_flex()
                            .gap_2()
                            .child(div().text_sm().text_color(theme.text_secondary).child(
                                mezon_i18n::t(
                                    &self.locale,
                                    "accountSetting.setPasswordAccount.email",
                                ),
                            ))
                            .child(
                                div()
                                    .h(px(52.))
                                    .px_4()
                                    .flex()
                                    .items_center()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(theme.border)
                                    .bg(theme.bg_secondary)
                                    .text_color(theme.text_secondary)
                                    .child(self.email.clone()),
                            ),
                    )
                    .when_some(self.current_input.clone(), |el, input| {
                        el.child(field(
                            mezon_i18n::t(
                                &self.locale,
                                "accountSetting.setPasswordAccount.currentPassword",
                            )
                            .into(),
                            input,
                            current_error,
                        ))
                    })
                    .child(field(
                        mezon_i18n::t(&self.locale, "accountSetting.setPasswordAccount.password")
                            .into(),
                        self.password_input.clone(),
                        password_error,
                    ))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.text_secondary)
                            .child(mezon_i18n::t(
                                &self.locale,
                                "accountSetting.setPasswordAccount.description",
                            )),
                    )
                    .child(field(
                        mezon_i18n::t(
                            &self.locale,
                            "accountSetting.setPasswordAccount.confirmPassword",
                        )
                        .into(),
                        self.confirm_input.clone(),
                        confirm_error,
                    )),
            )
            .child(
                div().p_6().pt_0().child(
                    GpuiButton::new("password-modal-save")
                        .label(mezon_i18n::t(
                            &self.locale,
                            if self.saving {
                                "accountSetting.setPasswordModal.loading"
                            } else {
                                "accountSetting.setPasswordAccount.confirm"
                            },
                        ))
                        .primary()
                        .disabled(!can_save)
                        .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_validation_matches_account_policy() {
        assert!(matches!(
            validate_password("Aa1!"),
            Some(PasswordValidationError::Characters)
        ));
        assert!(matches!(
            validate_password("lowercase1!"),
            Some(PasswordValidationError::Uppercase)
        ));
        assert!(matches!(
            validate_password("UPPERCASE1!"),
            Some(PasswordValidationError::Lowercase)
        ));
        assert!(matches!(
            validate_password("Password!"),
            Some(PasswordValidationError::Number)
        ));
        assert!(matches!(
            validate_password("Password1"),
            Some(PasswordValidationError::Symbol)
        ));
        assert!(validate_password("Password1!").is_none());
    }
}
