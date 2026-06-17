use anyhow::Result;
use futures::StreamExt;
use gpui::{App, AppContext, AsyncApp, Bounds, Entity, WindowBounds, WindowOptions, px, size};
use gpui_platform::application;
use mezon_client::{
    AppApi, ConnectionStatus, MezonClient, RealtimeEvent, TransportClient, keychain,
};
use mezon_native::instance::SingleInstance;
use mezon_store::{AppConfig, AuthState, Settings};
use mezon_ui::{RootView, init as init_ui, title_bar::TitleBar};
use std::borrow::Cow;
use std::sync::Arc;
use tracing_subscriber::{EnvFilter, fmt};

fn main() -> Result<()> {
    load_dotenv();

    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("mezon=debug,info")),
        )
        .init();

    tracing::info!("Starting Mezon desktop app v{}", env!("CARGO_PKG_VERSION"));

    // Check if a mezonapp:// deep link URL was passed as argv[1].
    let deep_link_url: Option<String> = std::env::args()
        .nth(1)
        .filter(|a| a.starts_with("mezonapp://"));

    // Single instance guard — forward deep link URL to an existing instance if running.
    let lock_result = match deep_link_url.as_deref() {
        Some(url) => SingleInstance::try_acquire_or_forward(url)?,
        None => SingleInstance::try_acquire()?,
    };

    match lock_result {
        None => {
            tracing::info!("Another instance is already running — exiting");
            return Ok(());
        }
        Some(lock) => run_app(lock, deep_link_url),
    }

    Ok(())
}

/// Load `.env` from the current working directory or workspace root.
fn load_dotenv() {
    if dotenvy::dotenv().is_ok() {
        return;
    }
    let workspace_env = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env");
    let _ = dotenvy::from_path(workspace_env);
}

