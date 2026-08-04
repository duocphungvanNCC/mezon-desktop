use std::time::Duration;

use crate::components::primitives::{
    Avatar, Button as GpuiButton, ButtonVariants, Dropdown, DropdownTriggerStyle, Icon, Input,
    InputEvent, InputState, Label, h_flex, v_flex,
};
use gpui::{
    Context, Entity, FontWeight, PathPromptOptions, Rgba, SharedString, Subscription, Task, Window,
    div, prelude::*, px,
};
use mezon_store::{AccountEvent, AccountStore, ClanList, Settings};

use super::profile_page::profile_status;
use crate::theme::{ActiveTheme, Theme};
use crate::{image_cache::LruImageCache, util::avatar_color::spawn_banner_color_task};

struct ClanProfileState {
    selected_clan_id: SharedString,
    nick_name: SharedString,
    avatar_url: Option<SharedString>,
    original_nick_name: SharedString,
    original_avatar_url: Option<SharedString>,
    loading: bool,
    saving: bool,
    duplicate_error: bool,
    #[allow(dead_code)]
    fetched: bool,
}

pub struct ClanProfileSection {
    settings: Entity<Settings>,
    clan_list: Entity<ClanList>,
    profile: Option<ClanProfileState>,
    display_name: SharedString,
    username: SharedString,
    user_avatar_url: Option<SharedString>,
    status: SharedString,
    custom_status: SharedString,
    nick_name_input: Option<Entity<InputState>>,
    _subscriptions: Vec<Subscription>,
    toast_message: Option<SharedString>,
    selected_clan_id: String,
    clan_dropdown_open: bool,
    avatar_image_cache: Entity<LruImageCache>,
    banner_color: Option<Rgba>,
    banner_source: String,
    banner_task: Option<Task<()>>,
}

impl ClanProfileSection {
    pub fn new(
        settings: Entity<Settings>,
        clan_list: Entity<ClanList>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        cx.observe(&clan_list, |_, _, cx| cx.notify()).detach();
        cx.observe(&AccountStore::global(cx), |this, store, cx| {
            if let Some(account) = store.read(cx).account.as_ref() {
                let presence_changed = this.status.as_ref() != account.status
                    || this.custom_status.as_ref() != account.user_status
                    || this.user_avatar_url.as_ref().map(|url| url.as_ref())
                        != account.avatar_url.as_deref();
                this.user_avatar_url = account.avatar_url.clone().map(Into::into);
                this.status = account.status.clone().into();
                this.custom_status = account.user_status.clone().into();
                if presence_changed {
                    cx.notify();
                }
            }
            let store = store.read(cx);
            if let Some(clan_profile) = store.clan_profile.as_ref()
                && clan_profile.clan_id.to_string() == this.selected_clan_id
            {
                let nick: SharedString = clan_profile.nick_name.clone().into();
                let avatar: Option<SharedString> = clan_profile.avatar_url.clone().map(Into::into);
                let should_sync = this.profile.as_ref().is_none_or(|profile| {
                    profile.loading
                        || profile.selected_clan_id.as_ref() != clan_profile.clan_id.to_string()
                        || (!this.is_dirty()
                            && (profile.original_nick_name != nick
                                || profile.original_avatar_url != avatar))
                });
                if should_sync {
                    this.profile = Some(ClanProfileState {
                        selected_clan_id: clan_profile.clan_id.to_string().into(),
                        nick_name: nick.clone(),
                        avatar_url: avatar.clone(),
                        original_nick_name: nick,
                        original_avatar_url: avatar,
                        loading: store.clan_profile_loading,
                        saving: false,
                        duplicate_error: store.nickname_duplicate,
                        fetched: true,
                    });
                    cx.notify();
                }
            }
            this.refresh_banner_color(cx);
        })
        .detach();
        cx.subscribe(
            &AccountStore::global(cx),
            |this, _store, event, cx| match event {
                AccountEvent::ClanProfileSaved => {
                    if let Some(state) = &mut this.profile {
                        state.original_nick_name = state.nick_name.clone();
                        state.original_avatar_url = state.avatar_url.clone();
                        state.saving = false;
                    }
                    let locale = this.settings.read(cx).language.clone();
                    this.show_toast(mezon_i18n::t(&locale, "setting.clanProfile.saved"), cx);
                }
                AccountEvent::ClanProfileSaveFailed(msg) => {
                    if let Some(state) = &mut this.profile {
                        state.saving = false;
                    }
                    let locale = this.settings.read(cx).language.clone();
                    this.show_toast(
                        format!(
                            "{} {}",
                            mezon_i18n::t(&locale, "setting.clanProfile.saveFailed"),
                            msg
                        ),
                        cx,
                    );
                }
                AccountEvent::ClanProfileLoadFailed(msg) => {
                    let locale = this.settings.read(cx).language.clone();
                    this.show_toast(
                        format!(
                            "{} {}",
                            mezon_i18n::t(&locale, "setting.clanProfile.loadFailed"),
                            msg
                        ),
                        cx,
                    );
                }
                AccountEvent::NicknameDuplicateChecked(is_dup) => {
                    if let Some(state) = &mut this.profile {
                        state.duplicate_error = *is_dup;
                    }
                    cx.notify();
                }
                AccountEvent::ClanAvatarUploaded(url) => {
                    if let Some(state) = &mut this.profile {
                        state.avatar_url = Some(url.clone().into());
                    }
                    this.refresh_banner_color(cx);
                    cx.notify();
                }
                AccountEvent::ClanAvatarUploadFailed(msg) => {
                    let locale = this.settings.read(cx).language.clone();
                    this.show_toast(
                        format!(
                            "{} {}",
                            mezon_i18n::t(&locale, "setting.clanProfile.avatarUploadFailed"),
                            msg
                        ),
                        cx,
                    );
                }
                _ => {}
            },
        )
        .detach();
        let mut this = Self {
            settings,
            clan_list,
            profile: None,
            display_name: SharedString::default(),
            username: SharedString::default(),
            user_avatar_url: None,
            status: SharedString::default(),
            custom_status: SharedString::default(),
            nick_name_input: None,
            _subscriptions: Vec::new(),
            toast_message: None,
            selected_clan_id: String::new(),
            clan_dropdown_open: false,
            avatar_image_cache: crate::image_cache::shared_avatar_cache(cx),
            banner_color: None,
            banner_source: String::new(),
            banner_task: None,
        };
        this.refresh_banner_color(cx);
        this
    }

