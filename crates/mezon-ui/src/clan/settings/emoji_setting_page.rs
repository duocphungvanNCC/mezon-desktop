use gpui::{
    AnyElement, App, Context, Entity, FontWeight, ScrollHandle, SharedString, Subscription, Window,
    div, img, point, prelude::*, px,
};
use mezon_store::{
    BadgeService, ClanId, ClanMembersEvent, ClanMembersStore, Emoji, EmojiEvent, EmojiStore,
    PermissionStore, Settings, UserId,
};

use super::emoji_sticker_picker::{
    EmojiStickerPicker, EmojiStickerPickerEvent, EmoticonEditTarget, EmoticonKind,
};
use crate::app::shell::Shell;
use crate::components::primitives::{
    Avatar, Button, ButtonVariants, Icon, IconName, h_flex, v_flex,
};
use crate::image_cache::{AVATAR_ENTRY_MAX_BYTES, LruImageCache};
use crate::theme::{ActiveTheme, Theme};

const EMOJI_ROW_H: f32 = 48.0;
const EMOJI_ROW_ACTION_RIGHT: f32 = 20.0;
const EMOJI_THUMB_PX: f32 = 32.0;
const EMOJI_LIST_CACHE_CAPACITY: usize = 512;
const EMOJI_LIST_CACHE_BYTES: u64 = 32 * 1024 * 1024;
const TABLE_HEADER_SCROLL_CHILD: usize = 1;

fn emoji_thumb_fallback(
    size: gpui::Pixels,
    color: impl Into<gpui::Hsla>,
) -> impl Fn() -> AnyElement + 'static {
    let color = color.into();
    move || {
        div()
            .size(size)
            .flex()
            .items_center()
            .justify_center()
            .child(
                Icon::new(IconName::ImageThumbnail)
                    .size(size)
                    .text_color(color),
            )
            .into_any_element()
    }
}

fn section_heading_xs(text: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
    div()
        .text_xs()
        .font_weight(FontWeight::BOLD)
        .text_color(theme.text_primary)
        .child(text.into().to_string().to_uppercase())
}

fn body_text(text: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
    div()
        .text_sm()
        .font_weight(FontWeight::NORMAL)
        .text_color(theme.text_secondary)
        .child(text.into())
}

pub struct EmojiSettingPage {
    clan_id: ClanId,
    settings: Entity<Settings>,
    emoji_image_cache: Entity<LruImageCache>,
    scroll: ScrollHandle,
    _emoji_sub: Subscription,
    _members_sub: Subscription,
}

impl EmojiSettingPage {
    pub fn new(clan_id: ClanId, settings: Entity<Settings>, cx: &mut Context<Self>) -> Self {
        EmojiStore::global(cx).update(cx, |store, cx| store.ensure_loaded(cx));
        ClanMembersStore::global(cx).update(cx, |store, cx| store.ensure_loaded(clan_id, cx));

        let emoji_sub = cx.subscribe(&EmojiStore::global(cx), |_, _, event, cx| {
            if matches!(event, EmojiEvent::Changed) {
                cx.notify();
            }
        });
        let members_sub = cx.subscribe(&ClanMembersStore::global(cx), |this, _, event, cx| {
            if matches!(event, ClanMembersEvent::Changed { clan_id } if *clan_id == this.clan_id) {
                cx.notify();
            }
        });
        let emoji_image_cache = cx.new(|cx| {
            LruImageCache::avatar_thumbnail(
                "clan-emoji-settings-thumbs",
                EMOJI_LIST_CACHE_CAPACITY,
                EMOJI_LIST_CACHE_BYTES,
                AVATAR_ENTRY_MAX_BYTES,
                cx,
            )
        });

        Self {
            clan_id,
            settings,
            emoji_image_cache,
            scroll: ScrollHandle::new(),
            _emoji_sub: emoji_sub,
            _members_sub: members_sub,
        }
    }

    pub fn release(&mut self) {
        self.scroll.set_offset(point(px(0.0), px(0.0)));
    }

    fn table_header_sticky(&self) -> bool {
        let Some(header_bounds) = self.scroll.bounds_for_item(TABLE_HEADER_SCROLL_CHILD) else {
            return false;
        };
        let viewport = self.scroll.bounds();
        let header_top_in_viewport =
            header_bounds.top() + self.scroll.offset().y - viewport.top();
        header_top_in_viewport <= px(0.)
    }

    fn can_manage(&self, cx: &App) -> bool {
        PermissionStore::global(cx)
            .read(cx)
            .clan_settings_permissions(self.clan_id, cx)
            .has_manage_clan
    }

    fn can_modify_emoji(&self, creator_id: &str, cx: &App) -> bool {
        if self.can_manage(cx) {
            return true;
        }
        BadgeService::global(cx)
            .read(cx)
            .current_user_id(cx)
            .is_some_and(|uid| uid.get().to_string() == creator_id)
    }

