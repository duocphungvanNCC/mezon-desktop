use crate::components::primitives::{Icon, IconName, Label, Switch, h_flex, v_flex};
use crate::theme::ActiveTheme;
use gpui::{ClipboardItem, Context, Entity, Window, div, prelude::*, px};
use mezon_store::{McpServerStatus, PlatformStore, Settings};
use std::time::Duration;
use ui::Tooltip;

const MCP_STDIO_CONFIG: &str = r#"{
  "mcpServers": {
    "mezon": {
      "type": "stdio",
      "command": "mezon",
      "args": {stdio_args}
    }
  }
}"#;

const MCP_HTTP_CONFIG: &str = r#"{
  "mcpServers": {
    "mezon-http": {
      "type": "http",
      "url": "http://127.0.0.1:{port}/mcp"
    }
  }
}"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum McpCopyTarget {
    Url,
    HttpConfig,
    StdioConfig,
}

pub struct AdvancedPage {
    settings: Entity<Settings>,
    cli_busy: bool,
    mcp_busy: bool,
    mcp_status: McpServerStatus,
    mcp_copy_feedback: Option<McpCopyTarget>,
}

impl AdvancedPage {
    pub fn new(settings: Entity<Settings>, cx: &mut Context<Self>) -> Self {
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        let mcp_status = PlatformStore::try_global(cx)
            .map(|platform| platform.read(cx).mcp_server_status())
            .unwrap_or_default();
        Self {
            settings,
            cli_busy: false,
            mcp_busy: false,
            mcp_status,
            mcp_copy_feedback: None,
        }
    }