fn run_app(lock: SingleInstance, initial_url: Option<String>) {
    // Build a multi-threaded tokio runtime that lives for the entire process.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build tokio runtime");
    let rt_handle = Arc::new(rt.handle().clone());

    let settings = rt.block_on(Settings::load()).unwrap_or_default();

    tracing::debug!(
        "Settings: theme={}, zoom={}, auto_start={}",
        settings.theme,
        settings.zoom_factor,
        settings.auto_start
    );

    // ── Determine initial auth state from keychain ────────────────────────────
    let app_config = Arc::new(AppConfig::from_env());
    tracing::debug!(
        "App config: rest={}:{} secure={} (gw host; api_host={})",
        app_config.client_host(),
        app_config.client_port(),
        app_config.api_secure,
        app_config.api_host,
    );
    let client = MezonClient::new(
        app_config.client_host(),
        app_config.client_port(),
        app_config.api_secure,
        &app_config.api_key,
    );
    let client = Arc::new(client);
    let transport = Arc::new(TransportClient::new(String::new()));
    let api = Arc::new(AppApi::new(transport.clone()));
    let initial_auth_state = resolve_initial_auth_state();

    // Sync login-item with settings.
    mezon_native::autostart::sync_auto_start(settings.auto_start);

    // Register mezonapp:// deep link scheme (idempotent).
    mezon_native::deep_link::register_deep_link_scheme();

    // Subscribe to screen lock/unlock events.
    mezon_native::power::subscribe(Box::new(|event| match event {
        mezon_native::power::PowerEvent::ScreenLocked => {
            tracing::info!("Screen locked");
        }
        mezon_native::power::PowerEvent::ScreenUnlocked => {
            tracing::info!("Screen unlocked");
        }
    }));

    application()
        .with_http_client(Arc::new(reqwest_client::ReqwestClient::new()))
        .with_assets(mezon_ui::assets::Assets)
        .run(move |cx: &mut App| {
            tracing::debug!("App started");

            // Register gg sans font (TTFs pre-decompressed by build.rs)
            let gg_sans_paths: &[(&[u8], &str)] = &[
                (
                    include_bytes!(concat!(env!("OUT_DIR"), "/ggsans-Normal.ttf")),
                    "Normal",
                ),
                (
                    include_bytes!(concat!(env!("OUT_DIR"), "/ggsans-Medium.ttf")),
                    "Medium",
                ),
                (
                    include_bytes!(concat!(env!("OUT_DIR"), "/ggsans-Semibold.ttf")),
                    "Semibold",
                ),
                (
                    include_bytes!(concat!(env!("OUT_DIR"), "/ggsans-Bold.ttf")),
                    "Bold",
                ),
                (
                    include_bytes!(concat!(env!("OUT_DIR"), "/ggsans-ExtraBold.ttf")),
                    "ExtraBold",
                ),
            ];
            let fonts: Vec<Cow<'static, [u8]>> = gg_sans_paths
                .iter()
                .map(|(data, _)| Cow::Borrowed(*data))
                .collect();
            if !fonts.is_empty() {
                if let Err(e) = cx.text_system().add_fonts(fonts) {
                    tracing::error!("Failed to register gg sans fonts: {e}");
                } else {
                    tracing::info!("Registered gg sans font ({} weights)", gg_sans_paths.len());
                }
            }

            init_ui(cx);

            mezon_ui::theme::set_theme(mezon_ui::theme::resolve_theme(&settings.theme), cx);

            if std::env::var("MEZON_DEV_GALLERY").is_ok() {
                open_dev_gallery_window(cx);
                return;
            }

            // Shared channel so background threads can send deep link URLs to the GPUI main thread.
            let (url_tx, mut url_rx) = futures::channel::mpsc::unbounded::<String>();

            // Listen for deep link URLs forwarded from secondary instances.
            {
                let tx = url_tx.clone();
                lock.listen_for_urls(move |url| {
                    let _ = tx.unbounded_send(url);
                });
            }

            // If we were launched with a deep link, inject it immediately.
            if let Some(url) = initial_url {
                let _ = url_tx.unbounded_send(url);
            }

            // Create the shared Settings entity so all views can observe theme changes.
            let settings_entity = cx.new(|_| Settings::load_sync());

            // Open the main window and obtain the auth_state entity handle.
            let auth_state_handle = open_main_window(
                cx,
                &settings,
                settings_entity,
                client.clone(),
                api.clone(),
                initial_auth_state,
            );

            {
                let auth_state = auth_state_handle.clone();
                cx.spawn(async move |cx: &mut AsyncApp| {
                    while let Some(url) = url_rx.next().await {
                        tracing::info!(
                            "Received deep link: {}",
                            url.split(['?', '#']).next().unwrap_or_default()
                        );
                        cx.update(|cx| {
                            if url.starts_with("mezonapp://callback") {
                                auth_state.update(cx, |state, cx| {
                                    // Deep link OAuth — kept for future use.
                                    *state = AuthState::AwaitingCallback;
                                    cx.notify();
                                });
                            } else if let Some(route) = mezon_ui::router::parse_link(&url) {
                                mezon_ui::router::navigate(cx, route);
                            }
                        });
                    }
                })
                .detach();
            }

            // Session refresh is server-pushed over the socket (`refresh_session_event`);
            // consume it and update the stored session (mezon-js `onrefreshsession`).
            spawn_session_refresh_consumer(cx, auth_state_handle.clone(), api.clone());
            spawn_transport_task(
                cx,
                auth_state_handle.clone(),
                transport.clone(),
                api.clone(),
            );

            // System tray.
            if let Some(tray) = setup_tray(cx, rt_handle.clone()) {
                cx.set_global(TrayGlobal(tray));
            }

            cx.activate(true);
        });
}

