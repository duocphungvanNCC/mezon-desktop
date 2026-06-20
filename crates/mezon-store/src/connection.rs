//! Connection / session lifecycle store — the gpui-coupled manager that keeps the abridged-TCP
//! transport connected, drives reconnect with backoff, and applies server-pushed session refresh.
//!
//! In Zed this lives in the `client` crate; `mezon-client` is gpui-free (pure transport), so the
//! gpui-coupled lifecycle lives here as a store instead of in the app binary.

use std::sync::Arc;

use gpui::{
    App, AppContext, AsyncApp, BackgroundExecutor, Context, Entity, Global, Subscription, Task,
};
use mezon_client::{
    AppApi, ConnectionStatus, NetworkMonitor, RealtimeEvent, Session, TransportClient, keychain,
};

use crate::AuthState;
use crate::realtime::{RealtimeDispatch, RealtimeKind};

const HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
const CONNECT_CONFIRM_GRACE: std::time::Duration = std::time::Duration::from_secs(1);

/// Owns the transport connection manager task + the auth-state observation. Registered as a
/// [`Global`] so it lives for the process; the held [`Task`]/[`Subscription`] cancel on drop.
pub struct ConnectionStore {
    online: bool,
    _manager: Task<()>,
    _auth_observe: Subscription,
    _heartbeat: Task<()>,
    _online_watch: Task<()>,
    _network: NetworkMonitor,
}

struct GlobalConnectionStore(Entity<ConnectionStore>);
impl Global for GlobalConnectionStore {}

impl ConnectionStore {
    /// Spawn the connection manager and register session-refresh. Call **after** the realtime
    /// dispatcher and the `auth_state` entity exist.
    pub fn init(
        transport: Arc<TransportClient>,
        api: Arc<AppApi>,
        auth_state: Entity<AuthState>,
        cx: &mut App,
    ) -> Entity<Self> {
        let entity = cx.new(|cx| Self::new(transport, api, auth_state, cx));
        cx.set_global(GlobalConnectionStore(entity.clone()));
        entity
    }

    pub fn is_online(&self) -> bool {
        self.online
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalConnectionStore>().0.clone()
    }

