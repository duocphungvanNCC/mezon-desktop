use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, SharedString};
use mezon_client::AppApi;
use mezon_client::transport::ApiChannelAttachment;

use crate::gallery::resolve_attachment_uploader;
use crate::ids::{ChannelId, ClanId, MessageId, UserId};

pub const FILES_CACHE_TTL: Duration = Duration::from_secs(60 * 60);
pub const FILES_PAGE_SIZE: i32 = 100;
pub const FILES_FILE_TYPE: &str = "FILE";

#[derive(Debug, Clone)]
pub struct ChannelDocument {
    pub id: i64,
    pub channel_id: ChannelId,
    pub clan_id: ClanId,
    pub message_id: MessageId,
    pub uploader_id: UserId,
    pub url: String,
    pub filename: String,
    pub filetype: String,
    pub create_time_seconds: u32,
    pub uploader_name: SharedString,
}

impl ChannelDocument {
    pub fn from_api(api: ApiChannelAttachment, channel_id: ChannelId, clan_id: ClanId) -> Self {
        let filename = if api.filename.is_empty() {
            "File".to_string()
        } else {
            api.filename
        };
        let filetype = if api.filetype.is_empty() {
            "File".to_string()
        } else {
            api.filetype
        };
        Self {
            id: api.id,
            channel_id,
            clan_id,
            message_id: MessageId(api.message_id),
            uploader_id: UserId(api.uploader),
            url: api.url,
            filename,
            filetype,
            create_time_seconds: api.create_time_seconds,
            uploader_name: SharedString::default(),
        }
    }

    pub fn is_failed(&self) -> bool {
        self.filename == "failAttachment"
    }
}

pub fn is_document(filetype: &str) -> bool {
    let ft = filetype.trim();
    if ft.is_empty() {
        return true;
    }
    let lower = ft.to_ascii_lowercase();
    if lower == "sticker" {
        return false;
    }
    if lower.starts_with("image/") || lower.starts_with("video/") {
        return false;
    }
    if lower.contains("mp4") || lower.contains("mov") {
        return false;
    }
    true
}

#[derive(Default)]
struct FilesChannel {
    documents: Vec<ChannelDocument>,
    is_loading: bool,
    fetch_error: bool,
    fetched_at: Option<Instant>,
}

