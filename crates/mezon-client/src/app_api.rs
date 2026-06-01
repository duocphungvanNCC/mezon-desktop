use std::sync::Arc;

use anyhow::Result;

use crate::{
    TransportClient,
    transport::{ApiAccount, ApiChannelDesc, ApiClanDesc, ApiMessage},
};

#[derive(Clone)]
pub struct AppApi {
    transport: Arc<TransportClient>,
}

impl AppApi {
    pub fn new(transport: Arc<TransportClient>) -> Self {
        Self { transport }
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
        limit: u32,
    ) -> Result<Vec<ApiMessage>> {
        self.transport
            .list_channel_messages(clan_id, channel_id, limit)
            .await
    }

    pub async fn send_channel_message(
        &self,
        clan_id: &str,
        channel_id: &str,
        content: &str,
    ) -> Result<ApiMessage> {
        self.transport
            .send_channel_message(clan_id, channel_id, content)
            .await
    }
}
