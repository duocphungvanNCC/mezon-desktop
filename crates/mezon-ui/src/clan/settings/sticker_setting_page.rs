use gpui::{
    AnyElement, App, Context, Entity, FontWeight, SharedString, Subscription, Window, div, img,
    prelude::*, px,
};
use mezon_store::{
    BadgeService, ClanId, ClanMembersEvent, ClanMembersStore, PermissionStore, Settings, Sticker,
    StickerEvent, StickerStore, UserId,
};

use super::emoji_sticker_picker::{
    EmojiStickerPicker, EmojiStickerPickerEvent, EmoticonEditTarget, EmoticonKind,
};
use crate::app::shell::Shell;
use crate::components::primitives::{
    Avatar, Button, ButtonVariants, Icon, IconName, Sizable, Size, h_flex, v_flex,
};
use crate::image_cache::{
    AVATAR_ENTRY_MAX_BYTES, AVATAR_IMAGE_CACHE_BYTES, AVATAR_IMAGE_CACHE_CAPACITY, LruImageCache,
};
use crate::theme::{ActiveTheme, Theme};

const MAX_STICKER_SLOTS: usize = 250;
const CARD_WIDTH: f32 = 120.0;
const CARD_HEIGHT: f32 = 150.0;
const STICKER_IMAGE_SIZE: f32 = 72.0;
const STICKER_THUMB_PROXY_PX: u32 = 144;
const STICKER_CONTENT_MAX_WIDTH: f32 = 740.0;
const STICKER_GRID_GAP_X: f32 = 16.0;
const STICKER_GRID_GAP_Y: f32 = 16.0;
const STICKER_GRID_MIN_COLUMNS: u16 = 3;
const STICKER_GRID_MAX_COLUMNS: u16 = 5;

fn sticker_grid_gap_x() -> f32 {
    let mut columns = STICKER_GRID_MIN_COLUMNS;
    for column_count in STICKER_GRID_MIN_COLUMNS..=STICKER_GRID_MAX_COLUMNS {
        let row_width =
            column_count as f32 * CARD_WIDTH + (column_count - 1) as f32 * STICKER_GRID_GAP_X;
        if row_width <= STICKER_CONTENT_MAX_WIDTH {
            columns = column_count;
        } else {
            break;
        }
    }
    let gaps = (columns - 1).max(1) as f32;
    ((STICKER_CONTENT_MAX_WIDTH - columns as f32 * CARD_WIDTH) / gaps).max(STICKER_GRID_GAP_X)
}

fn section_heading_xs(text: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
    let text = text.into().to_string().to_uppercase();
    div()
        .text_xs()
        .font_weight(FontWeight::BOLD)
        .text_color(theme.text_primary)
        .mb(px(8.0))
        .child(text)
}

fn body_text(text: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
    div()
        .text_sm()
        .font_weight(FontWeight::NORMAL)
        .text_color(theme.text_secondary)
        .child(text.into())
}

pub struct StickerSettingPage {
    clan_id: ClanId,
    settings: Entity<Settings>,
    image_cache: Entity<LruImageCache>,
    _sticker_sub: Subscription,
    _members_sub: Subscription,
}

impl StickerSettingPage {
    pub fn new(clan_id: ClanId, settings: Entity<Settings>, cx: &mut Context<Self>) -> Self {
        StickerStore::global(cx).update(cx, |store, cx| store.ensure_loaded(cx));
        ClanMembersStore::global(cx).update(cx, |store, cx| store.ensure_loaded(clan_id, cx));

        let sticker_sub = cx.subscribe(&StickerStore::global(cx), |_, _, _: &StickerEvent, cx| {
            cx.notify();
        });
        let members_sub = cx.subscribe(&ClanMembersStore::global(cx), |this, _, event, cx| {
            if matches!(event, ClanMembersEvent::Changed { clan_id } if *clan_id == this.clan_id) {
                cx.notify();
            }
        });
        let image_cache = cx.new(|cx| {
            LruImageCache::avatar_thumbnail(
                "clan-sticker-settings",
                AVATAR_IMAGE_CACHE_CAPACITY,
                AVATAR_IMAGE_CACHE_BYTES,
                AVATAR_ENTRY_MAX_BYTES,
                cx,
            )
        });

        Self {
            clan_id,
            settings,
            image_cache,
            _sticker_sub: sticker_sub,
            _members_sub: members_sub,
        }
    }

    pub fn release(&mut self) {}