/// Keep the shared abridged TCP transport connected for authenticated UI API calls.
fn spawn_transport_task(
    cx: &mut App,
    auth_state: Entity<AuthState>,
    transport: Arc<TransportClient>,
    api: Arc<AppApi>,
) {
    // Wake signal that drives reconciliation — fired on auth-state changes and on socket
    // disconnect. Replaces the old 500ms poll (cf. Zed's reactive `client.status()` loop).
    let wake = std::sync::Arc::new(tokio::sync::Notify::new());
    cx.observe(&auth_state, {
        let wake = wake.clone();
        move |_, _| wake.notify_one()
    })
    .detach();

    cx.spawn(async move |cx: &mut AsyncApp| {
        let exec = cx.background_executor().clone();
        let mut connected_token: Option<String> = None;
        let mut retry_backoff_secs = 1u64;

        loop {
            let session = cx.update(|cx| match auth_state.read(cx).clone() {
                AuthState::Connecting(s) | AuthState::Authenticated(s) => Some(s),
                _ => None,
            });

            let Some(session) = session else {
                if connected_token.take().is_some() {
                    if let Err(e) = transport.close().await {
                        tracing::warn!("Failed to close TCP transport after logout: {e}");
                    }
                    api.set_status(ConnectionStatus::Disconnected);
                }
                retry_backoff_secs = 1;
                wake.notified().await;
                continue;
            };

            // Already connected with the right credential — park until something changes.
            if connected_token.as_deref() == Some(session.ws_credential())
                && transport.is_open().await
            {
                retry_backoff_secs = 1;
                wake.notified().await;
                continue;
            }

            let host = session
                .tcp_host
                .clone()
                .or(session.ws_host.clone())
                .unwrap_or_else(|| mezon_client::DEFAULT_WS_HOST.to_string());
            let port = resolve_tcp_port(&session);

            if transport.is_open().await
                && let Err(e) = transport.close().await
            {
                tracing::warn!("Failed to close stale transport: {e}");
            }

            tracing::info!("Connecting shared abridged TCP transport to {host}:{port}");
            api.set_status(ConnectionStatus::Connecting);
            // Socket credential: prefer the durable `session_id` (matches mezon-js), not the JWT.
            let token = session.ws_credential().to_string();
            let session_for_update = session.clone();
            let api_for_publish = api.clone();
            let api_for_close = api.clone();
            let wake_for_close = wake.clone();
            match transport
                .connect(
                    &host,
                    port,
                    &token,
                    move |event| {
                        api_for_publish.publish_event(event);
                    },
                    move |was_clean| {
                        if was_clean {
                            tracing::info!("TCP transport closed cleanly");
                        } else {
                            tracing::warn!("TCP transport closed with error");
                        }
                        // Reactive reconnect: flag disconnected + wake the manager loop.
                        api_for_close.set_status(ConnectionStatus::Disconnected);
                        wake_for_close.notify_one();
                    },
                )
                .await
            {
                Ok(()) => {
                    tracing::info!("Shared abridged TCP transport connected");

                    exec.timer(std::time::Duration::from_millis(250)).await;
                    match api.get_account().await {
                        Ok(account) => {
                            connected_token = Some(token);
                            retry_backoff_secs = 1;
                            api.set_status(ConnectionStatus::Connected);
                            tracing::info!("get_account over shared TCP succeeded");
                            tracing::info!("  User ID: {}", account.user_id);
                            tracing::info!("  Username: {}", account.username);
                            tracing::info!("  Email: {:?}", account.email);
                            tracing::info!("  Display name: {:?}", account.display_name);

                            cx.update(|cx| {
                                auth_state.update(cx, |state, cx| {
                                    if matches!(state, AuthState::Connecting(_)) {
                                        *state = AuthState::Authenticated(session_for_update);
                                        cx.notify();
                                    }
                                });
                            });
                        }
                        Err(e) => {
                            tracing::error!("get_account over shared TCP failed: {e}");
                            let _ = transport.close().await;
                            connected_token = None;
                            api.set_status(ConnectionStatus::Disconnected);
                            retry_backoff_secs = (retry_backoff_secs * 2).min(60);
                            promote_connecting_to_authenticated(
                                &auth_state,
                                session_for_update,
                                cx,
                            );
                            backoff_wait(&exec, &wake, retry_backoff_secs).await;
                        }
                    }
                }
                Err(e) => {
                    connected_token = None;
                    api.set_status(ConnectionStatus::Disconnected);
                    retry_backoff_secs = (retry_backoff_secs * 2).min(60);
                    tracing::error!("Shared abridged TCP transport connect failed: {e}");
                    promote_connecting_to_authenticated(&auth_state, session_for_update, cx);
                    backoff_wait(&exec, &wake, retry_backoff_secs).await;
                }
            }
        }
    })
    .detach();
}