    fn mark_mcp_copied(&mut self, target: McpCopyTarget, cx: &mut Context<Self>) {
        self.mcp_copy_feedback = Some(target);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            this.update(cx, |this, cx| {
                if this.mcp_copy_feedback == Some(target) {
                    this.mcp_copy_feedback = None;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn copy_mcp_url(&mut self, url: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(url.to_string()));
        self.mark_mcp_copied(McpCopyTarget::Url, cx);
    }

    fn copy_mcp_http_config(&mut self, cx: &mut Context<Self>) {
        let config = mcp_http_config_content(&self.mcp_status);
        cx.write_to_clipboard(ClipboardItem::new_string(config));
        self.mark_mcp_copied(McpCopyTarget::HttpConfig, cx);
    }

    fn copy_mcp_stdio_config(&mut self, read_only: bool, cx: &mut Context<Self>) {
        let config = mcp_stdio_config_content(read_only);
        cx.write_to_clipboard(ClipboardItem::new_string(config));
        self.mark_mcp_copied(McpCopyTarget::StdioConfig, cx);
    }

    fn toggle_cli_install(&mut self, cx: &mut Context<Self>) {
        if self.cli_busy {
            return;
        }
        let Some(platform) = PlatformStore::try_global(cx) else {
            return;
        };
        let Some(toggle) = platform.read(cx).cli_install_toggle_fn() else {
            return;
        };
        self.cli_busy = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { toggle() })
                .await;

            this.update(cx, |this, cx| {
                this.cli_busy = false;
                if let Err(error) = result {
                    tracing::warn!("CLI install toggle failed: {error}");
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toggle_mcp_server(&mut self, cx: &mut Context<Self>) {
        if self.mcp_busy {
            return;
        }
        let Some(platform) = PlatformStore::try_global(cx) else {
            return;
        };
        let platform = platform.read(cx);
        let running = self.mcp_status.running;
        let read_only = self.settings.read(cx).mcp_read_only;
        let stop_fn = if running {
            platform.mcp_server_stop_fn()
        } else {
            None
        };
        let start_fn = if running {
            None
        } else {
            platform.mcp_server_start_fn()
        };
        if stop_fn.is_none() && start_fn.is_none() {
            return;
        }
        self.mcp_busy = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Some(stop) = stop_fn {
                        stop()
                    } else if let Some(start) = start_fn {
                        start(read_only)
                    } else {
                        Err(anyhow::anyhow!("MCP server hooks unavailable"))
                    }
                })
                .await;

            this.update(cx, |this, cx| {
                this.mcp_busy = false;
                match result {
                    Ok(status) => this.mcp_status = status,
                    Err(error) => tracing::warn!("MCP server toggle failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toggle_mcp_read_only(&mut self, cx: &mut Context<Self>) {
        if self.mcp_busy {
            return;
        }
        let was_running = self.mcp_status.running;
        let next_read_only = !self.settings.read(cx).mcp_read_only;
        self.settings.update(cx, |settings, _| {
            settings.mcp_read_only = next_read_only;
        });
        mezon_store::schedule_settings_save(&self.settings, cx);

        if !was_running {
            cx.notify();
            return;
        }

        let Some(platform) = PlatformStore::try_global(cx) else {
            cx.notify();
            return;
        };
        let platform = platform.read(cx);
        let Some(start) = platform.mcp_server_start_fn() else {
            cx.notify();
            return;
        };
        let Some(stop) = platform.mcp_server_stop_fn() else {
            cx.notify();
            return;
        };

        self.mcp_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    stop()?;
                    start(next_read_only)
                })
                .await;

            this.update(cx, |this, cx| {
                this.mcp_busy = false;
                match result {
                    Ok(status) => this.mcp_status = status,
                    Err(error) => tracing::warn!("MCP read-only toggle failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn mcp_server_url(status: &McpServerStatus) -> Option<String> {
    if !status.running {
        return None;
    }
    status.url.clone().or_else(|| {
        status
            .port
            .map(|port| format!("http://127.0.0.1:{port}/mcp"))
    })
}

fn mcp_stdio_server(read_only: bool) -> String {
    let stdio_args = if read_only {
        "[\"mcp\", \"stdio\", \"--read-only\"]"
    } else {
        "[\"mcp\", \"stdio\"]"
    };
    MCP_STDIO_CONFIG.replace("{stdio_args}", stdio_args)
}

fn mcp_http_server(status: &McpServerStatus) -> String {
    let port = status
        .port
        .filter(|_| status.running)
        .map(|port| port.to_string())
        .unwrap_or_else(|| "{port}".to_string());
    MCP_HTTP_CONFIG.replace("{port}", &port)
}

fn mcp_stdio_config_content(read_only: bool) -> String {
    mcp_stdio_server(read_only)
}

fn mcp_http_config_content(status: &McpServerStatus) -> String {
    mcp_http_server(status)
}

fn render_mcp_copy_button(
    theme: &crate::theme::Theme,
    locale: &str,
    copy_id: &'static str,
    copied: bool,
    on_copy: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(copy_id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded_md()
        .cursor_pointer()
        .hover(|el| el.bg(theme.bg_hover))
        .tooltip(Tooltip::text(mezon_i18n::t(locale, "common.copy")))
        .child(
            Icon::new(if copied {
                IconName::Check
            } else {
                IconName::CopyIcon
            })
            .size(px(16.))
            .text_color(if copied {
                theme.status_online
            } else {
                theme.text_muted
            }),
        )
        .on_click(on_copy)
}

fn render_mcp_config_box(
    theme: &crate::theme::Theme,
    config: &str,
    locale: &str,
    copy_id: &'static str,
    copied: bool,
    on_copy: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let config_for_label = config.to_string();
    div().rounded_md().bg(theme.bg_secondary).p_3().child(
        h_flex()
            .items_start()
            .gap_2()
            .child(
                div().flex_1().min_w_0().child(
                    Label::new(config_for_label)
                        .text_sm()
                        .text_color(theme.text_muted),
                ),
            )
            .child(render_mcp_copy_button(
                theme, locale, copy_id, copied, on_copy,
            )),
    )
}

fn render_mcp_url_box(
    theme: &crate::theme::Theme,
    url: &str,
    locale: &str,
    copied: bool,
    on_copy: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let url_for_label = url.to_string();
    h_flex()
        .items_center()
        .gap_2()
        .rounded_md()
        .bg(theme.bg_secondary)
        .p_3()
        .child(
            div().flex_1().min_w_0().overflow_hidden().child(
                Label::new(url_for_label)
                    .text_sm()
                    .text_color(theme.text_primary),
            ),
        )
        .child(render_mcp_copy_button(
            theme,
            locale,
            "mcp-url-copy",
            copied,
            on_copy,
        ))
}

fn setting_group(
    theme: &crate::theme::Theme,
    title: &str,
    description: &str,
    body: impl IntoElement,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(Label::new(title).text_color(theme.text_primary))
        .child(
            Label::new(description)
                .text_sm()
                .text_color(theme.text_muted),
        )
        .child(
            v_flex()
                .rounded_lg()
                .bg(theme.bg_primary)
                .p_4()
                .gap_2()
                .child(body),
        )
}

fn toggle_setting_row(
    theme: &crate::theme::Theme,
    title: &str,
    description: &str,
    switch: impl IntoElement,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(Label::new(title).text_color(theme.text_primary))
                .child(switch),
        )
        .child(
            Label::new(description)
                .text_sm()
                .text_color(theme.text_muted),
        )
}

impl Render for AdvancedPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let locale = self.settings.read(cx).language.clone();
        let hw_accel = self.settings.read(cx).hardware_acceleration;
        let mcp_read_only = self.settings.read(cx).mcp_read_only;
        let platform = PlatformStore::try_global(cx);
        let cli_visible = platform
            .as_ref()
            .is_some_and(|store| store.read(cx).cli_install_visible());
        let cli_installed = platform
            .as_ref()
            .is_some_and(|store| store.read(cx).cli_install_installed());
        let mcp_available = platform
            .as_ref()
            .is_some_and(|store| store.read(cx).mcp_server_available());
        let cli_busy = self.cli_busy;
        let mcp_busy = self.mcp_busy;
        let mcp_running = self.mcp_status.running;
        let mcp_server_url = mcp_server_url(&self.mcp_status);
        let url_copied = self.mcp_copy_feedback == Some(McpCopyTarget::Url);
        let http_config_copied = self.mcp_copy_feedback == Some(McpCopyTarget::HttpConfig);
        let stdio_config_copied = self.mcp_copy_feedback == Some(McpCopyTarget::StdioConfig);

        let mut page = v_flex().gap_6().child(toggle_setting_row(
            &theme,
            mezon_i18n::t(&locale, "setting.advanced.hardwareAcceleration"),
            mezon_i18n::t(&locale, "setting.advanced.hardwareAccelerationDesc"),
            Switch::new("hardware-acceleration")
                .checked(hw_accel)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.settings.update(cx, |s, _| {
                        s.hardware_acceleration = !s.hardware_acceleration;
                    });
                    mezon_store::schedule_settings_save(&this.settings, cx);
                    cx.notify();
                })),
        ));

        if cli_visible {
            page = page.child(toggle_setting_row(
                &theme,
                mezon_i18n::t(&locale, "setting.advanced.cliInstall.title"),
                mezon_i18n::t(&locale, "setting.advanced.cliInstall.desc"),
                Switch::new("cli-install")
                    .checked(cli_installed)
                    .disabled(cli_busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_cli_install(cx);
                    })),
            ));
        }

        if mcp_available {
            let mut mcp_content = v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(
                            Label::new(mezon_i18n::t(
                                &locale,
                                "setting.advanced.mcp.httpServer.title",
                            ))
                            .text_color(theme.text_primary),
                        )
                        .child(
                            Switch::new("mcp-http-server")
                                .checked(mcp_running)
                                .disabled(mcp_busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_mcp_server(cx);
                                })),
                        ),
                )
                .child(
                    Label::new(mezon_i18n::t(
                        &locale,
                        "setting.advanced.mcp.httpServer.desc",
                    ))
                    .text_sm()
                    .text_color(theme.text_muted),
                );

            if let Some(url) = mcp_server_url {
                let url_for_display = url.clone();
                let url_for_copy = url;
                mcp_content = mcp_content.child(render_mcp_url_box(
                    &theme,
                    &url_for_display,
                    &locale,
                    url_copied,
                    cx.listener(move |this, _, _, cx| {
                        this.copy_mcp_url(&url_for_copy, cx);
                    }),
                ));
            }

            mcp_content = mcp_content
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(
                            Label::new(mezon_i18n::t(
                                &locale,
                                "setting.advanced.mcp.readOnly.title",
                            ))
                            .text_color(theme.text_primary),
                        )
                        .child(
                            Switch::new("mcp-read-only")
                                .checked(mcp_read_only)
                                .disabled(mcp_busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_mcp_read_only(cx);
                                })),
                        ),
                )
                .child(
                    Label::new(mezon_i18n::t(&locale, "setting.advanced.mcp.readOnly.desc"))
                        .text_sm()
                        .text_color(theme.text_muted),
                );

