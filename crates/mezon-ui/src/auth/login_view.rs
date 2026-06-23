use mezon_store::LoginMethod;

use std::sync::Arc;

use crate::components::primitives::{Button, ButtonVariants as _, Spinner};
use gpui::{App, Context, Entity, FontWeight, MouseButton, Task, Window, div, prelude::*};
use mezon_store::{AuthState, LoginStore, Session, Settings};

use crate::components::compositions::{FormField, OtpInput};
use crate::theme::ActiveTheme;

pub struct LoginView {
    auth_state: Entity<AuthState>,
    settings: Entity<Settings>,

    method: LoginMethod,

    otp_step: u8,
    otp_req_id: String,
    otp_email: String,

    email_field: Option<Entity<FormField>>,
    password_field: Option<Entity<FormField>>,
    otp_input: Option<Entity<OtpInput>>,

    loading: bool,
    error: Option<String>,
    countdown: u32,

    qr_login_id: Option<String>,
    qr_expired: bool,
    _qr_poll_task: Option<Task<()>>,
}

impl LoginView {
    pub fn new(
        auth_state: Entity<AuthState>,
        settings: Entity<Settings>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        let qr_auth_state = auth_state.clone();
        let qr_auth_state_for_observe = qr_auth_state.clone();
        cx.observe(&auth_state, move |this, auth, cx| {
            match auth.read(cx) {
                AuthState::NotAuthenticated | AuthState::OtpRequested { .. } => {
                    if this._qr_poll_task.is_none() {
                        this._qr_poll_task =
                            Some(Self::start_qr_flow(qr_auth_state_for_observe.clone(), cx));
                    }
                }
                _ => {
                    this._qr_poll_task = None;
                }
            }
            cx.notify();
        })
        .detach();
        let mut view = Self {
            auth_state,
            settings,
            method: LoginMethod::Otp,
            otp_step: 0,
            otp_req_id: String::new(),
            otp_email: String::new(),
            email_field: None,
            password_field: None,
            otp_input: None,
            loading: false,
            error: None,
            countdown: 0,
            qr_login_id: None,
            qr_expired: false,
            _qr_poll_task: None,
        };
        if matches!(
            view.auth_state.read(cx),
            AuthState::NotAuthenticated | AuthState::OtpRequested { .. }
        ) {
            view._qr_poll_task = Some(Self::start_qr_flow(qr_auth_state.clone(), cx));
        }
        view
    }

    fn ensure_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.email_field.is_none() {
            let label = mezon_i18n::t(&self.settings.read(cx).language, "login.email");
            self.email_field = Some(cx.new(|cx| FormField::new(window, cx, label)));
        }

        if self.password_field.is_none() {
            let label = mezon_i18n::t(&self.settings.read(cx).language, "login.password");
            self.password_field = Some(cx.new(|cx| {
                let field = FormField::new(window, cx, label);
                field.set_masked(window, cx);
                field
            }));
        }