/// Wait out a reconnect backoff, but wake early if auth/connection state changes.
async fn backoff_wait(exec: &gpui::BackgroundExecutor, wake: &tokio::sync::Notify, secs: u64) {
    tokio::select! {
        _ = wake.notified() => {}
        _ = exec.timer(std::time::Duration::from_secs(secs)) => {}
    }
}

/// Leave the loading screen if transport setup fails — user can retry from the app shell.
fn promote_connecting_to_authenticated(
    auth_state: &Entity<AuthState>,
    session: mezon_client::Session,
    cx: &mut AsyncApp,
) {
    cx.update(|cx| {
        auth_state.update(cx, |state, cx| {
            if matches!(state, AuthState::Connecting(_)) {
                *state = AuthState::Authenticated(session);
                cx.notify();
            }
        });
    });
}

/// Resolve abridged TCP port — session field, env override, then heuristics.
fn resolve_tcp_port(session: &mezon_client::Session) -> u16 {
    if let Some(port) = session.tcp_port {
        return port;
    }
    if let Some(port) = session.ws_port {
        return port;
    }
    if let Ok(v) = std::env::var("NX_CHAT_APP_TCP_PORT")
        && let Ok(port) = v.parse()
    {
        return port;
    }
    match session.tcp_host.as_deref().or(session.ws_host.as_deref()) {
        Some(h) if h.contains("dev-mezon") || h.contains("nccsoft.vn") => 7349,
        _ => 4433,
    }
}

/// Restore a stored session from the OS keychain.
///
/// - Stored session → `Connecting` (the socket validates it; the server pushes a fresh token
///   via `refresh_session_event`, so an expired JWT is fine — `session_id` is the durable cred)
/// - Nothing stored → `NotAuthenticated`
///
/// Note: we no longer REST-refresh on startup. That endpoint (`/v2/account/session/refresh`)
/// was removed server-side (it 404s); refresh is now socket-driven (see
/// [`spawn_session_refresh_consumer`]).
fn resolve_initial_auth_state() -> AuthState {
    match keychain::load_session() {
        None => {
            tracing::info!("No stored session — showing login");
            AuthState::NotAuthenticated
        }
        Some(session) => {
            tracing::info!(
                "Restored stored session for user_id={} — connecting",
                session.user_id
            );
            AuthState::Connecting(session)
        }
    }
}