    fn clan_id_str(&self) -> String {
        self.clan_id.get().to_string()
    }

    fn stickers(&self, cx: &App) -> Vec<Sticker> {
        StickerStore::global(cx)
            .read(cx)
            .for_clan(&self.clan_id_str())
            .into_iter()
            .cloned()
            .collect()
    }

    fn can_manage_sticker(&self, sticker: &Sticker, cx: &App) -> bool {
        if PermissionStore::global(cx)
            .read(cx)
            .clan_settings_permissions(self.clan_id, cx)
            .has_manage_clan
        {
            return true;
        }
        let Some(current) = BadgeService::global(cx).read(cx).current_user_id(cx) else {
            return false;
        };
        sticker
            .creator_id
            .parse::<i64>()
            .ok()
            .is_some_and(|id| id == current.get())
    }

    fn creator_display(&self, sticker: &Sticker, cx: &App) -> (SharedString, Option<SharedString>) {
        let creator_id = sticker.creator_id.parse::<UserId>().ok();
        if let Some(user_id) = creator_id
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
        (SharedString::default(), None)
    }

    fn sticker_image_src(sticker: &Sticker, cx: &App) -> SharedString {
        if sticker.src.is_empty() {
            SharedString::default()
        } else {
            crate::util::imgproxy::proxied(
                cx,
                &sticker.src,
                STICKER_THUMB_PROXY_PX,
                STICKER_THUMB_PROXY_PX,
                "fit",
            )
            .into()
        }
    }

    fn open_picker(
        &self,
        editing: Option<EmoticonEditTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = self.settings.clone();
        let clan_id = self.clan_id;
        let modal = cx.new(|cx| {
            EmojiStickerPicker::new(
                EmoticonKind::Sticker,
                clan_id,
                editing,
                settings,
                window,
                cx,
            )
        });
        cx.subscribe(&modal, |_, _, _: &EmojiStickerPickerEvent, cx| cx.notify())
            .detach();
        Shell::global(cx).update(cx, |shell, cx| shell.show_modal(modal.into(), cx));
    }

    fn open_create_modal(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_picker(None, window, cx);
    }