    fn open_create_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_picker(None, window, cx);
    }

    fn open_edit_modal(&mut self, emoji: &Emoji, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_modify_emoji(&emoji.creator_id, cx) {
            return;
        }
        let editing = EmoticonEditTarget {
            id: emoji.id.clone(),
            shortname: emoji.shortname.clone(),
            source: emoji.src.clone(),
        };
        self.open_picker(Some(editing), window, cx);
    }

    fn open_picker(
        &mut self,
        editing: Option<EmoticonEditTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = self.settings.clone();
        let clan_id = self.clan_id;
        let picker = cx.new(|cx| {
            EmojiStickerPicker::new(EmoticonKind::Emoji, clan_id, editing, settings, window, cx)
        });
        cx.subscribe(&picker, |_, _, _: &EmojiStickerPickerEvent, cx| cx.notify())
            .detach();
        Shell::global(cx).update(cx, |shell, cx| shell.show_modal(picker.into(), cx));
    }

    fn confirm_delete_emoji(
        &mut self,
        emoji_id: SharedString,
        shortname: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let locale = self.settings.read(cx).language.clone();
        let clan_id = self.clan_id;
        Shell::global(cx).update(cx, |shell, cx| {
            shell.confirm_delete_emoji(clan_id, emoji_id, shortname, &locale, window, cx);
        });
    }

    /// Match web `getSrcEmoji(id)`: `${NX_BASE_IMG_URL}/emojis/${id}.webp` (+ imgproxy on prod CDN only).
    fn emoji_image_src(emoji: &Emoji, cx: &App) -> SharedString {
        crate::util::imgproxy::emoji_url(cx, &emoji.id).into()
    }

    fn creator_display(&self, emoji: &Emoji, cx: &App) -> (SharedString, Option<SharedString>) {
        if let Ok(user_id) = emoji.creator_id.parse::<UserId>()
            && let Some(member) = ClanMembersStore::global(cx)
                .read(cx)
                .member(self.clan_id, user_id)
        {
            let name = member.name().to_string();
            let avatar = member.avatar();
            let avatar_src = if avatar.is_empty() {
                None
            } else {
                Some(SharedString::from(crate::util::imgproxy::avatar_url(
                    cx, avatar,
                )))
            };
            return (SharedString::from(name), avatar_src);
        }
        let label = if emoji.creator_id.len() > 8 {
            format!("{}…", &emoji.creator_id[..8])
        } else {
            emoji.creator_id.clone()
        };
        (SharedString::from(label), None)
    }

    fn clan_emojis(&self, cx: &App) -> Vec<Emoji> {
        let clan_id = self.clan_id.get().to_string();
        EmojiStore::global(cx)
            .read(cx)
            .for_clan(&clan_id)
            .into_iter()
            .cloned()
            .collect()
    }

    fn render_require_list(require_list: &str, theme: &Theme) -> gpui::Div {
        let lines: Vec<&str> = require_list
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        v_flex()
            .gap_1()
            .children(lines.into_iter().map(|line| body_text(line.trim(), theme)))
    }

    fn render_table_header(locale: &str, theme: &Theme, pinned: bool) -> impl IntoElement {
        h_flex()
            .id(if pinned {
                "clan-emoji-table-header-sticky"
            } else {
                "clan-emoji-table-header"
            })
            .w_full()
            .items_center()
            .pb(px(8.0))
            .when(pinned, |row| {
                row.bg(theme.tokens.theme_setting_primary)
                    .border_b_1()
                    .border_color(theme.border)
            })
            .child(
                div()
                    .w(px(56.0))
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.text_primary)
                    .child(mezon_i18n::t(locale, "clanEmojiSetting.image").to_uppercase()),
            )
            .child(
                div()
                    .flex_1()
                    .pl(px(20.0))
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.text_primary)
                    .child(mezon_i18n::t(locale, "clanEmojiSetting.name").to_uppercase()),
            )
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.text_primary)
                    .child(mezon_i18n::t(locale, "clanEmojiSetting.uploadedBy").to_uppercase()),
            )
    }
}