/// Consume server-pushed `refresh_session_event`s from the socket and update the stored
/// session in place — the native equivalent of mezon-js `client.onrefreshsession`.
///
/// Replaces the old 60s REST-polling refresh task: the server now decides when to refresh
/// and pushes the new token/session_id over the realtime socket.
fn spawn_session_refresh_consumer(cx: &mut App, auth_state: Entity<AuthState>, api: Arc<AppApi>) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        let mut rx = api.subscribe();
        loop {
            match rx.recv().await {
                Ok(RealtimeEvent::SessionRefreshed(ev)) => {
                    tracing::info!("Session refreshed over socket for user_id={}", ev.user_id);
                    cx.update(|cx| {
                        auth_state.update(cx, |state, cx| match state {
                            AuthState::Authenticated(session) | AuthState::Connecting(session) => {
                                session.apply_refresh(&ev.token, &ev.refresh_token, &ev.session_id);
                                if let Err(e) = keychain::save_session(session) {
                                    tracing::warn!("Failed to persist refreshed session: {e}");
                                }
                                cx.notify();
                            }
                            _ => {}
                        });
                    });
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("Session refresh consumer lagged by {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
    .detach();
}

/// Open the main window and return a cloneable handle to the `AuthState` entity.
fn open_dev_gallery_window(cx: &mut App) {
    let options = WindowOptions {
        titlebar: Some(gpui::TitlebarOptions {
            title: Some("Mezon Component Gallery".into()),
            appears_transparent: false,
            ..Default::default()
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(900.0), px(800.0)),
            cx,
        ))),
        ..Default::default()
    };

    cx.open_window(options, |window, cx| {
        cx.new(|cx| mezon_ui::DevGallery::new(window, cx))
    })
    .unwrap_or_else(|e| {
        tracing::error!("Failed to open dev gallery window: {e}");
        std::process::exit(1);
    });

    cx.activate(true);
}

fn open_main_window(
    cx: &mut App,
    settings: &Settings,
    settings_entity: Entity<Settings>,
    client: Arc<MezonClient>,
    api: Arc<AppApi>,
    initial_auth: AuthState,
) -> Entity<AuthState> {
    let window_bounds = if let Some([x, y, w, h]) = settings.window_bounds {
        WindowBounds::Windowed(Bounds {
            origin: gpui::point(px(x as f32), px(y as f32)),
            size: size(px(w as f32), px(h as f32)),
        })
    } else {
        WindowBounds::Windowed(Bounds::centered(None, size(px(1280.0), px(720.0)), cx))
    };

    let options = WindowOptions {
        titlebar: Some(gpui::TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(gpui::point(px(-100.0), px(-100.0))),
        }),
        window_bounds: Some(window_bounds),
        window_min_size: Some(size(px(950.0), px(500.0))),
        kind: gpui::WindowKind::Normal,
        focus: true,
        show: true,
        ..Default::default()
    };

    // Smuggle the Entity out of the open_window closure via an Arc<Mutex<Option<_>>>.
    let auth_out = Arc::new(std::sync::Mutex::new(None::<Entity<AuthState>>));
    let auth_out_clone = auth_out.clone();

    cx.open_window(options, move |_window, cx| {
        let auth_state = cx.new(|_cx| initial_auth);
        *auth_out_clone.lock().unwrap_or_else(|e| e.into_inner()) = Some(auth_state.clone());

        let title_bar = cx.new(|cx| TitleBar::new(settings_entity.clone(), cx));

        // Register the domain stores as globals before any view reads them.
        // Order matters: ChannelList subscribes to ClanList's events.
        mezon_store::ClanList::init(api.clone(), cx);
        mezon_store::ChannelList::init(api.clone(), cx);

        cx.new(|cx| {
            RootView::new(
                title_bar,
                auth_state,
                client,
                api,
                settings_entity.clone(),
                cx,
            )
        })
    })
    .unwrap_or_else(|e| {
        tracing::error!("Failed to open main window: {e}");
        std::process::exit(1);
    });

    auth_out
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .expect("auth_state not initialised after open_window")
}

struct TrayGlobal(#[allow(dead_code)] mezon_native::tray::MezonTray);
impl gpui::Global for TrayGlobal {}

/// Create the system tray (best-effort — log a warning on failure).
fn setup_tray(
    cx: &mut App,
    rt_handle: Arc<tokio::runtime::Handle>,
) -> Option<mezon_native::tray::MezonTray> {
    let (quit_tx, mut quit_rx) = futures::channel::mpsc::unbounded::<()>();

    cx.spawn(async move |cx: &mut AsyncApp| {
        if quit_rx.next().await.is_some() {
            cx.update(|cx| cx.quit());
        }
    })
    .detach();

    // TODO Stage 2: store WindowHandle and bring window to front on on_show.
    match mezon_native::tray::MezonTray::new(
        || {
            tracing::debug!("Tray: Show Mezon");
        },
        move || {
            tracing::info!("Tray: Quit requested");
            let _ = quit_tx.unbounded_send(());
        },
        rt_handle,
    ) {
        Ok(tray) => {
            tracing::debug!("System tray initialised");
            Some(tray)
        }
        Err(e) => {
            tracing::warn!("Failed to create system tray: {e}");
            None
        }
    }
}