impl FilesChannel {
    fn is_fresh(&self) -> bool {
        self.fetched_at
            .is_some_and(|t| t.elapsed() < FILES_CACHE_TTL)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FilesEvent {
    Changed(ChannelId),
}

pub struct FilesStore {
    by_channel: HashMap<ChannelId, FilesChannel>,
    api: Arc<AppApi>,
}

struct GlobalFilesStore(Entity<FilesStore>);
impl Global for GlobalFilesStore {}

impl EventEmitter<FilesEvent> for FilesStore {}

impl FilesStore {
    pub fn init(api: Arc<AppApi>, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|_| Self {
            by_channel: HashMap::new(),
            api,
        });
        cx.set_global(GlobalFilesStore(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalFilesStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalFilesStore>().map(|g| g.0.clone())
    }

    pub fn documents(&self, channel_id: ChannelId) -> &[ChannelDocument] {
        self.by_channel
            .get(&channel_id)
            .map(|c| c.documents.as_slice())
            .unwrap_or(&[])
    }

    pub fn is_loading(&self, channel_id: ChannelId) -> bool {
        self.by_channel
            .get(&channel_id)
            .is_some_and(|c| c.is_loading)
    }

    pub fn fetch_error(&self, channel_id: ChannelId) -> bool {
        self.by_channel
            .get(&channel_id)
            .is_some_and(|c| c.fetch_error)
    }

    pub fn is_empty(&self, channel_id: ChannelId) -> bool {
        self.documents(channel_id).is_empty()
    }

    pub fn ensure_loaded(
        &mut self,
        clan_id: ClanId,
        channel_id: ChannelId,
        cx: &mut Context<Self>,
    ) {
        let needs_fetch = match self.by_channel.get(&channel_id) {
            Some(c) => !c.is_loading && (c.documents.is_empty() || !c.is_fresh() || c.fetch_error),
            None => true,
        };
        if needs_fetch {
            self.fetch(clan_id, channel_id, cx);
        }
    }

    pub fn refresh(&mut self, clan_id: ClanId, channel_id: ChannelId, cx: &mut Context<Self>) {
        self.fetch(clan_id, channel_id, cx);
    }

    pub fn clear_channel(&mut self, channel_id: ChannelId, cx: &mut Context<Self>) {
        if self.by_channel.remove(&channel_id).is_some() {
            cx.emit(FilesEvent::Changed(channel_id));
            cx.notify();
        }
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        if self.by_channel.is_empty() {
            return;
        }
        let channel_ids: Vec<ChannelId> = self.by_channel.keys().copied().collect();
        self.by_channel.clear();
        for channel_id in channel_ids {
            cx.emit(FilesEvent::Changed(channel_id));
        }
        cx.notify();
    }

    pub fn refresh_uploaders(&mut self, channel_id: ChannelId, cx: &mut Context<Self>) {
        if !self.by_channel.contains_key(&channel_id) {
            return;
        }
        self.enrich_channel(channel_id, cx);
        cx.emit(FilesEvent::Changed(channel_id));
        cx.notify();
    }

    fn fetch(&mut self, clan_id: ClanId, channel_id: ChannelId, cx: &mut Context<Self>) {
        let entry = self.by_channel.entry(channel_id).or_default();
        entry.is_loading = true;
        entry.fetch_error = false;
        cx.emit(FilesEvent::Changed(channel_id));
        cx.notify();

        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .list_channel_attachments(
                    clan_id.0,
                    channel_id.0,
                    FILES_FILE_TYPE,
                    0,
                    FILES_PAGE_SIZE,
                    0,
                    0,
                )
                .await;
            let mapped = result.map(|list| {
                let mut docs: Vec<ChannelDocument> = list
                    .into_iter()
                    .filter(|a| is_document(&a.filetype))
                    .map(|a| ChannelDocument::from_api(a, channel_id, clan_id))
                    .collect();
                sort_desc_in_place(&mut docs);
                dedupe_by_id(docs)
            });
            let _ = this.update(cx, |this, cx| {
                let entry = this.by_channel.entry(channel_id).or_default();
                entry.is_loading = false;
                match mapped {
                    Ok(docs) => {
                        entry.documents = docs;
                        entry.fetched_at = Some(Instant::now());
                        entry.fetch_error = false;
                        this.enrich_channel(channel_id, cx);
                    }
                    Err(e) => {
                        tracing::error!("list_channel_attachments (FILE) failed: {e}");
                        entry.fetch_error = true;
                    }
                }
                cx.emit(FilesEvent::Changed(channel_id));
                cx.notify();
            });
        })
        .detach();
    }

    fn enrich_channel(&mut self, channel_id: ChannelId, cx: &App) {
        let Some(entry) = self.by_channel.get_mut(&channel_id) else {
            return;
        };
        for doc in entry.documents.iter_mut() {
            let info = resolve_attachment_uploader(
                doc.clan_id,
                doc.channel_id,
                doc.uploader_id,
                doc.message_id,
                None,
                cx,
            );
            doc.uploader_name = if info.name.is_empty() {
                SharedString::from("Unknown")
            } else {
                info.name.into()
            };
        }
    }
}

fn sort_desc_in_place(items: &mut [ChannelDocument]) {
    items.sort_by(|a, b| {
        a.create_time_seconds
            .cmp(&b.create_time_seconds)
            .reverse()
            .then_with(|| a.id.cmp(&b.id).reverse())
    });
}

fn dedupe_by_id(docs: Vec<ChannelDocument>) -> Vec<ChannelDocument> {
    let mut seen = std::collections::HashSet::new();
    docs.into_iter().filter(|doc| seen.insert(doc.id)).collect()
}

pub fn short_file_type_label(filetype: &str) -> SharedString {
    let ft = filetype.trim();
    if ft.is_empty() || ft.eq_ignore_ascii_case("file") {
        return "FILE".into();
    }
    if ft == "application/vnd.android.package-archive" {
        return "FILE".into();
    }
    let lower = ft.to_ascii_lowercase();
    let label = match lower.as_str() {
        "application/pdf" => "PDF",
        "text/csv" | "application/csv" => "CSV",
        "text/plain" => "TXT",
        "text/markdown" => "MD",
        "application/json" => "JSON",
        "application/zip" | "application/x-zip-compressed" => "ZIP",
        "application/vnd.rar" | "application/x-rar-compressed" => "RAR",
        "application/x-7z-compressed" => "7Z",
        "application/msword"
        | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        | "docx" => "DOC",
        "application/vnd.ms-excel"
        | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        | "xlsx" => "XLS",
        "application/vnd.ms-powerpoint"
        | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        | "pptx" => "PPT",
        _ => {
            if let Some(ext) = lower.rsplit('/').next() {
                if ext.len() <= 8 && !ext.is_empty() {
                    return SharedString::from(ext.to_ascii_uppercase());
                }
            }
            "FILE"
        }
    };
    label.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_document_excludes_media_and_sticker() {
        assert!(!is_document("image/png"));
        assert!(!is_document("video/mp4"));
        assert!(!is_document("sticker"));
        assert!(!is_document("application/mp4"));
        assert!(is_document("application/pdf"));
        assert!(is_document("text/csv"));
        assert!(is_document("FILE"));
        assert!(is_document(""));
        assert!(is_document("application/zip"));
    }

    #[test]
    fn from_api_defaults_empty_names() {
        let api = ApiChannelAttachment {
            id: 1,
            filename: String::new(),
            filetype: String::new(),
            ..ApiChannelAttachment::default()
        };
        let doc = ChannelDocument::from_api(api, ChannelId(2), ClanId(3));
        assert_eq!(doc.filename, "File");
        assert_eq!(doc.filetype, "File");
        assert_eq!(doc.channel_id, ChannelId(2));
        assert_eq!(doc.clan_id, ClanId(3));
    }

    #[test]
    fn sort_desc_orders_by_time_then_id() {
        let mut items = vec![
            ChannelDocument {
                id: 1,
                channel_id: ChannelId(1),
                clan_id: ClanId(1),
                message_id: MessageId(0),
                uploader_id: UserId(0),
                url: String::new(),
                filename: "a".into(),
                filetype: "FILE".into(),
                create_time_seconds: 10,
                uploader_name: SharedString::default(),
            },
            ChannelDocument {
                id: 3,
                channel_id: ChannelId(1),
                clan_id: ClanId(1),
                message_id: MessageId(0),
                uploader_id: UserId(0),
                url: String::new(),
                filename: "b".into(),
                filetype: "FILE".into(),
                create_time_seconds: 20,
                uploader_name: SharedString::default(),
            },
            ChannelDocument {
                id: 2,
                channel_id: ChannelId(1),
                clan_id: ClanId(1),
                message_id: MessageId(0),
                uploader_id: UserId(0),
                url: String::new(),
                filename: "c".into(),
                filetype: "FILE".into(),
                create_time_seconds: 20,
                uploader_name: SharedString::default(),
            },
        ];
        sort_desc_in_place(&mut items);
        assert_eq!(items[0].id, 3);
        assert_eq!(items[1].id, 2);
        assert_eq!(items[2].id, 1);
    }

    #[test]
    fn short_file_type_label_maps_common_mimes() {
        assert_eq!(short_file_type_label("application/pdf").as_ref(), "PDF");
        assert_eq!(short_file_type_label("text/csv").as_ref(), "CSV");
        assert_eq!(short_file_type_label("FILE").as_ref(), "FILE");
        assert_eq!(
            short_file_type_label("application/vnd.android.package-archive").as_ref(),
            "FILE"
        );
    }

    #[test]
    fn dedupe_by_id_keeps_first() {
        let docs = vec![
            ChannelDocument {
                id: 1,
                channel_id: ChannelId(1),
                clan_id: ClanId(1),
                message_id: MessageId(0),
                uploader_id: UserId(0),
                url: String::new(),
                filename: "a".into(),
                filetype: "FILE".into(),
                create_time_seconds: 20,
                uploader_name: SharedString::default(),
            },
            ChannelDocument {
                id: 1,
                channel_id: ChannelId(1),
                clan_id: ClanId(1),
                message_id: MessageId(0),
                uploader_id: UserId(0),
                url: String::new(),
                filename: "b".into(),
                filetype: "FILE".into(),
                create_time_seconds: 10,
                uploader_name: SharedString::default(),
            },
            ChannelDocument {
                id: 2,
                channel_id: ChannelId(1),
                clan_id: ClanId(1),
                message_id: MessageId(0),
                uploader_id: UserId(0),
                url: String::new(),
                filename: "c".into(),
                filetype: "FILE".into(),
                create_time_seconds: 5,
                uploader_name: SharedString::default(),
            },
        ];
        let deduped = dedupe_by_id(docs);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].id, 1);
        assert_eq!(deduped[0].filename, "a");
        assert_eq!(deduped[1].id, 2);
    }
}
