use futures::StreamExt as _;
use gpui::{App, AsyncApp};
use mezon_client::AppApi;
use mezon_mcp::{
    CaptureTarget, McpCommand, McpController, McpStartParams, ToolCallParams, is_write_tool,
    list_tools_json,
};
use mezon_native::control::{ControlHandler, ControlRequest, ControlResponse, ControlServer};
use mezon_store::{AuthState, LoginStore, Settings};
use serde_json::{Value, json};
use std::sync::Arc;

pub struct McpRuntime {
    controller: Arc<McpController>,
    _control_server: Option<ControlServer>,
}

impl McpRuntime {
    pub fn controller(&self) -> Arc<McpController> {
        self.controller.clone()
    }

    pub fn start(
        api: Arc<AppApi>,
        mcp_cmd_tx: futures::channel::mpsc::UnboundedSender<McpCommand>,
    ) -> anyhow::Result<Self> {
        let controller = Arc::new(McpController::new());
        let controller_for_handler = controller.clone();
        let runtime_handle = mezon_client::transport_runtime::handle();
        let runtime_for_handler = runtime_handle.clone();

        let handler: ControlHandler = Arc::new(move |request| {
            let controller = controller_for_handler.clone();
            let request_id = request.id;
            match runtime_for_handler.block_on(handle_control_request(controller, request)) {
                Ok(response) => response,
                Err(error) => ControlResponse::err(request_id, error.to_string()),
            }
        });

        let control_server = if mezon_native::control::control_server_enabled() {
            let server = ControlServer::bind(handler)?;
            server.run_in_background();
            Some(server)
        } else {
            None
        };

        let controller_for_backend = controller.clone();
        let api_for_backend = api.clone();
        let ui_tx_for_backend = mcp_cmd_tx.clone();
        runtime_handle.spawn(async move {
            controller_for_backend
                .set_backend(api_for_backend, ui_tx_for_backend)
                .await;
        });

        Ok(Self {
            controller,
            _control_server: control_server,
        })
    }

    pub fn spawn_ui_bridge(
        cx: &mut App,
        auth_state: gpui::Entity<AuthState>,
        settings: gpui::Entity<Settings>,
        mut mcp_cmd_rx: futures::channel::mpsc::UnboundedReceiver<McpCommand>,
    ) {
        cx.spawn(async move |cx: &mut AsyncApp| {
            while let Some(command) = mcp_cmd_rx.next().await {
                match command {
                    McpCommand::GetContext { reply } => {
                        let value = cx.update(|cx| build_context(cx, &auth_state));
                        let _ = reply.send(value);
                    }
                    McpCommand::Navigate { path, reply } => {
                        let result = cx.update(|cx| navigate_path(cx, &path));
                        let _ = reply.send(result);
                    }
                    McpCommand::Logout { reply } => {
                        let result = cx.update(|cx| {
                            LoginStore::global(cx).update(cx, |store, cx| store.logout(cx));
                            Ok(())
                        });
                        let _ = reply.send(result);
                    }
                    McpCommand::Refresh { reply } => {
                        let result = cx.update(refresh_app);
                        let _ = reply.send(result);
                    }
                    McpCommand::Quit { reply } => {
                        let result = cx.update(|cx| {
                            cx.quit();
                            Ok(())
                        });
                        let _ = reply.send(result);
                    }
                    McpCommand::ShowWindow { reply } => {
                        let result = cx.update(mezon_ui::app::capture::show_main_window);
                        let _ = reply.send(result);
                    }
                    McpCommand::Capture { target, reply } => {
                        let result = cx.update(|cx| match target {
                            CaptureTarget::Window => mezon_ui::app::capture::capture_png(cx, false),
                            CaptureTarget::Chat => mezon_ui::app::capture::capture_png(cx, true),
                        });
                        let _ = reply.send(result);
                    }
                    McpCommand::GoBack { reply } => {
                        let result = cx.update(mezon_ui::app::capture::go_back);
                        let _ = reply.send(result);
                    }
                    McpCommand::GoForward { reply } => {
                        let result = cx.update(mezon_ui::app::capture::go_forward);
                        let _ = reply.send(result);
                    }
                    McpCommand::GetSettings { reply } => {
                        let value = cx.update(|cx| build_settings(cx, &settings, &auth_state));
                        let _ = reply.send(value);
                    }
                    McpCommand::SetSetting { key, value, reply } => {
                        let result = cx.update(|cx| set_setting(cx, &settings, &key, value));
                        let _ = reply.send(result);
                    }
                    McpCommand::SetCliEnabled { enabled, reply } => {
                        let result = cx.update(|_| set_cli_enabled(enabled));
                        let _ = reply.send(result);
                    }
                }
            }
        })
        .detach();
    }
}