    pub fn set_user_profile(
        &mut self,
        display_name: SharedString,
        username: SharedString,
        avatar_url: Option<SharedString>,
        status: SharedString,
        custom_status: SharedString,
        cx: &mut Context<Self>,
    ) {
        self.display_name = display_name;
        self.username = username;
        self.user_avatar_url = avatar_url;
        self.status = status;
        self.custom_status = custom_status;
        self.refresh_banner_color(cx);
    }

    fn show_toast(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast_message = Some(message.into());
        cx.notify();

        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            this.update(cx, |this, cx| {
                this.toast_message = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn init_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let locale = self.settings.read(cx).language.clone();
        let nick_ph = mezon_i18n::t(&locale, "setting.clanProfile.nicknamePlaceholder");
        let nick = cx.new(|cx| InputState::new(window, cx).placeholder(nick_ph));

        if let Some(state) = &self.profile {
            nick.update(cx, |input, cx| {
                input.set_value(&state.nick_name, window, cx);
            });
        }

        self._subscriptions.push(cx.subscribe_in(&nick, window, {
            let nick = nick.clone();
            move |this: &mut Self, _, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    let value = nick.read(cx).value().to_string();
                    if let Some(state) = &mut this.profile
                        && !state.saving
                    {
                        state.nick_name = value.clone().into();
                        state.duplicate_error = false;
                    }
                    cx.notify();

                    let value = value.trim().to_string();
                    if value.len() >= 2 {
                        let clan_id = this.selected_clan_id.clone();
                        cx.spawn(async move |this, cx| {
                            cx.background_executor()
                                .timer(Duration::from_millis(600))
                                .await;
                            this.update(cx, |_, cx| {
                                AccountStore::global(cx).update(cx, |store, cx| {
                                    store.check_clan_nickname(
                                        clan_id.parse().unwrap_or_default(),
                                        &value,
                                        cx,
                                    );
                                });
                            })
                            .ok();
                        })
                        .detach();
                    }
                }
            }
        }));

        self.nick_name_input = Some(nick);
    }

    fn is_dirty(&self) -> bool {
        if let Some(state) = &self.profile {
            state.nick_name != state.original_nick_name
                || state.avatar_url != state.original_avatar_url
        } else {
            false
        }
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.is_dirty()
    }

    pub fn is_saving(&self) -> bool {
        self.profile.as_ref().is_some_and(|profile| profile.saving)
    }

