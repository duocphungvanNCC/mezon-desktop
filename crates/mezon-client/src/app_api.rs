use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use image::GenericImageView;

use crate::{
    TransportClient,
    transport::{ApiAccount, ApiChannelDesc, ApiClanDesc, ApiMessage, RealtimeEvent},
};

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Connection lifecycle of the realtime transport — the analog of Zed's `client::Status`.
/// Exposed as a `watch` stream via [`AppApi::status`] so stores/UI react instead of polling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Clone)]
pub struct AppApi {
    transport: Arc<TransportClient>,
    realtime_tx: Arc<tokio::sync::broadcast::Sender<RealtimeEvent>>,
    status_tx: Arc<tokio::sync::watch::Sender<ConnectionStatus>>,
}

impl AppApi {
    pub fn new(transport: Arc<TransportClient>) -> Self {
        let (realtime_tx, _) = tokio::sync::broadcast::channel(256);
        let (status_tx, _) = tokio::sync::watch::channel(ConnectionStatus::Disconnected);
        Self {
            transport,
            realtime_tx: Arc::new(realtime_tx),
            status_tx: Arc::new(status_tx),
        }
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<RealtimeEvent> {
        self.realtime_tx.subscribe()
    }

    pub fn publish_event(&self, event: RealtimeEvent) {
        let _ = self.realtime_tx.send(event);
    }

    /// Watch the realtime connection status (cf. Zed `Client::status`). Reactive — no polling.
    pub fn status(&self) -> tokio::sync::watch::Receiver<ConnectionStatus> {
        self.status_tx.subscribe()
    }

    /// Current connection status snapshot.
    pub fn connection_status(&self) -> ConnectionStatus {
        *self.status_tx.borrow()
    }

    /// Update the connection status — called by the transport connection manager.
    pub fn set_status(&self, status: ConnectionStatus) {
        let _ = self.status_tx.send(status);
    }

    pub async fn get_account(&self) -> Result<ApiAccount> {
        self.transport.get_account().await
    }

    pub async fn list_channel_descs(&self, clan_id: &str) -> Result<Vec<ApiChannelDesc>> {
        self.transport.list_channel_descs(clan_id).await
    }

    pub async fn list_channel_by_user_id(&self) -> Result<Vec<ApiChannelDesc>> {
        self.transport.list_channel_by_user_id().await
    }

    pub async fn list_clan_descs(&self) -> Result<Vec<ApiClanDesc>> {
        self.transport.list_clan_descs().await
    }

    pub async fn create_clan_desc(
        &self,
        clan_name: &str,
        logo: &str,
        banner: &str,
    ) -> Result<ApiClanDesc> {
        self.transport
            .create_clan_desc(clan_name, logo, banner)
            .await
    }

    pub async fn is_open(&self) -> bool {
        self.transport.is_open().await
    }

    pub async fn ping_roundtrip(&self) -> Result<()> {
        self.transport.ping_roundtrip().await
    }

    pub async fn list_channel_messages(
        &self,
        clan_id: &str,
        channel_id: &str,
        message_id: &str,
        direction: i32,
        limit: u32,
    ) -> Result<Vec<ApiMessage>> {
        self.transport
            .list_channel_messages(clan_id, channel_id, message_id, direction, limit)
            .await
    }

    pub async fn join_chat(
        &self,
        clan_id: &str,
        channel_id: &str,
        channel_type: i32,
        is_public: bool,
    ) -> Result<()> {
        self.transport
            .join_chat(clan_id, channel_id, channel_type, is_public)
            .await
    }

    pub async fn send_channel_message(
        &self,
        clan_id: &str,
        channel_id: &str,
        content: &str,
        is_public: bool,
    ) -> Result<ApiMessage> {
        self.transport
            .send_channel_message(clan_id, channel_id, content, is_public)
            .await
    }

    pub async fn update_user(&self, display_name: &str, avatar_url: &str) -> Result<()> {
        self.transport.update_user(display_name, avatar_url).await
    }

    pub async fn update_account(
        &self,
        display_name: Option<&str>,
        avatar_url: Option<&str>,
        about_me: Option<&str>,
    ) -> Result<()> {
        self.transport
            .update_account(display_name, avatar_url, about_me)
            .await
    }

    pub async fn upload_attachment_file(
        &self,
        filename: &str,
        filetype: &str,
        size: i32,
        width: i32,
        height: i32,
    ) -> Result<mezon_proto::api::UploadAttachment> {
        self.transport
            .upload_attachment_file(filename, filetype, size, width, height)
            .await
    }

    /// Full avatar upload flow: get pre-signed URL, PUT file bytes, return permanent URL.
    pub async fn get_user_clan_profile(
        &self,
        clan_id: &str,
    ) -> Result<mezon_proto::api::ClanProfile> {
        self.transport.get_user_profile_on_clan(clan_id).await
    }

    pub async fn update_user_clan_profile(
        &self,
        clan_id: &str,
        nick_name: &str,
        avatar_url: Option<&str>,
    ) -> Result<()> {
        self.transport
            .update_user_profile_by_clan(clan_id, nick_name, avatar_url)
            .await
    }

    pub async fn check_duplicate_clan_nickname(
        &self,
        clan_id: &str,
        nick_name: &str,
    ) -> Result<bool> {
        let condition_id: i64 = clan_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid clan_id {clan_id:?}: {e}"))?;
        let resp = self
            .transport
            .check_duplicate_name(nick_name, 4, condition_id)
            .await?;
        Ok(resp.is_duplicate)
    }

    pub async fn upload_avatar(&self, path: &Path) -> Result<String> {
        tracing::info!("upload_avatar: reading file path={:?}", path);
        let data = crate::transport_runtime::read_file(path.to_path_buf()).await?;

        let raw_filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("avatar")
            .to_string();
        let filename = sanitize_filename(&raw_filename);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_string();
        let filetype = format!("image/{}", ext);
        let size = data.len() as i32;

        let (width, height) = image::load_from_memory(&data)
            .ok()
            .map(|img| {
                let dims = img.dimensions();
                tracing::info!(
                    "upload_avatar: detected image dimensions {}x{}",
                    dims.0,
                    dims.1
                );
                dims
            })
            .unwrap_or((0, 0));

        tracing::info!(
            "upload_avatar: file read ok filename={} filetype={} size={} width={} height={}",
            filename,
            filetype,
            size,
            width,
            height
        );

        tracing::info!("upload_avatar: requesting presigned URL");
        let upload = self
            .transport
            .upload_attachment_file(&filename, &filetype, size, width as i32, height as i32)
            .await?;
        tracing::info!("upload_avatar: presigned URL received url={}", upload.url);

        tracing::info!("upload_avatar: PUTing file bytes to presigned URL");
        crate::transport_runtime::put_bytes_to_url(&upload.url, data).await?;
        tracing::info!("upload_avatar: PUT completed successfully");

        let permanent_url = upload
            .url
            .split('?')
            .next()
            .unwrap_or(&upload.url)
            .to_string();

        tracing::info!("Avatar upload complete: url={}", permanent_url);

        Ok(permanent_url)
    }

    pub async fn list_loged_device(&self) -> Result<Vec<mezon_proto::api::LogedDevice>> {
        self.transport.list_loged_device().await
    }

    pub async fn session_logout(&self, token: &str, refresh_token: &str) -> Result<()> {
        self.transport.session_logout(token, refresh_token).await
    }

    pub async fn logout_device(
        &self,
        token: &str,
        refresh_token: &str,
        device_id: &str,
    ) -> Result<()> {
        self.transport
            .logout_device(token, refresh_token, device_id)
            .await
    }
}