async fn handle_control_request(
    controller: Arc<McpController>,
    request: ControlRequest,
) -> anyhow::Result<ControlResponse> {
    match request.method.as_str() {
        "mcp.start" => {
            let params: McpStartParams = serde_json::from_value(request.params)?;
            let result = controller.start(params.read_only, params.port).await?;
            Ok(ControlResponse::ok(
                request.id,
                serde_json::to_value(result)?,
            ))
        }
        "mcp.status" => {
            let status = controller.status().await;
            Ok(ControlResponse::ok(
                request.id,
                serde_json::to_value(status)?,
            ))
        }
        "mcp.stop" => {
            controller.stop().await?;
            Ok(ControlResponse::ok(request.id, Value::Null))
        }
        "tool.call" => {
            let params: ToolCallParams = serde_json::from_value(request.params)?;
            let read_only = controller.status().await.read_only;
            if read_only && is_write_tool(&params.name) {
                return Ok(ControlResponse::err(
                    request.id,
                    format!("Tool {} is disabled in read-only mode", params.name),
                ));
            }
            let result = controller.call_tool(&params.name, params.arguments).await?;
            Ok(ControlResponse::ok(request.id, result))
        }
        "tools.list" => {
            let read_only = request
                .params
                .get("read_only")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Ok(ControlResponse::ok(request.id, list_tools_json(read_only)))
        }
        "app.logout" => {
            let result = controller.call_tool("logout", Value::Null).await?;
            Ok(ControlResponse::ok(request.id, result))
        }
        "app.refresh" => {
            let result = controller.call_tool("refresh", Value::Null).await?;
            Ok(ControlResponse::ok(request.id, result))
        }
        "app.quit" => {
            let result = controller.call_tool("quit_app", Value::Null).await?;
            Ok(ControlResponse::ok(request.id, result))
        }
        "app.status" => {
            let result = controller.call_tool("get_app_info", Value::Null).await?;
            Ok(ControlResponse::ok(request.id, result))
        }
        other => Ok(ControlResponse::err(
            request.id,
            format!("unknown control method: {other}"),
        )),
    }
}

fn build_context(cx: &App, auth_state: &gpui::Entity<AuthState>) -> Value {
    let router = mezon_ui::router::Router::global(cx).read(cx);
    let route = router.current_path();
    let auth = match auth_state.read(cx) {
        AuthState::NotAuthenticated => "not_authenticated",
        AuthState::AwaitingCallback => "awaiting_callback",
        AuthState::Connecting(_) => "connecting",
        AuthState::Authenticated(session) => {
            return json!({
                "route": route,
                "auth": "authenticated",
                "user_id": session.user_id,
                "clan_id": parse_route_field(&route, "clan"),
                "channel_id": parse_route_field(&route, "channel").or_else(|| parse_route_field(&route, "direct")),
            });
        }
        AuthState::OtpRequested { .. } => "otp_requested",
    };
    json!({
        "route": route,
        "auth": auth,
        "clan_id": parse_route_field(&route, "clan"),
        "channel_id": parse_route_field(&route, "channel").or_else(|| parse_route_field(&route, "direct")),
    })
}

fn parse_route_field(route: &str, kind: &str) -> Option<String> {
    let segments: Vec<_> = route.split('/').filter(|s| !s.is_empty()).collect();
    match kind {
        "clan" => segments
            .windows(2)
            .find(|w| w[0] == "clans")
            .map(|w| w[1].to_string()),
        "channel" => segments
            .windows(2)
            .find(|w| w[0] == "channels")
            .map(|w| w[1].to_string()),
        "direct" => segments
            .windows(2)
            .find(|w| w[0] == "message")
            .map(|w| w[1].to_string()),
        _ => None,
    }
}