    pub fn save_changes(&mut self, cx: &mut Context<Self>) {
        self.save(cx);
    }

    pub fn discard_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.discard(window, cx);
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(state) = &mut self.profile else {
            return;
        };
        if state.saving || state.duplicate_error {
            return;
        }
        state.saving = true;
        cx.notify();

        let clan_id: String = state.selected_clan_id.to_string();
        let nick_name: String = state.nick_name.to_string();
        let avatar_url: Option<String> = state.avatar_url.as_ref().map(|s| s.to_string());

        AccountStore::global(cx).update(cx, |store, cx| {
            store.save_clan_profile(
                clan_id.parse().unwrap_or_default(),
                nick_name,
                avatar_url,
                cx,
            );
        });
    }

    fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(state) = &mut self.profile {
            state.nick_name = state.original_nick_name.clone();
            state.avatar_url = state.original_avatar_url.clone();
            state.duplicate_error = false;
        }
        self.refresh_banner_color(cx);
        if let (Some(input), Some(state)) = (&self.nick_name_input, &self.profile) {
            input.update(cx, |input_state, cx| {
                input_state.set_value(state.nick_name.clone(), window, cx);
            });
        }
    }

    pub fn fetch(&mut self, clan_id: &str, cx: &mut Context<Self>) {
        self.selected_clan_id = clan_id.to_string();
        self.profile = Some(ClanProfileState {
            selected_clan_id: clan_id.into(),
            nick_name: "".into(),
            avatar_url: None,
            original_nick_name: "".into(),
            original_avatar_url: None,
            loading: true,
            saving: false,
            duplicate_error: false,
            fetched: false,
        });
        self.refresh_banner_color(cx);
        cx.notify();
        AccountStore::global(cx).update(cx, |store, cx| {
            store.fetch_clan_profile(clan_id.parse().unwrap_or_default(), cx)
        });
    }

    fn refresh_banner_color(&mut self, cx: &mut Context<Self>) {
        let source = self
            .profile
            .as_ref()
            .and_then(|profile| profile.avatar_url.clone())
            .or_else(|| self.user_avatar_url.clone())
            .map(|url| crate::util::imgproxy::profile_url(cx, &url))
            .unwrap_or_default();
        if source == self.banner_source {
            return;
        }
        self.banner_source = source.clone();
        self.banner_color = None;
        self.banner_task = spawn_banner_color_task(
            self.avatar_image_cache.clone(),
            source,
            cx,
            |this, color, cx| {
                this.banner_color = Some(color);
                cx.notify();
            },
        );
    }
}

impl Render for ClanProfileSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let locale = self.settings.read(cx).language.clone();

        if self.profile.as_ref().is_some_and(|p| !p.loading) && self.nick_name_input.is_none() {
            self.init_inputs(window, cx);
        }

        let loading = self.profile.as_ref().is_some_and(|p| p.loading);

        let clans = self.clan_list.read(cx);
        let clan_options: Vec<(SharedString, SharedString)> = clans
            .clans
            .iter()
            .map(|c| (c.id.to_string().into(), c.name.clone().into()))
            .collect();

        let selected_clan_id: SharedString = self
            .profile
            .as_ref()
            .map_or("".into(), |s| s.selected_clan_id.clone());

        let nick_name: SharedString = self
            .profile
            .as_ref()
            .map_or("".into(), |s| s.nick_name.clone());

        let avatar_url = self.profile.as_ref().and_then(|s| s.avatar_url.clone());
        let avatar_display = avatar_url
            .or_else(|| self.user_avatar_url.clone())
            .as_ref()
            .map(|url| SharedString::from(crate::util::imgproxy::profile_url(cx, url.as_ref())));

        let duplicate_error = self.profile.as_ref().is_some_and(|s| s.duplicate_error);

        let form = self.render_clan_form(
            &theme,
            &clan_options,
            &selected_clan_id,
            &nick_name,
            avatar_display.clone(),
            loading,
            duplicate_error,
            cx,
        );
        let preview = Self::render_clan_preview(
            &theme,
            &locale,
            &nick_name,
            avatar_display,
            &self.display_name,
            &self.username,
            &self.status,
            &self.custom_status,
            self.banner_color,
            self.avatar_image_cache.clone(),
        );

        v_flex()
            .gap_6()
            .child(
                h_flex()
                    .gap_8()
                    .items_start()
                    .child(div().min_w_0().flex_1().flex_basis(px(0.)).child(form))
                    .child(div().min_w_0().flex_1().flex_basis(px(0.)).child(preview)),
            )
            .when_some(self.toast_message.clone(), |this, msg| {
                this.child(
                    div()
                        .px_3()
                        .py_2()
                        .bg(theme.bg_floating)
                        .rounded_md()
                        .text_sm()
                        .text_color(theme.text_primary)
                        .child(msg),
                )
            })
            .into_any_element()
    }
}