fn render_emoji_row(
    emoji: &Emoji,
    theme: &Theme,
    can_modify: bool,
    creator_name: SharedString,
    creator_avatar: Option<SharedString>,
    src: SharedString,
    entity: Entity<EmojiSettingPage>,
) -> AnyElement {
    let group_name = SharedString::from(format!("emoji-row-{}", emoji.id));
    let shortname = SharedString::from(emoji.shortname.clone());
    let emoji_id = SharedString::from(emoji.id.clone());
    let emoji_for_edit = emoji.clone();
    let fallback_color = theme.text_muted;

    h_flex()
        .id(SharedString::from(format!("emoji-row-{emoji_id}")))
        .group(group_name.clone())
        .relative()
        .w_full()
        .pr(px(EMOJI_ROW_ACTION_RIGHT))
        .h(px(EMOJI_ROW_H))
        .items_center()
        .border_b_1()
        .border_color(theme.border)
        .hover(|s| s.bg(theme.bg_hover))
        .child(
            div()
                .w(px(56.0))
                .h(px(EMOJI_THUMB_PX))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(src)
                        .id(SharedString::from(format!("emoji-thumb-{emoji_id}")))
                        .size(px(EMOJI_THUMB_PX))
                        .flex_none()
                        .object_fit(gpui::ObjectFit::Contain)
                        .with_fallback(emoji_thumb_fallback(px(EMOJI_THUMB_PX), fallback_color)),
                ),
        )
        .child(
            div()
                .id(SharedString::from(format!("emoji-name-{emoji_id}")))
                .flex_1()
                .pl(px(20.0))
                .text_sm()
                .text_color(theme.text_primary)
                .child(shortname.clone())
                .when(can_modify, |el| {
                    let entity = entity.clone();
                    el.cursor_pointer()
                        .on_click(move |_, window, cx| {
                            entity.update(cx, |this, cx| {
                                this.open_edit_modal(&emoji_for_edit, window, cx);
                            });
                        })
                }),
        )
        .child(
            h_flex()
                .flex_1()
                .gap(px(6.0))
                .items_center()
                .child({
                    let mut avatar =
                        Avatar::new().name(creator_name.clone()).size_px(px(24.0));
                    if let Some(src) = creator_avatar {
                        avatar = avatar.src(src);
                    }
                    avatar
                })
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.text_secondary)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(creator_name),
                ),
        )
        .when(emoji.is_for_sale || can_modify, |row| {
            row.child(
                h_flex()
                    .absolute()
                    .right(px(EMOJI_ROW_ACTION_RIGHT))
                    .top_0()
                    .bottom_0()
                    .items_center()
                    .gap_2()
                    .when(emoji.is_for_sale, |actions| {
                        actions.child(
                            Icon::new(IconName::MarketIcons)
                                .size(px(16.0))
                                .text_color(gpui::rgb(0xfacc15)),
                        )
                    })
                    .when(can_modify, |actions| {
                        let entity = entity.clone();
                        let emoji_id = emoji_id.clone();
                        let shortname = shortname.clone();
                        actions.child(
                            div()
                                .child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "emoji-delete-{emoji_id}"
                                        )))
                                        .size(px(20.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .border_1()
                                        .border_color(theme.border)
                                        .bg(theme.bg_secondary)
                                        .cursor_pointer()
                                        .child(
                                            Icon::new(IconName::Close)
                                                .size(px(14.0))
                                                .text_color(theme.status_dnd),
                                        )
                                        .on_click(move |_, window, cx| {
                                            cx.stop_propagation();
                                            entity.update(cx, |this, cx| {
                                                this.confirm_delete_emoji(
                                                    emoji_id.clone(),
                                                    shortname.clone(),
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }),
                                ),
                        )
                    }),
            )
        })
        .into_any_element()
}

impl Render for EmojiSettingPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let locale = self.settings.read(cx).language.clone();
        let emojis = self.clan_emojis(cx);
        let require_list = mezon_i18n::t(&locale, "clanEmojiSetting.description.requireList");
        let entity = cx.entity();
        let emoji_image_cache = self.emoji_image_cache.clone();
        let table_header_sticky = self.table_header_sticky();
        let emoji_list = v_flex()
            .image_cache(emoji_image_cache)
            .id("clan-emoji-settings-list")
            .w_full()
            .children(emojis.iter().map(|emoji| {
                let can_modify = self.can_modify_emoji(&emoji.creator_id, cx);
                let (creator_name, creator_avatar) = self.creator_display(emoji, cx);
                let src = Self::emoji_image_src(emoji, cx);
                render_emoji_row(
                    emoji,
                    &theme,
                    can_modify,
                    creator_name,
                    creator_avatar,
                    src,
                    entity.clone(),
                )
            }));

        let upload_section = v_flex()
            .id("clan-emoji-upload-section")
            .w_full()
            .gap_3()
            .pb(px(40.0))
            .child(
                v_flex()
                    .gap_2()
                    .child(section_heading_xs(
                        mezon_i18n::t(&locale, "clanSettings.emoji.uploadInstructions"),
                        &theme,
                    ))
                    .child(body_text(
                        mezon_i18n::t(&locale, "clanEmojiSetting.description.descriptions"),
                        &theme,
                    ))
                    .child(section_heading_xs(
                        mezon_i18n::t(&locale, "clanEmojiSetting.description.requirements"),
                        &theme,
                    ))
                    .child(Self::render_require_list(require_list, &theme))
                    .child(
                        Button::new("emoji-upload")
                            .label(mezon_i18n::t(&locale, "clanEmojiSetting.button.upload"))
                            .primary()
                            .self_start()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_create_modal(window, cx);
                            })),
                    ),
            );

        div()
            .relative()
            .size_full()
            .min_h_0()
            .child(
                div()
                    .id("clan-emoji-settings-scroll")
                    .absolute()
                    .inset_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                    .child(upload_section)
                    .child(
                        div()
                            .w_full()
                            .mt(px(16.0))
                            .child(Self::render_table_header(&locale, &theme, false)),
                    )
                    .child(
                        div()
                            .w_full()
                            .pb(px(60.0))
                            .child(emoji_list),
                    ),
            )
            .when(table_header_sticky, |panel| {
                panel.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .occlude()
                        .child(Self::render_table_header(&locale, &theme, true)),
                )
            })
    }
}