fn navigate_path(cx: &mut App, path: &str) -> anyhow::Result<()> {
    let route = mezon_ui::router::Route::from_path(path);
    mezon_ui::router::navigate(cx, route);
    Ok(())
}

fn refresh_app(cx: &mut App) -> anyhow::Result<()> {
    if let Some(store) = mezon_store::ClanList::try_global(cx) {
        store.update(cx, |store, cx| store.reload(cx));
    }
    if let Some(store) = mezon_store::DirectMessageStore::try_global(cx) {
        store.update(cx, |store, cx| store.refresh(cx));
    }
    if let Some(store) = mezon_store::MessagesStore::try_global(cx) {
        store.update(cx, |store, cx| store.refresh(cx));
    }
    Ok(())
}

fn build_settings(
    cx: &App,
    settings: &gpui::Entity<Settings>,
    auth_state: &gpui::Entity<AuthState>,
) -> Value {
    let s = settings.read(cx);
    let voice = mezon_store::VoiceStore::try_global(cx).map(|store| {
        let store = store.read(cx);
        json!({
            "in_call": store.connection().active_channel_id().is_some(),
            "channel_label": store.channel_label(),
            "mic_enabled": store.mic_enabled(),
            "camera_enabled": store.camera_enabled(),
        })
    });
    json!({
        "theme": s.theme,
        "language": s.language,
        "zoom_factor": s.zoom_factor,
        "notifications_enabled": s.notifications_enabled,
        "activity_tracking": s.activity_tracking,
        "hardware_acceleration": s.hardware_acceleration,
        "cli_enabled": mezon_native::cli_install::is_installed(),
        "cli_visible": mezon_native::cli_install::menu_visible(),
        "auth": match auth_state.read(cx) {
            AuthState::Authenticated(_) => "authenticated",
            _ => "not_authenticated",
        },
        "voice": voice,
    })
}

fn set_setting(
    cx: &mut App,
    settings: &gpui::Entity<Settings>,
    key: &str,
    value: Value,
) -> anyhow::Result<()> {
    match key {
        "theme" => {
            let Some(theme) = value.as_str() else {
                anyhow::bail!("theme must be a string");
            };
            let entity = settings.clone();
            settings.update(cx, |settings, cx| {
                settings.theme = theme.to_string();
                mezon_store::schedule_settings_save(&entity, cx);
                cx.notify();
            });
        }
        "language" => {
            let Some(language) = value.as_str() else {
                anyhow::bail!("language must be a string");
            };
            let entity = settings.clone();
            settings.update(cx, |settings, cx| {
                settings.language = language.to_string();
                mezon_store::schedule_settings_save(&entity, cx);
                cx.notify();
            });
        }
        "zoom_factor" => {
            let Some(zoom) = value.as_f64() else {
                anyhow::bail!("zoom_factor must be a number");
            };
            let entity = settings.clone();
            settings.update(cx, |settings, cx| {
                settings.zoom_factor = zoom as f32;
                mezon_store::schedule_settings_save(&entity, cx);
                cx.notify();
            });
        }
        "notifications_enabled" | "activity_tracking" => {
            let Some(enabled) = value.as_bool() else {
                anyhow::bail!("{key} must be a boolean");
            };
            let entity = settings.clone();
            settings.update(cx, |settings, cx| {
                if key == "notifications_enabled" {
                    settings.notifications_enabled = enabled;
                } else {
                    settings.activity_tracking = enabled;
                }
                mezon_store::schedule_settings_save(&entity, cx);
                cx.notify();
            });
        }
        other => anyhow::bail!("unsupported setting key: {other}"),
    }
    Ok(())
}

fn set_cli_enabled(enabled: bool) -> anyhow::Result<bool> {
    if enabled {
        mezon_native::cli_install::install()?;
        Ok(true)
    } else {
        mezon_native::cli_install::uninstall()?;
        Ok(false)
    }
}