impl ClanProfileSection {
    #[allow(clippy::too_many_arguments)]
    fn render_clan_form(
        &self,
        theme: &Theme,
        clan_options: &[(SharedString, SharedString)],
        selected_clan_id: &SharedString,
        _nick_name: &SharedString,
        _avatar_url: Option<SharedString>,
        loading: bool,
        duplicate_error: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let locale = self.settings.read(cx).language.clone();
        v_flex()
            .gap_5()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.text_muted)
                            .child(mezon_i18n::t(
                                &locale,
                                "profileSetting.showProfilesDescription",
                            )),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_muted)
                            .child(mezon_i18n::t(&locale, "setting.clanProfile.chooseClan")),
                    )
                    .child({
                        let selected = clan_options
                            .iter()
                            .position(|(id, _)| id == selected_clan_id);
                        let ids: Vec<SharedString> =
                            clan_options.iter().map(|(id, _)| id.clone()).collect();
                        Dropdown::new("clan-profile-select")
                            .items(clan_options.iter().map(|(_, name)| name.clone()).collect())
                            .selected(selected)
                            .open(self.clan_dropdown_open)
                            .trigger_style(DropdownTriggerStyle::InputPrimary)
                            .on_toggle({
                                let entity = cx.entity().clone();
                                move |_, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.clan_dropdown_open = !this.clan_dropdown_open;
                                        cx.notify();
                                    })
                                }
                            })
                            .on_select({
                                let entity = cx.entity().clone();
                                move |index, _, cx| {
                                    if let Some(id) = ids.get(index) {
                                        entity.update(cx, |this, cx| {
                                            this.clan_dropdown_open = false;
                                            this.fetch(id, cx);
                                        });
                                    }
                                }
                            })
                    }),
            )
            .when(loading, |el| {
                el.child(
                    Label::new(mezon_i18n::t(&locale, "setting.profile.loadingClan"))
                        .text_color(theme.text_muted)
                        .text_sm(),
                )
            })
            .when(!loading, |el| {
                el.child(
                    v_flex()
                        .gap_4()
                        .child(
                            v_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.text_muted)
                                        .child(mezon_i18n::t(
                                            &locale,
                                            "setting.clanProfile.clanNickname",
                                        )),
                                )
                                .child(Input::new(
                                    self.nick_name_input
                                        .as_ref()
                                        .expect("nick_name_input not initialized"),
                                ))
                                .when(duplicate_error, |el| {
                                    el.child(div().text_xs().text_color(theme.danger_text).child(
                                        mezon_i18n::t(
                                            &locale,
                                            "setting.clanProfile.nicknameExists",
                                        ),
                                    ))
                                }),
                        )
                        .child(
                            v_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.text_muted)
                                        .child(mezon_i18n::t(&locale, "profileSetting.avatar")),
                                )
                                .child(
                                    h_flex()
                                        .gap_5()
                                        .items_center()
                                        .child(
                                            GpuiButton::new("clan-change-avatar-btn")
                                                .label(mezon_i18n::t(
                                                    &locale,
                                                    "common.changeAvatar",
                                                ))
                                                .text_color(theme.text_primary)
                                                .primary()
                                                .on_click({
                                                    let entity = cx.entity().clone();
                                                    let choose_avatar = mezon_i18n::t(
                                                        &locale,
                                                        "setting.profile.chooseAvatar",
                                                    );
                                                    move |_, _, cx| {
                                                        let entity = entity.clone();
                                                        let rx = cx.prompt_for_paths(
                                                            PathPromptOptions {
                                                                files: true,
                                                                directories: false,
                                                                multiple: false,
                                                                prompt: Some(choose_avatar.into()),
                                                            },
                                                        );
                                                        cx.spawn(async move |cx| {
                                                            let paths = match rx.await {
                                                                Ok(Ok(Some(p))) => p,
                                                                _ => return,
                                                            };
                                                            let path =
                                                                match paths.into_iter().next() {
                                                                    Some(p) => p,
                                                                    None => return,
                                                                };
                                                            entity.update(cx, |_, cx| {
                                                                AccountStore::global(cx).update(
                                                                    cx,
                                                                    |store, cx| {
                                                                        store.upload_clan_avatar(
                                                                            &path, cx,
                                                                        );
                                                                    },
                                                                );
                                                            });
                                                        })
                                                        .detach();
                                                    }
                                                }),
                                        )
                                        .child(
                                            GpuiButton::new("clan-remove-avatar-btn")
                                                .label(mezon_i18n::t(
                                                    &locale,
                                                    "common.removeAvatar",
                                                ))
                                                .text_color(theme.text_muted)
                                                .border_1()
                                                .border_color(theme.border)
                                                .on_click({
                                                    let entity = cx.entity().clone();
                                                    move |_, _, cx| {
                                                        entity.clone().update(cx, |this, cx| {
                                                            if let Some(state) = &mut this.profile {
                                                                state.avatar_url = None;
                                                            }
                                                            this.refresh_banner_color(cx);
                                                            cx.notify();
                                                        });
                                                    }
                                                }),
                                        ),
                                ),
                        ),
                )
            })
    }

    fn render_clan_preview(
        theme: &Theme,
        locale: &str,
        nick_name: &SharedString,
        avatar_url: Option<SharedString>,
        display_name: &SharedString,
        username: &SharedString,
        status: &SharedString,
        custom_status: &SharedString,
        banner_color: Option<Rgba>,
        avatar_image_cache: Entity<LruImageCache>,
    ) -> impl IntoElement {
        let display_label = if nick_name.is_empty() {
            display_name.clone()
        } else {
            nick_name.clone()
        };
        let (status_icon, status_color) = profile_status(status, theme);
        let banner_color = banner_color
            .map(gpui::Hsla::from)
            .unwrap_or(theme.tokens.bg_secondary.into());

        v_flex()
            .relative()
            .gap_2()
            .child(
                Label::new(mezon_i18n::t(locale, "common.preview"))
                    .text_sm()
                    .text_color(theme.text_muted)
                    .font_weight(FontWeight::BOLD),
            )
            .child(
                div()
                    .relative()
                    .h(px(330.))
                    .w_full()
                    .rounded_lg()
                    .overflow_hidden()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.bg_secondary)
                    .child(
                        div()
                            .h(px(132.0))
                            .w_full()
                            .rounded_tl_lg()
                            .rounded_tr_lg()
                            .bg(banner_color),
                    )
                    .child(
                        v_flex()
                            .absolute()
                            .left(px(20.))
                            .right(px(20.))
                            .bottom(px(48.))
                            .p_4()
                            .rounded_lg()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.bg_primary)
                            .child(
                                Label::new(display_label.clone())
                                    .text_xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.text_secondary),
                            )
                            .child(
                                Label::new(username.clone())
                                    .text_sm()
                                    .text_color(theme.text_muted),
                            ),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(20.))
                            .top(px(86.))
                            .size(px(92.))
                            .rounded_full()
                            .bg(theme.bg_secondary)
                            .p(px(6.))
                            .child(
                                Avatar::new()
                                    .when_some(avatar_url, |av, url| av.src(url))
                                    .name(display_label)
                                    .size_px(px(80.))
                                    .image_cache(avatar_image_cache),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .right(px(5.))
                                    .bottom(px(5.))
                                    .p(px(2.))
                                    .rounded_full()
                                    .bg(theme.bg_secondary)
                                    .child(
                                        Icon::new(status_icon)
                                            .size(px(15.))
                                            .text_color(status_color),
                                    ),
                            )
                            .when(!custom_status.is_empty(), |avatar| {
                                avatar.child(
                                    div()
                                        .absolute()
                                        .right(px(-20.))
                                        .top(px(28.))
                                        .size(px(14.))
                                        .rounded_full()
                                        .bg(theme.tokens.bg_secondary)
                                        .border_1()
                                        .border_color(theme.bg_secondary),
                                )
                            }),
                    ),
            )
            .when(!custom_status.is_empty(), |preview| {
                preview.child(
                    div()
                        .absolute()
                        .left(px(120.))
                        .top(px(168.))
                        .max_w(px(250.))
                        .max_h(px(64.))
                        .px_4()
                        .py_3()
                        .rounded_xl()
                        .bg(theme.tokens.bg_secondary)
                        .border_1()
                        .border_color(theme.border)
                        .shadow_md()
                        .text_sm()
                        .text_color(theme.text_secondary)
                        .overflow_hidden()
                        .child(custom_status.clone()),
                )
            })
    }
}