            if mcp_running {
                mcp_content = mcp_content
                    .child(
                        Label::new(mezon_i18n::t(
                            &locale,
                            "setting.advanced.mcp.httpConfig.title",
                        ))
                        .text_color(theme.text_primary),
                    )
                    .child(
                        Label::new(mezon_i18n::t(
                            &locale,
                            "setting.advanced.mcp.httpConfig.desc",
                        ))
                        .text_sm()
                        .text_color(theme.text_muted),
                    )
                    .child(render_mcp_config_box(
                        &theme,
                        &mcp_http_config_content(&self.mcp_status),
                        &locale,
                        "mcp-http-config-template",
                        http_config_copied,
                        cx.listener(|this, _, _, cx| {
                            this.copy_mcp_http_config(cx);
                        }),
                    ));
            }

            if cli_installed {
                mcp_content = mcp_content
                    .child(
                        Label::new(mezon_i18n::t(&locale, "setting.advanced.mcp.stdio.title"))
                            .text_color(theme.text_primary),
                    )
                    .child(
                        Label::new(mezon_i18n::t(&locale, "setting.advanced.mcp.stdio.desc"))
                            .text_sm()
                            .text_color(theme.text_muted),
                    )
                    .child(render_mcp_config_box(
                        &theme,
                        &mcp_stdio_config_content(mcp_read_only),
                        &locale,
                        "mcp-stdio-config-template",
                        stdio_config_copied,
                        cx.listener(move |this, _, _, cx| {
                            this.copy_mcp_stdio_config(mcp_read_only, cx);
                        }),
                    ));
            }

            page = page.child(setting_group(
                &theme,
                mezon_i18n::t(&locale, "setting.advanced.mcp.title"),
                mezon_i18n::t(&locale, "setting.advanced.mcp.desc"),
                mcp_content,
            ));
        }

        page
    }
}
