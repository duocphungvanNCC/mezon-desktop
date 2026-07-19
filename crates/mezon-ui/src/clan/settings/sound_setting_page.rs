use gpui::{
    App, Context, Entity, FontWeight, Render, SharedString, Subscription, Window, div, prelude::*,
    px, relative,
};
use mezon_store::{
    BadgeService, ClanId, ClanMembersEvent, ClanMembersStore, ClanSound, PermissionStore, Settings,
    StickerEvent, StickerStore, UserId, VoiceStore,
};

use super::sound_picker::{SoundEditTarget, SoundPicker, SoundPickerEvent};
use crate::app::shell::Shell;
use crate::components::primitives::{
    Avatar, Button, ButtonVariants, Icon, IconName, Sizable, Size, h_flex, v_flex,
};
use crate::theme::{ActiveTheme, Theme};
use crate::util::download::save_with_progress_toast;

const LIST_GAP: f32 = 16.0;
const CARD_WIDTH: f32 = 236.0;

fn format_mmss(secs: f64) -> String {
    let total = secs.max(0.0).round() as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

fn audio_time_label(current: f64, duration: f64) -> SharedString {
    let duration_secs = duration.max(0.0).round() as u64;
    let label = if duration_secs == 0 {
        format_mmss(current)
    } else {
        format!(
            "{} / {}",
            format_mmss(current),
            format_mmss(duration)
        )
    };
    label.into()
}

fn sound_download_name(shortname: &str, url: &str) -> SharedString {
    url.split('/')
        .next_back()
        .filter(|name| !name.is_empty())
        .map(SharedString::from)
        .unwrap_or_else(|| SharedString::from(format!("{shortname}.mp3")))
}

fn section_heading_xs(text: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
    let text = text.into().to_string().to_uppercase();
    div()
        .text_xs()
        .font_weight(FontWeight::BOLD)
        .text_color(theme.text_secondary)
        .child(text)
}

fn body_text(text: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
    div()
        .text_sm()
        .font_weight(FontWeight::NORMAL)
        .text_color(theme.tokens.text_theme_primary)
        .child(text.into())
}

pub struct SoundSettingPage {
    clan_id: ClanId,
    settings: Entity<Settings>,
    sticker_store: Entity<StickerStore>,
    _sticker_sub: Subscription,
    _voice_observe: Subscription,
    _members_observe: Subscription,
    _perm_observe: Subscription,
    _modal_sub: Option<Subscription>,
}

impl SoundSettingPage {
    pub fn new(clan_id: ClanId, settings: Entity<Settings>, cx: &mut Context<Self>) -> Self {
        let sticker_store = StickerStore::global(cx);
        sticker_store.update(cx, |store, cx| store.ensure_loaded(cx));
        ClanMembersStore::global(cx).update(cx, |store, cx| store.ensure_loaded(clan_id, cx));
        PermissionStore::global(cx).update(cx, |store, cx| {
            store.load_clan_permissions(clan_id, cx);
        });

        let sticker_sub = cx.subscribe(&sticker_store, |_, _, _: &StickerEvent, cx| {
            cx.notify();
        });
        let voice_observe = cx.observe(&VoiceStore::global(cx), |_, _, cx| cx.notify());
        let members_observe = cx.subscribe(&ClanMembersStore::global(cx), |this, _, event, cx| {
            if matches!(event, ClanMembersEvent::Changed { clan_id } if *clan_id == this.clan_id) {
                cx.notify();
            }
        });
        let perm_observe = cx.observe(&PermissionStore::global(cx), |_, _, cx| cx.notify());

        Self {
            clan_id,
            settings,
            sticker_store,
            _sticker_sub: sticker_sub,
            _voice_observe: voice_observe,
            _members_observe: members_observe,
            _perm_observe: perm_observe,
            _modal_sub: None,
        }
    }

    pub fn release(&mut self) {
        self._modal_sub.take();
    }

    fn clan_id_str(&self) -> String {
        self.clan_id.get().to_string()
    }

    fn sounds(&self, cx: &App) -> Vec<ClanSound> {
        self.sticker_store
            .read(cx)
            .sounds_for_clan(&self.clan_id_str())
            .into_iter()
            .cloned()
            .collect()
    }

    fn can_manage_sound(&self, creator_id: &str, cx: &App) -> bool {
        let perms = PermissionStore::global(cx)
            .read(cx)
            .clan_settings_permissions(self.clan_id, cx);
        if perms.has_manage_clan {
            return true;
        }
        let current = BadgeService::global(cx)
            .read(cx)
            .current_user_id(cx)
            .map(|id| id.get().to_string());
        current.is_some_and(|uid| uid == creator_id)
    }

    fn creator_label(&self, creator_id: &str, cx: &App) -> Option<(SharedString, SharedString)> {
        if creator_id.is_empty() {
            return None;
        }
        let user_id = creator_id.parse::<UserId>().ok()?;
        let member = ClanMembersStore::global(cx)
            .read(cx)
            .member(self.clan_id, user_id)?;
        let name = member.name().to_string().into();
        let avatar = if member.avatar().is_empty() {
            SharedString::default()
        } else {
            crate::util::imgproxy::avatar_url(cx, member.avatar()).into()
        };
        Some((name, avatar))
    }

    fn open_picker(
        &mut self,
        editing: Option<SoundEditTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let clan_id = self.clan_id;
        let settings = self.settings.clone();
        let modal = cx.new(|cx| SoundPicker::new(clan_id, editing, settings, window, cx));
        self._modal_sub = Some(cx.subscribe(&modal, |this, _, _: &SoundPickerEvent, cx| {
            this._modal_sub = None;
            cx.notify();
        }));
        Shell::global(cx).update(cx, |shell, cx| shell.show_modal(modal.into(), cx));
    }

    fn open_upload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_picker(None, window, cx);
    }

    fn toggle_preview(&mut self, url: String, cx: &mut Context<Self>) {
        VoiceStore::global(cx).update(cx, |store, cx| {
            store.toggle_sound_preview(url, cx);
        });
    }

    fn confirm_delete_sound(
        &mut self,
        sound_id: SharedString,
        shortname: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let locale = self.settings.read(cx).language.clone();
        let clan_id = self.clan_id;
        Shell::global(cx).update(cx, |shell, cx| {
            shell.confirm_delete_sound(clan_id, sound_id, shortname, &locale, window, cx);
        });
    }

    fn render_upload_card(
        &self,
        locale: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .w_full()
            .p(px(16.0))
            .gap(px(16.0))
            .items_center()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.tokens.theme_setting_nav)
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_secondary)
                            .child(mezon_i18n::t(locale, "clanSoundSetting.main.uploadHere")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.tokens.text_theme_primary)
                            .child(mezon_i18n::t(
                                locale,
                                "clanSoundSetting.main.personalizeDescription",
                            )),
                    ),
            )
            .child(
                Button::new("sound-upload-open")
                    .label(mezon_i18n::t(locale, "clanSoundSetting.main.uploadSound"))
                    .primary()
                    .with_size(Size::Large)
                    .on_click(cx.listener(|this, _, window, cx| this.open_upload(window, cx))),
            )
    }

    fn render_playbar(
        &self,
        sound_id: &str,
        url: SharedString,
        download_name: SharedString,
        previewing: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let voice = VoiceStore::global(cx).read(cx);
        let (position, duration) = if previewing {
            voice
                .sound_preview_timeline(url.as_ref())
                .unwrap_or_else(|| {
                    (
                        0.0,
                        voice.cached_sound_duration(url.as_ref()).unwrap_or(0.0),
                    )
                })
        } else {
            (0.0, voice.cached_sound_duration(url.as_ref()).unwrap_or(0.0))
        };
        let progress = if duration > 0.0 {
            (position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let time_label = audio_time_label(if previewing { position } else { 0.0 }, duration);
        let show_time = previewing || duration > 0.0;

        div()
            .flex_1()
            .min_w(px(0.0))
            .h(px(36.0))
            .px(px(10.0))
            .rounded_full()
            .border_1()
            .border_color(theme.border)
            .bg(theme.bg_secondary)
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(px(4.0))
                    .rounded_full()
                    .bg(theme.border)
                    .overflow_hidden()
                    .when(progress > 0.0, |track| {
                        track.child(
                            div()
                                .h_full()
                                .w(relative(progress))
                                .rounded_full()
                                .bg(theme.brand),
                        )
                    }),
            )
            .child(if show_time {
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .whitespace_nowrap()
                    .text_color(theme.tokens.text_theme_primary)
                    .child(time_label)
                    .into_any_element()
            } else {
                div()
                    .id(SharedString::from(format!("sound-download-{sound_id}")))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.8))
                    .child(
                        Icon::new(IconName::Download)
                            .size(px(16.0))
                            .text_color(theme.tokens.text_theme_primary),
                    )
                    .on_click(cx.listener(move |_, _, _, cx| {
                        save_with_progress_toast(url.clone(), download_name.clone(), cx);
                    }))
                    .into_any_element()
            })
    }

    fn render_sound_card(
        &self,
        sound: ClanSound,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let can_manage = self.can_manage_sound(&sound.creator_id, cx);
        let url = sound.src.clone();
        let download_url: SharedString = url.clone().into();
        let download_name = sound_download_name(&sound.shortname, &url);
        let previewing = VoiceStore::global(cx)
            .read(cx)
            .previewing_sound()
            .is_some_and(|active| active == url.as_str());
        let creator = self.creator_label(&sound.creator_id, cx);
        let sound_id = sound.id.clone();

        v_flex()
            .id(SharedString::from(format!("sound-card-{sound_id}")))
            .w(px(CARD_WIDTH))
            .flex_shrink_0()
            .relative()
            .p(px(16.0))
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.tokens.theme_setting_nav)
            .child(
                div()
                    .relative()
                    .w_full()
                    .mb(px(12.0))
                    .child(
                        div()
                            .w_full()
                            .px(px(28.0))
                            .text_center()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text_secondary)
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(sound.shortname.clone()),
                    )
                    .when(can_manage, |el| {
                        el.child(
                            div()
                                .absolute()
                                .top(px(-8.0))
                                .right(px(-8.0))
                                .id(SharedString::from(format!("sound-delete-{sound_id}")))
                                .size(px(24.0))
                                .rounded_full()
                                .bg(theme.tokens.theme_setting_primary)
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .child(
                                    Icon::new(IconName::Close)
                                        .size(px(12.0))
                                        .text_color(theme.status_dnd),
                                )
                                .on_click(cx.listener({
                                    let sound_id = SharedString::from(sound_id.clone());
                                    let shortname = SharedString::from(sound.shortname.clone());
                                    move |this, _, window, cx| {
                                        this.confirm_delete_sound(
                                            sound_id.clone(),
                                            shortname.clone(),
                                            window,
                                            cx,
                                        );
                                    }
                                })),
                        )
                    }),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(px(8.0))
                    .mb(px(8.0))
                    .child(
                        div()
                            .id(SharedString::from(format!("sound-play-{}", sound.id)))
                            .size(px(36.0))
                            .rounded_full()
                            .bg(theme.brand)
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .child(
                                Icon::new(if previewing {
                                    IconName::AudioPause
                                } else {
                                    IconName::AudioPlay
                                })
                                .size(px(16.0))
                                .text_color(theme.text_primary),
                            )
                            .on_click(cx.listener({
                                let url = url.clone();
                                move |this, _, _, cx| this.toggle_preview(url.clone(), cx)
                            })),
                    )
                    .child(self.render_playbar(
                        &sound_id,
                        download_url,
                        download_name,
                        previewing,
                        &theme,
                        cx,
                    )),
            )
            .when_some(creator, |el, (name, avatar)| {
                el.child(
                    h_flex()
                        .w_full()
                        .max_w_full()
                        .justify_center()
                        .items_center()
                        .gap(px(4.0))
                        .mt(px(4.0))
                        .child({
                            let mut avatar_el =
                                Avatar::new().name(name.clone()).size_px(px(16.0));
                            if !avatar.is_empty() {
                                avatar_el = avatar_el.src(avatar);
                            }
                            div().flex_shrink_0().child(avatar_el)
                        })
                        .child(
                            div()
                                .min_w(px(0.0))
                                .max_w(px(80.0))
                                .text_xs()
                                .text_color(theme.tokens.text_theme_primary)
                                .truncate()
                                .whitespace_nowrap()
                                .child(name),
                        ),
                )
            })
    }

    fn render_empty_state(&self, locale: &str, theme: &Theme) -> impl IntoElement {
        div()
            .w_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .text_center()
            .py(px(40.0))
            .rounded_lg()
            .border_2()
            .border_dashed()
            .border_color(theme.border)
            .bg(theme.tokens.theme_setting_nav)
            .child(
                Icon::new(IconName::Speaker)
                    .size(px(40.0))
                    .text_color(theme.tokens.text_theme_primary)
                    .mb(px(8.0)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.tokens.text_theme_primary)
                    .child(mezon_i18n::t(
                        locale,
                        "clanSoundSetting.main.noSoundEffects",
                    )),
            )
    }

    fn render_voice_grid(
        &self,
        sounds: &[ClanSound],
        locale: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut grid = div().w_full().flex().flex_wrap().gap(px(LIST_GAP));

        if sounds.is_empty() {
            grid = grid.child(self.render_empty_state(locale, theme));
        } else {
            for sound in sounds {
                grid = grid.child(self.render_sound_card(sound.clone(), theme, cx));
            }
        }
        grid
    }
}

