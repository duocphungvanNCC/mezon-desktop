use std::sync::Arc;

use anyhow::Result;
use gpui::{App, AppContext, Entity, Global};
use mezon_client::{MezonClient, Session, keychain};

pub struct LoginStore {
    client: Arc<MezonClient>,
}

struct GlobalLoginStore(Entity<LoginStore>);
impl Global for GlobalLoginStore {}

impl LoginStore {
    pub fn init(client: Arc<MezonClient>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|_| Self { client });
        cx.set_global(GlobalLoginStore(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalLoginStore>().0.clone()
    }

    pub fn client(&self) -> Arc<MezonClient> {
        self.client.clone()
    }

    pub async fn request_otp(&self, email: &str) -> Result<String> {
        self.client.request_otp(email).await
    }

    pub async fn confirm_otp(&self, req_id: &str, otp_code: &str) -> Result<Session> {
        self.client.confirm_otp(req_id, otp_code).await
    }

    pub async fn authenticate_email(&self, email: &str, password: &str) -> Result<Session> {
        self.client.authenticate_email(email, password).await
    }

    pub fn persist_session(session: &Session) -> Result<()> {
        keychain::save_session(session)
    }
}
