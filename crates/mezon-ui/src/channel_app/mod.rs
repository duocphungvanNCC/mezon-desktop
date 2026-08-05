use gpui::App;
use mezon_store::PlatformStore;

fn open_in_browser(url: &str, cx: &App) {
    let Some(platform) = PlatformStore::try_global(cx) else {
        tracing::warn!("channel app launch skipped — platform store is missing");
        return;
    };
    if let Err(error) = platform.read(cx).open_url_app_window(url) {
        tracing::warn!("channel app launch failed: {error:#}");
    }
}

/// Fetch a signed launch URL and open the app in the system browser.
pub fn launch_channel_app_from_store(
    app_id: i64,
    app_url: String,
    clan_id: mezon_store::ClanId,
    clan_name: String,
    channel_list: gpui::Entity<mezon_store::ChannelList>,
    cx: &mut App,
) {
    let task = channel_list.update(cx, |store, cx| {
        store.fetch_channel_app_url(app_id, app_url, clan_id, clan_name, cx)
    });
    cx.spawn(async move |cx| {
        let Some(url) = task.await else {
            return;
        };
        cx.update(|cx| open_in_browser(&url, cx));
    })
    .detach();
}