    fn new(
        transport: Arc<TransportClient>,
        api: Arc<AppApi>,
        auth_state: Entity<AuthState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (connect_ack_tx, connect_ack_rx) = tokio::sync::watch::channel(0u64);
        Self::register_session_refresh(&auth_state, connect_ack_tx, cx);

        // Wake signal that drives reconciliation — fired on auth-state changes and on socket
        // disconnect. Replaces the old 500ms poll (cf. Zed's reactive `client.status()` loop).
        let wake = Arc::new(tokio::sync::Notify::new());
        let auth_observe = cx.observe(&auth_state, {
            let wake = wake.clone();
            move |_, _, _| wake.notify_one()
        });

        let network = NetworkMonitor::new();
        let online = network.is_online();
        let online_watch = {
            let wake = wake.clone();
            let mut online_rx = network.online();
            cx.spawn(async move |this, cx| {
                while online_rx.changed().await.is_ok() {
                    let is_online = *online_rx.borrow();
                    if is_online {
                        wake.notify_one();
                    }
                    if this
                        .update(cx, |store, cx| {
                            store.online = is_online;
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
        };

        let heartbeat = Self::spawn_heartbeat(transport.clone(), api.clone(), wake.clone(), cx);

        let manager = cx.spawn(async move |_this, cx| {
            let exec = cx.background_executor().clone();
            let mut connected_user_id: Option<String> = None;
            let mut retry_backoff_secs = 1u64;
            let mut connect_ack_rx = connect_ack_rx;

            loop {
                let session = cx.update(|cx| match auth_state.read(cx).clone() {
                    AuthState::Connecting(s) | AuthState::Authenticated(s) => Some(s),
                    _ => None,
                });

                let Some(session) = session else {
                    if connected_user_id.take().is_some() {
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
                if connected_user_id.as_deref() == Some(session.user_id.as_str())
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
                let api_for_publish = api.clone();
                let api_for_close = api.clone();
                let wake_for_close = wake.clone();
                connect_ack_rx.borrow_and_update();
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

                        let confirmed = {
                            let signaled = tokio::select! {
                                res = connect_ack_rx.changed() => res.is_ok(),
                                _ = exec.timer(CONNECT_CONFIRM_GRACE) => true,
                            };
                            signaled && transport.is_open().await
                        };

                        if confirmed {
                            connected_user_id = Some(session.user_id.clone());
                            retry_backoff_secs = 1;
                            api.set_status(ConnectionStatus::Connected);
                            tracing::info!("Connection confirmed — handshake accepted");

                            cx.update(|cx| {
                                auth_state.update(cx, |state, cx| {
                                    if let AuthState::Connecting(s) = state {
                                        let session = s.clone();
                                        *state = AuthState::Authenticated(session);
                                        cx.notify();
                                    }
                                });
                            });
                        } else {
                            tracing::warn!(
                                "Connection not confirmed — handshake rejected or dropped"
                            );
                            let _ = transport.close().await;
                            connected_user_id = None;
                            api.set_status(ConnectionStatus::Disconnected);
                            retry_backoff_secs = (retry_backoff_secs * 2).min(60);
                            promote_connecting_to_authenticated(&auth_state, cx);
                            backoff_wait(&exec, &wake, retry_backoff_secs).await;
                        }
                    }
                    Err(e) => {
                        connected_user_id = None;
                        api.set_status(ConnectionStatus::Disconnected);
                        retry_backoff_secs = (retry_backoff_secs * 2).min(60);
                        tracing::error!("Shared abridged TCP transport connect failed: {e}");
                        promote_connecting_to_authenticated(&auth_state, cx);
                        backoff_wait(&exec, &wake, retry_backoff_secs).await;
                    }
                }
            }
        });

        Self {
            online,
            _manager: manager,
            _auth_observe: auth_observe,
            _heartbeat: heartbeat,
            _online_watch: online_watch,
            _network: network,
        }
    }

    fn spawn_heartbeat(
        transport: Arc<TransportClient>,
        api: Arc<AppApi>,
        wake: Arc<tokio::sync::Notify>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let exec = cx.background_executor().clone();
        exec.clone().spawn(async move {
            loop {
                exec.timer(HEARTBEAT_INTERVAL).await;
                if !transport.is_open().await {
                    continue;
                }
                if let Err(e) = transport.ping_roundtrip().await {
                    tracing::warn!("heartbeat ping failed ({e}) — forcing reconnect");
                    let _ = transport.close().await;
                    api.set_status(ConnectionStatus::Disconnected);
                    wake.notify_one();
                }
            }
        })
    }

    /// Apply server-pushed `refresh_session_event`s and persist the refreshed session — the
    /// native equivalent of mezon-js `client.onrefreshsession`.
    fn register_session_refresh(
        auth_state: &Entity<AuthState>,
        connect_ack_tx: tokio::sync::watch::Sender<u64>,
        cx: &mut Context<Self>,
    ) {
        RealtimeDispatch::global(cx).update(cx, |dispatch, _| {
            dispatch.on(
                RealtimeKind::SessionRefreshed,
                auth_state,
                move |state, event, cx| {
                    let RealtimeEvent::SessionRefreshed(ev) = event else {
                        return;
                    };
                    connect_ack_tx.send_modify(|n| *n = n.wrapping_add(1));
                    if ev.session_id.is_empty() {
                        return;
                    }
                    tracing::info!("Session refreshed over socket for user_id={}", ev.user_id);
                    match state {
                        AuthState::Authenticated(session) | AuthState::Connecting(session) => {
                            session.session_id = ev.session_id.clone();
                            if let Err(e) = keychain::save_session(session) {
                                tracing::warn!("Failed to persist refreshed session: {e}");
                            }
                            cx.notify();
                        }
                        _ => {}
                    }
                },
            );
        });
    }
}

/// Wait out a reconnect backoff, but wake early if auth/connection state changes.
async fn backoff_wait(exec: &BackgroundExecutor, wake: &tokio::sync::Notify, secs: u64) {
    let base_ms = secs.saturating_mul(1000);
    let jitter_cap = (base_ms / 4).max(1);
    let jitter_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos()) % jitter_cap)
        .unwrap_or(0);
    let delay = std::time::Duration::from_millis(base_ms + jitter_ms);
    tokio::select! {
        _ = wake.notified() => {}
        _ = exec.timer(delay) => {}
    }
}

/// Leave the loading screen if transport setup fails — user can retry from the app shell.
fn promote_connecting_to_authenticated(auth_state: &Entity<AuthState>, cx: &mut AsyncApp) {
    cx.update(|cx| {
        auth_state.update(cx, |state, cx| {
            if let AuthState::Connecting(s) = state {
                let session = s.clone();
                *state = AuthState::Authenticated(session);
                cx.notify();
            }
        });
    });
}

/// Resolve abridged TCP port — session field, env override, then heuristics.
fn resolve_tcp_port(session: &Session) -> u16 {
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
/// - Stored session → `Connecting` (the socket validates it; the server pushes a fresh token via
///   `refresh_session_event`, so an expired JWT is fine — `session_id` is the durable cred).
/// - Nothing stored → `NotAuthenticated`.
pub fn resolve_initial_auth_state() -> AuthState {
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