        if self.otp_input.is_none() {
            let entity = cx.entity().clone();
            self.otp_input = Some(cx.new(|cx| {
                OtpInput::new(window, cx, 6).on_complete(Arc::new(move |code, _window, cx| {
                    Self::handle_confirm_otp(&entity, code, cx);
                }))
            }));
        }
    }

    fn start_qr_flow(auth_state: Entity<AuthState>, cx: &mut Context<LoginView>) -> Task<()> {
        let client = LoginStore::global(cx).read(cx).client();
        cx.spawn(async move |this, cx| {
            let qr_result = client.create_qr_login().await;
            let login_id = match qr_result {
                Ok(qr) => qr.login_id,
                Err(e) => {
                    tracing::warn!("QR login create failed: {e}");
                    let _ = this.update(cx, |this, cx| {
                        this.qr_expired = true;
                        cx.notify();
                    });
                    return;
                }
            };
            let _ = this.update(cx, |this, cx| {
                this.qr_login_id = Some(login_id.clone());
                cx.notify();
            });

            let exec = cx.background_executor().clone();
            let mut elapsed: u32 = 0;
            loop {
                exec.timer(std::time::Duration::from_secs(2)).await;
                elapsed += 2;
                if elapsed >= 60 {
                    let _ = this.update(cx, |this, cx| {
                        this.qr_expired = true;
                        cx.notify();
                    });
                    break;
                }
                let result = client.confirm_qr_login(&login_id).await;
                if let Ok(session) = result {
                    let _ = this.update(cx, |_this, cx| {
                        Self::on_auth_success(session, &auth_state, cx);
                    });
                    break;
                }
            }
        })
    }

    fn handle_send_otp(entity: &Entity<LoginView>, _window: &mut Window, cx: &mut App) {
        let email = entity
            .read(cx)
            .email_field
            .as_ref()
            .map(|field| field.read(cx).value(cx))
            .unwrap_or_default();
        if email.trim().is_empty() {
            entity.update(cx, |this, cx| {
                let locale = this.settings.read(cx).language.clone();
                this.error = Some(mezon_i18n::t(&locale, "login.errors.emailRequired").to_string());
                cx.notify();
            });
            return;
        }

        entity.update(cx, |this, cx| {
            this.loading = true;
            this.error = None;
            cx.notify();
        });

        let client = LoginStore::global(cx).read(cx).client();
        let email_clone = email.clone();
        let entity_clone = entity.clone();

        cx.spawn(async move |cx| {
            let result = client.request_otp(&email_clone).await;
            entity_clone.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(req_id) => {
                        this.otp_req_id = req_id.clone();
                        this.otp_email = email_clone.clone();
                        this.otp_step = 1;
                        this.countdown = 60;
                        this.error = None;
                        this.auth_state.update(cx, |state, cx| {
                            *state = AuthState::OtpRequested {
                                req_id,
                                email: email_clone,
                            };
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        this.error = Some(format!("{e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();

        Self::start_countdown(entity, cx);
    }

    fn handle_confirm_otp(entity: &Entity<LoginView>, otp_code: String, cx: &mut App) {
        if entity.read(cx).loading {
            return;
        }

        let req_id = entity.read(cx).otp_req_id.clone();

        entity.update(cx, |this, cx| {
            this.loading = true;
            this.error = None;
            cx.notify();
        });

        let client = LoginStore::global(cx).read(cx).client();
        let auth_state = entity.read(cx).auth_state.clone();
        let entity_clone = entity.clone();

        cx.spawn(async move |cx| {
            let result = client.confirm_otp(&req_id, &otp_code).await;
            entity_clone.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(session) => {
                        Self::on_auth_success(session, &auth_state, cx);
                    }
                    Err(e) => {
                        this.error = Some(format!("{e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn handle_sign_in(entity: &Entity<LoginView>, cx: &mut App) {
        let (email, password) = {
            let this = entity.read(cx);
            (
                this.email_field
                    .as_ref()
                    .map(|field| field.read(cx).value(cx))
                    .unwrap_or_default(),
                this.password_field
                    .as_ref()
                    .map(|field| field.read(cx).value(cx))
                    .unwrap_or_default(),
            )
        };

        if email.trim().is_empty() || password.is_empty() {
            entity.update(cx, |this, cx| {
                let locale = this.settings.read(cx).language.clone();
                this.error =
                    Some(mezon_i18n::t(&locale, "login.errors.emailPasswordRequired").to_string());
                cx.notify();
            });
            return;
        }

        entity.update(cx, |this, cx| {
            this.loading = true;
            this.error = None;
            cx.notify();
        });

        let client = LoginStore::global(cx).read(cx).client();
        let auth_state = entity.read(cx).auth_state.clone();
        let entity_clone = entity.clone();

        cx.spawn(async move |cx| {
            let result = client.authenticate_email(&email, &password).await;
            entity_clone.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(session) => {
                        Self::on_auth_success(session, &auth_state, cx);
                    }
                    Err(e) => {
                        this.error = Some(format!("{e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn on_auth_success(session: Session, auth_state: &Entity<AuthState>, cx: &mut App) {
        tracing::info!("Authentication successful");
        tracing::debug!("  User ID: {}", session.user_id);
        tracing::debug!("  Username: {}", session.username);

        let session_for_keychain = session.clone();
        cx.background_executor()
            .spawn(async move {
                if let Err(e) = LoginStore::persist_session(&session_for_keychain) {
                    tracing::warn!("Failed to save session to keychain: {e}");
                }
            })
            .detach();

        auth_state.update(cx, |state, cx| {
            *state = AuthState::Connecting(session);
            tracing::debug!("User authenticated, connecting transport.");
            cx.notify();
        });
    }

    fn start_countdown(entity: &Entity<LoginView>, cx: &mut App) {
        let entity_clone = entity.clone();
        cx.spawn(async move |cx| {
            let exec = cx.background_executor().clone();
            loop {
                exec.timer(std::time::Duration::from_secs(1)).await;
                let should_stop = entity_clone.update(cx, |this, cx| {
                    if this.countdown > 0 {
                        this.countdown -= 1;
                        cx.notify();
                    }
                    this.countdown == 0
                });
                if should_stop {
                    break;
                }
            }
        })
        .detach();
    }

    fn reload_qr(entity: &Entity<LoginView>, cx: &mut App) {
        entity.update(cx, |this, cx| {
            this.qr_login_id = None;
            this.qr_expired = false;
            let auth_state = this.auth_state.clone();
            this._qr_poll_task = Some(Self::start_qr_flow(auth_state, cx));
            cx.notify();
        });
    }
}

impl Render for LoginView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_fields(window, cx);
        let locale = self.settings.read(cx).language.clone();
        let theme = cx.theme();

        let outer = div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .size_full();

        let mut left_col = div().flex().flex_col().gap_4().w(gpui::px(360.0));

        left_col = left_col.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_3()
                .mb_2()
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.text_primary)
                        .child(mezon_i18n::t(&locale, "common.login.welcomeBack")),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.text_secondary)
                        .child(mezon_i18n::t(&locale, "common.login.gladToMeetAgain")),
                ),
        );

        match self.method {
            LoginMethod::Otp => {
                if self.otp_step == 0 {
                    left_col = left_col.child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text_primary)
                            .child(mezon_i18n::t(&locale, "login.signInWithOtp")),
                    );
                    if let Some(field) = &self.email_field {
                        left_col = left_col.child(field.clone());
                    }

                    let loading = self.loading;
                    let entity = cx.entity().clone();
                    left_col = left_col.child(
                        div().w_full().child(
                            Button::new("send-otp")
                                .label(mezon_i18n::t(&locale, "login.sendOtp"))
                                .primary()
                                .w_full()
                                .loading(loading)
                                .disabled(loading)
                                .on_click(move |_, window, cx| {
                                    Self::handle_send_otp(&entity, window, cx);
                                })
                                .into_any_element(),
                        ),
                    );
                } else {
                    left_col = left_col.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text_primary)
                                    .child(mezon_i18n::t(&locale, "login.enterVerificationCode")),
                            )
                            .child(div().text_xs().text_color(theme.text_secondary).child(
                                format!(
                                    "{} {}",
                                    mezon_i18n::t(&locale, "login.codeSentTo"),
                                    self.otp_email
                                ),
                            )),
                    );

                    if let Some(input) = &self.otp_input {
                        left_col = left_col.child(input.clone());
                    }

                    if self.loading {
                        left_col =
                            left_col.child(div().flex().justify_center().child(Spinner::new()));
                    }

                    let countdown = self.countdown;
                    if countdown > 0 {
                        left_col = left_col.child(
                            div()
                                .flex()
                                .justify_center()
                                .text_xs()
                                .text_color(theme.text_muted)
                                .child(format!(
                                    "{} {countdown}s",
                                    mezon_i18n::t(&locale, "login.resendCodeIn")
                                )),
                        );
                    } else {
                        let entity = cx.entity().clone();
                        left_col = left_col.child(
                            div()
                                .flex()
                                .justify_center()
                                .text_xs()
                                .text_color(theme.brand)
                                .cursor_pointer()
                                .hover(|s| s.opacity(0.8))
                                .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.otp_step = 0;
                                        cx.notify();
                                    });
                                })
                                .child(mezon_i18n::t(&locale, "login.resendCode")),
                        );
                    }

                    let entity_back = cx.entity().clone();
                    left_col = left_col.child(
                        div()
                            .flex()
                            .justify_center()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.8))
                            .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                entity_back.update(cx, |this, cx| {
                                    this.otp_step = 0;
                                    this.otp_req_id.clear();
                                    this.error = None;
                                    cx.notify();
                                });
                            })
                            .child(format!("← {}", mezon_i18n::t(&locale, "login.changeEmail"))),
                    );
                }
            }

            LoginMethod::Password => {
                left_col = left_col.child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text_primary)
                        .child(mezon_i18n::t(&locale, "login.signInWithPassword")),
                );
                if let Some(field) = &self.email_field {
                    left_col = left_col.child(field.clone());
                }
                if let Some(field) = &self.password_field {
                    left_col = left_col.child(field.clone());
                }

                left_col = left_col.child(
                    div()
                        .flex()
                        .justify_end()
                        .text_xs()
                        .text_color(theme.brand)
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.8))
                        .on_mouse_down(MouseButton::Left, |_, _window, cx| {
                            if let Some(store) = mezon_store::AudioStore::try_global(cx) {
                                let _ = store
                                    .read(cx)
                                    .open_url_external("https://mezon.ai/forgot-password");
                            }
                        })
                        .child(mezon_i18n::t(&locale, "login.forgotPassword")),
                );

                let loading = self.loading;
                let entity = cx.entity().clone();
                left_col = left_col.child(
                    div().w_full().child(
                        Button::new("sign-in")
                            .label(mezon_i18n::t(&locale, "login.signIn"))
                            .primary()
                            .w_full()
                            .loading(loading)
                            .disabled(loading)
                            .on_click(move |_, _window, cx| {
                                Self::handle_sign_in(&entity, cx);
                            })
                            .into_any_element(),
                    ),
                );
            }
        }

        if let Some(err) = &self.error {
            left_col = left_col.child(
                div()
                    .text_xs()
                    .text_color(theme.status_dnd)
                    .child(err.clone()),
            );
        }

        left_col = left_col.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().flex_1().h(gpui::px(1.0)).bg(theme.border))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child(mezon_i18n::t(&locale, "common.or")),
                )
                .child(div().flex_1().h(gpui::px(1.0)).bg(theme.border)),
        );

        let toggle_label = match self.method {
            LoginMethod::Otp => mezon_i18n::t(&locale, "common.login.loginByPassword"),
            LoginMethod::Password => mezon_i18n::t(&locale, "common.login.loginByOTP"),
        };
        let entity_toggle = cx.entity().clone();
        left_col = left_col.child(
            div()
                .flex()
                .justify_center()
                .text_xs()
                .text_color(theme.brand)
                .cursor_pointer()
                .hover(|s| s.opacity(0.8))
                .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                    entity_toggle.update(cx, |this, cx| {
                        this.method = match this.method {
                            LoginMethod::Otp => LoginMethod::Password,
                            LoginMethod::Password => LoginMethod::Otp,
                        };
                        this.otp_step = 0;
                        this.error = None;
                        cx.notify();
                    });
                })
                .child(toggle_label),
        );

        let qr_expired = self.qr_expired;
        let qr_login_id = self.qr_login_id.clone();
        let entity_qr = cx.entity().clone();

        let qr_inner = div()
            .size(gpui::px(192.0))
            .border_2()
            .border_color(theme.border)
            .rounded_lg()
            .bg(gpui::white())
            .flex()
            .items_center()
            .justify_center()
            .relative();

        let qr_box = if qr_expired {
            qr_inner
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(gpui::rgba(0x00000080))
                        .rounded_lg()
                        .child(
                            div()
                                .text_xs()
                                .text_color(gpui::white())
                                .cursor_pointer()
                                .hover(|s| s.opacity(0.7))
                                .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                    Self::reload_qr(&entity_qr, cx);
                                })
                                .child(mezon_i18n::t(&locale, "common.errorBoundary.reload")),
                        ),
                )
                .into_any_element()
        } else if let Some(id) = qr_login_id {
            qr_inner
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .max_w(gpui::px(160.0))
                        .text_ellipsis()
                        .child(id),
                )
                .into_any_element()
        } else {
            qr_inner.child(Spinner::new()).into_any_element()
        };

        let right_col = div()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .child(qr_box)
            .child(
                div()
                    .text_sm()
                    .text_color(theme.text_secondary)
                    .child(mezon_i18n::t(&locale, "common.login.qr.signIn")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(mezon_i18n::t(&locale, "common.login.qr.useMobile")),
            );

        let card = div()
            .flex()
            .flex_row()
            .gap_8()
            .p_8()
            .rounded_lg()
            .bg(theme.bg_secondary)
            .items_center()
            .child(left_col)
            .child(right_col);

        outer.child(card)
    }
}