    fn confirm_delete_sticker(
        &mut self,
        sticker_id: SharedString,
        shortname: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let locale = self.settings.read(cx).language.clone();
        let clan_id = self.clan_id;
        Shell::global(cx).update(cx, |shell, cx| {
            shell.confirm_delete_sticker(clan_id, sticker_id, shortname, &locale, window, cx);
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
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.tokens.theme_setting_nav)
            .items_center()
            .gap_4()
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_primary)
                            .child(mezon_i18n::t(locale, "clanSettings.stickers.uploadHere")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_secondary)
                            .child(mezon_i18n::t(
                                locale,
                                "clanSettings.stickers.customizeMessage",
                            )),
                    ),
            )
            .child(
                Button::new("sticker-upload-card")
                    .label(mezon_i18n::t(locale, "clanStickerSetting.btn.upload"))
                    .primary()
                    .with_size(Size::Large)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_create_modal(window, cx);
                    })),
            )
    }

    fn render_sticker_card(
        &self,
        sticker: Sticker,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let can_manage = self.can_manage_sticker(&sticker, cx);
        let (creator_name, creator_avatar) = self.creator_display(&sticker, cx);
        let shortname = SharedString::from(sticker.shortname.clone());
        let src = Self::sticker_image_src(&sticker, cx);
        let is_for_sale = sticker.is_for_sale;
        let group_name = SharedString::from(format!("sticker-card-{}", sticker.id));

        let mut card = v_flex()
            .id(group_name.clone())
            .group(group_name.clone())
            .relative()
            .w(px(CARD_WIDTH))
            .h(px(CARD_HEIGHT))
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.tokens.bg_active_member_channel)
            .items_center()
            .justify_between()
            .child(
                div()
                    .h(px(STICKER_IMAGE_SIZE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        img(src)
                            .h(px(STICKER_IMAGE_SIZE))
                            .max_w(px(STICKER_IMAGE_SIZE))
                            .object_fit(gpui::ObjectFit::Contain),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .max_w(px(90.0))
                    .text_center()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text_primary)
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(shortname.clone()),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_end()
                    .justify_center()
                    .gap_1()
                    .child({
                        let mut avatar = Avatar::new().name(creator_name.clone()).size_px(px(16.0));
                        if let Some(src) = creator_avatar {
                            avatar = avatar.src(src);
                        }
                        div().flex_shrink_0().child(avatar)
                    })
                    .child(
                        div()
                            .max_w(px(80.0))
                            .text_xs()
                            .text_color(theme.text_secondary)
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(creator_name),
                    ),
            );

        if is_for_sale {
            card = card.child(
                div().absolute().top_1().left_1().child(
                    Icon::new(IconName::MarketIcons)
                        .size(px(16.0))
                        .text_color(gpui::rgb(0xfacc15)),
                ),
            );
        }

        if can_manage {
            let sticker_id_for_delete = SharedString::from(sticker.id.clone());
            let shortname_for_delete = shortname.clone();
            card = card.child(
                div()
                    .absolute()
                    .top(px(-8.0))
                    .right(px(-8.0))
                    .invisible()
                    .group_hover(group_name, |s| s.visible())
                    .child(
                        div()
                            .id(SharedString::from(format!("sticker-delete-{}", sticker.id)))
                            .size(px(20.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(theme.tokens.bg_theme_input_primary)
                            .shadow_sm()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_delete_sticker(
                                    sticker_id_for_delete.clone(),
                                    shortname_for_delete.clone(),
                                    window,
                                    cx,
                                );
                            }))
                            .child(
                                Icon::new(IconName::Close)
                                    .size(px(12.0))
                                    .text_color(theme.status_dnd),
                            ),
                    ),
            );
        }

        card
    }

    fn render_add_card(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("sticker-add-card")
            .group(SharedString::from("sticker-add-card"))
            .w(px(CARD_WIDTH))
            .h(px(CARD_HEIGHT))
            .p_3()
            .rounded_lg()
            .border_1()
            .border_dashed()
            .border_color(theme.tokens.bg_tertiary)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, window, cx| this.open_create_modal(window, cx)))
            .child(
                Icon::new(IconName::ImageUploadIcon)
                    .size(px(28.0))
                    .text_color(theme.text_secondary),
            )
    }

    fn render_grid(
        &self,
        stickers: &[Sticker],
        locale: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let slots_left = MAX_STICKER_SLOTS.saturating_sub(stickers.len());
        let available = mezon_i18n::t(locale, "clanStickerSetting.content.available")
            .replace("{{left}}", &slots_left.to_string());
        let gap_x = sticker_grid_gap_x();

        let mut cards: Vec<AnyElement> = Vec::with_capacity(stickers.len() + 1);
        for sticker in stickers.iter().cloned() {
            cards.push(
                self.render_sticker_card(sticker, theme, cx)
                    .into_any_element(),
            );
        }
        cards.push(self.render_add_card(theme, cx).into_any_element());

        div()
            .w_full()
            .max_w(px(STICKER_CONTENT_MAX_WIDTH))
            .child(
                div()
                    .mb(px(16.0))
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.text_secondary)
                    .child(available.to_uppercase()),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_wrap()
                    .gap_x(px(gap_x))
                    .gap_y(px(STICKER_GRID_GAP_Y))
                    .children(cards),
            )
    }
}

impl Render for StickerSettingPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let locale = self.settings.read(cx).language.clone();
        let stickers = self.stickers(cx);

        v_flex()
            .image_cache(self.image_cache.clone())
            .relative()
            .w_full()
            .gap_0()
            .pb(px(40.0))
            .child(
                v_flex()
                    .w_full()
                    .pb(px(24.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .gap_2()
                    .child(body_text(
                        mezon_i18n::t(&locale, "clanStickerSetting.content.description"),
                        &theme,
                    ))
                    .child(
                        section_heading_xs(
                            mezon_i18n::t(&locale, "clanStickerSetting.content.requirements"),
                            &theme,
                        )
                        .mt(px(8.0)),
                    )
                    .child(body_text(
                        mezon_i18n::t(&locale, "clanStickerSetting.content.reqType"),
                        &theme,
                    ))
                    .child(body_text(
                        mezon_i18n::t(&locale, "clanStickerSetting.content.reqDim"),
                        &theme,
                    ))
                    .child(body_text(
                        mezon_i18n::t(&locale, "clanStickerSetting.content.reqSize"),
                        &theme,
                    )),
            )
            .child(
                div()
                    .mt(px(16.0))
                    .child(self.render_upload_card(&locale, &theme, cx)),
            )
            .child(
                div()
                    .mt(px(16.0))
                    .child(self.render_grid(&stickers, &locale, &theme, cx)),
            )
    }
}