impl Render for SoundSettingPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let locale = self.settings.read(cx).language.clone();
        let sounds = self.sounds(cx);

        v_flex()
            .relative()
            .w_full()
            .gap_0()
            .pb(px(40.0))
            .child(
                v_flex()
                    .w_full()
                    .gap(px(8.0))
                    .pb(px(24.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(section_heading_xs(
                        mezon_i18n::t(&locale, "clanSoundSetting.main.uploadInstructions"),
                        &theme,
                    ))
                    .child(body_text(
                        mezon_i18n::t(&locale, "clanSoundSetting.main.fileRequirements"),
                        &theme,
                    )),
            )
            .child(
                div()
                    .w_full()
                    .mt(px(16.0))
                    .child(self.render_upload_card(&locale, &theme, cx)),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap(px(16.0))
                    .child(
                        h_flex()
                            .mt(px(16.0))
                            .w_full()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                Icon::new(IconName::Speaker)
                                    .size(px(20.0))
                                    .text_color(theme.tokens.text_theme_primary),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text_secondary)
                                    .child(mezon_i18n::t(
                                        &locale,
                                        "clanSoundSetting.main.soundEffectList",
                                    )),
                            ),
                    )
                    .child(self.render_voice_grid(&sounds, &locale, &theme, cx)),
            )
    }
}
