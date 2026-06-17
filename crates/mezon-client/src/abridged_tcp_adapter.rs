use crate::tls_crypto;
use anyhow::Result;
use async_trait::async_trait;
use prost::Message;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_rustls::rustls::pki_types::ServerName;

#[cfg(debug_assertions)]
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
#[cfg(debug_assertions)]
use tokio_rustls::rustls::pki_types::{CertificateDer, UnixTime};
#[cfg(debug_assertions)]
use tokio_rustls::rustls::{DigitallySignedStruct, SignatureScheme};

#[cfg(debug_assertions)]
#[derive(Debug)]
struct NoCertVerifier;

#[cfg(debug_assertions)]
impl ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, tokio_rustls::rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _m: &[u8],
        _c: &CertificateDer<'_>,
        _d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _m: &[u8],
        _c: &CertificateDer<'_>,
        _d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
        ]
    }
}

use crate::transport_adapter::{AdapterHandlers, TransportAdapter};

fn build_client_config() -> tokio_rustls::rustls::ClientConfig {
    tls_crypto::ensure_crypto_provider();

    #[cfg(debug_assertions)]
    if matches!(
        std::env::var("MEZON_DANGER_ACCEPT_INVALID_CERTS").as_deref(),
        Ok("1") | Ok("true")
    ) {
        tracing::warn!(
            "TLS certificate verification DISABLED via MEZON_DANGER_ACCEPT_INVALID_CERTS (debug build only)"
        );
        return tokio_rustls::rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertVerifier))
            .with_no_client_auth();
    }

    let mut root_store = tokio_rustls::rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    tokio_rustls::rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth()
}

const CODE_FIN: u16 = 0xff;
const PREFIX_RAW: u8 = 0xff;
const PREFIX_EXTENDED: u8 = 0x7f;
const RAW_HEADER_LENGTH: usize = 7;
const RAW_CHUNK_PAYLOAD_LEN: usize = 4096;
const MAX_REALTIME_FRAME_LEN: usize = 1 << 20;

fn read_varint(buf: &[u8]) -> Option<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0u32;
    for (i, &b) in buf.iter().enumerate() {
        if shift >= 64 {
            return None;
        }
        value |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some((value, i + 1));
        }
        shift += 7;
    }
    None
}

fn protobuf_message_len(buf: &[u8]) -> Option<usize> {
    let mut pos = 0usize;
    while pos < buf.len() {
        let (tag, tag_len) = read_varint(&buf[pos..])?;
        let field = tag >> 3;
        let wire = tag & 7;
        if field == 0 || matches!(wire, 3 | 4 | 6 | 7) {
            return Some(pos);
        }
        let value_start = pos + tag_len;
        let value_end = match wire {
            0 => {
                let (_, n) = read_varint(buf.get(value_start..)?)?;
                value_start + n
            }
            1 => value_start + 8,
            5 => value_start + 4,
            2 => {
                let (len, n) = read_varint(buf.get(value_start..)?)?;
                if len as usize > MAX_REALTIME_FRAME_LEN {
                    return Some(pos);
                }
                value_start + n + len as usize
            }
            _ => return Some(pos),
        };
        if value_end > buf.len() {
            return None;
        }
        pos = value_end;
    }
    Some(pos)
}

fn frame_is_valid_envelope(framed: &[u8]) -> bool {
    let unpadded_end = framed.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    mezon_proto::realtime::Envelope::decode(&framed[..unpadded_end]).is_ok()
}

fn envelope_prefix_plausible(payload: &[u8]) -> bool {
    match read_varint(payload) {
        None => true,
        Some((tag, _)) => {
            let field = tag >> 3;
            let wire = tag & 7;
            match field {
                0 => false,
                1 => wire == 0,
                _ => wire == 2,
            }
        }
    }
}

fn frame_kind(first: u8) -> &'static str {
    match first {
        0x00 => "ping/pong",
        0x82 => "ws-binary",
        0x80 | 0x81 | 0x83..=0x8f => "ws-other",
        0xef => "handshake",
        PREFIX_EXTENDED => "abridged-ext",
        PREFIX_RAW => "raw/api",
        0x01..=0x7e => "abridged",
        _ => "unknown",
    }
}

fn envelope_kind(envelope: &mezon_proto::realtime::Envelope) -> &'static str {
    use mezon_proto::realtime::envelope::Message as M;
    match &envelope.message {
        None => "<empty>",
        Some(M::ChannelMessage(_)) => "ChannelMessage",
        Some(M::MessageTypingEvent(_)) => "MessageTyping",
        Some(M::ChannelPresenceEvent(_)) => "ChannelPresence",
        Some(M::StatusPresenceEvent(_)) => "StatusPresence",
        Some(M::CustomStatusEvent(_)) => "CustomStatus",
        Some(M::MessageReactionEvent(_)) => "MessageReaction",
        Some(M::MarkAsRead(_)) => "MarkAsRead",
        Some(M::ChannelCreatedEvent(_)) => "ChannelCreated",
        Some(M::ChannelUpdatedEvent(_)) => "ChannelUpdated",
        Some(M::ChannelDeletedEvent(_)) => "ChannelDeleted",
        Some(M::VoiceStartedEvent(_)) => "VoiceStarted",
        Some(M::VoiceEndedEvent(_)) => "VoiceEnded",
        Some(M::VoiceJoinedEvent(_)) => "VoiceJoined",
        Some(M::VoiceLeavedEvent(_)) => "VoiceLeaved",
        Some(M::UserChannelAddedEvent(_)) => "UserChannelAdded",
        Some(M::UserChannelRemovedEvent(_)) => "UserChannelRemoved",
        Some(M::AddClanUserEvent(_)) => "AddClanUser",
        Some(M::UserClanRemovedEvent(_)) => "UserClanRemoved",
        Some(M::ClanUpdatedEvent(_)) => "ClanUpdated",
        Some(M::ClanProfileUpdatedEvent(_)) => "ClanProfileUpdated",
        Some(M::ClanDeletedEvent(_)) => "ClanDeleted",
        Some(M::AddFriend(_)) => "AddFriend",
        Some(M::RemoveFriend(_)) => "RemoveFriend",
        Some(M::RefreshSessionEvent(_)) => "RefreshSession",
        Some(M::ApiRequestEvent(_)) => "ApiRequest",
        Some(M::ChannelJoin(_)) => "ChannelJoin",
        Some(M::Error(_)) => "Error",
        Some(_) => "Other",
    }
}

fn content_snippet(content: &str) -> String {
    let text = serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|v| v.get("t").and_then(|t| t.as_str().map(str::to_owned)))
        .unwrap_or_else(|| content.to_owned());
    let one_line = text.replace('\n', " ");
    match one_line.char_indices().nth(120) {
        Some((idx, _)) => format!("{}...", &one_line[..idx]),
        None => one_line,
    }
}

fn envelope_detail(envelope: &mezon_proto::realtime::Envelope) -> String {
    use mezon_proto::realtime::envelope::Message as M;
    match &envelope.message {
        Some(M::ChannelMessage(m)) => format!(
            "ChannelMessage id={} ch={} from={} \"{}\"",
            m.message_id,
            m.channel_id,
            m.sender_id,
            content_snippet(&m.content)
        ),
        Some(M::ApiRequestEvent(e)) => format!("ApiRequest({})", e.api_name),
        Some(M::Error(e)) => format!("Error code={} {}", e.code, e.message),
        _ => envelope_kind(envelope).to_string(),
    }
}

type TlsStream = tokio_rustls::client::TlsStream<TcpStream>;

struct IoLoopState {
    handlers: Arc<Mutex<AdapterHandlers>>,
    streams: Arc<Mutex<HashMap<u16, Vec<Vec<u8>>>>>,
    is_connected: Arc<Mutex<bool>>,
    read_buffer: Arc<Mutex<Vec<u8>>>,
    pending_raw: Arc<Mutex<Option<Vec<u8>>>>,
}

pub struct AbridgedTcpAdapter {
    write_tx: Arc<Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>>,
    handlers: Arc<Mutex<AdapterHandlers>>,
    streams: Arc<Mutex<HashMap<u16, Vec<Vec<u8>>>>>,
    is_connected: Arc<Mutex<bool>>,
    read_buffer: Arc<Mutex<Vec<u8>>>,
    pending_raw: Arc<Mutex<Option<Vec<u8>>>>,
}

impl AbridgedTcpAdapter {
    pub fn new() -> Self {
        Self {
            write_tx: Arc::new(Mutex::new(None)),
            handlers: Arc::new(Mutex::new(AdapterHandlers::default())),
            streams: Arc::new(Mutex::new(HashMap::new())),
            is_connected: Arc::new(Mutex::new(false)),
            read_buffer: Arc::new(Mutex::new(Vec::new())),
            pending_raw: Arc::new(Mutex::new(None)),
        }
    }

    async fn handle_data(&self, incoming: Vec<u8>) -> Result<()> {
        if incoming.is_empty() {
            return Ok(());
        }

        // Check for a pending 0xff FIN header that was waiting for its body.
        // A 0xff FIN body is always protobuf (starts with byte < 0x80), so if
        // the incoming data starts with PREFIX_RAW (0xff) it is a *new* message
        // and the pending header must be completed with an empty payload.
        {
            let mut pending = self.pending_raw.lock().await;
            if let Some(header) = pending.take() {
                if incoming[0] == PREFIX_RAW {
                    let cid = u16::from_be_bytes([header[1], header[2]]);
                    let code = u32::from_be_bytes([header[3], header[4], header[5], header[6]]);
                    let response_code = (code >> 16) & 0xffff;
                    tracing::info!("Completing pending 0-length response for cid={}", cid);
                    let handlers = self.handlers.lock().await.clone();
                    handlers.trigger_message(cid, response_code, vec![]);
                } else {
                    // Continuation — prepend the pending header to incoming data
                    let mut combined = header;
                    combined.extend(&incoming);
                    drop(pending);
                    self.read_buffer.lock().await.extend(combined);
                    return self.process_raw_buffer().await;
                }
            }
        }

        self.read_buffer.lock().await.extend(incoming);
        self.process_raw_buffer().await
    }

    async fn process_raw_buffer(&self) -> Result<()> {
        let handlers = self.handlers.lock().await.clone();

        loop {
            let msg_bytes = {
                let mut buf = self.read_buffer.lock().await;
                if buf.is_empty() {
                    return Ok(());
                }

                let first_byte = buf[0];
                if first_byte == 0x00 {
                    if buf.len() < 3 {
                        return Ok(());
                    }
                    let msg = buf[..3].to_vec();
                    buf.drain(..3);
                    Some((msg, false))
                } else if first_byte == PREFIX_RAW {
                    if buf.len() < RAW_HEADER_LENGTH {
                        return Ok(());
                    }
                    let cid = u16::from_be_bytes([buf[1], buf[2]]);
                    let code = u32::from_be_bytes([buf[3], buf[4], buf[5], buf[6]]);
                    let response_code = (code >> 16) & 0xffff;
                    let fin_flag = (code & 0xffff) as u16;

                    if fin_flag != CODE_FIN {
                        if buf.len() < RAW_HEADER_LENGTH + RAW_CHUNK_PAYLOAD_LEN {
                            return Ok(());
                        }
                        let payload = buf
                            [RAW_HEADER_LENGTH..RAW_HEADER_LENGTH + RAW_CHUNK_PAYLOAD_LEN]
                            .to_vec();
                        let buffered = {
                            let mut streams = self.streams.lock().await;
                            let chunks = streams.entry(cid).or_default();
                            chunks.push(payload);
                            chunks.iter().map(Vec::len).sum::<usize>()
                        };
                        if buffered > MAX_REALTIME_FRAME_LEN {
                            tracing::warn!(
                                "resync: cid={cid} buffered {buffered} bytes exceeds cap, dropping stream"
                            );
                            self.streams.lock().await.remove(&cid);
                        }
                        buf.drain(..RAW_HEADER_LENGTH + RAW_CHUNK_PAYLOAD_LEN);
                        None
                    } else {
                        let mut streams = self.streams.lock().await;
                        let prefix_len = streams
                            .get(&cid)
                            .map(|c| c.iter().map(Vec::len).sum::<usize>())
                            .unwrap_or(0);
                        if prefix_len == 0 {
                            drop(streams);
                            if buf.len() == RAW_HEADER_LENGTH {
                                // Header-only FIN: save as pending — body may arrive in next TCP read
                                *self.pending_raw.lock().await = Some(buf.clone());
                                buf.drain(..RAW_HEADER_LENGTH);
                                let pending_raw = self.pending_raw.clone();
                                let handlers = self.handlers.lock().await.clone();
                                tokio::spawn(async move {
                                    tokio::time::sleep(Duration::from_millis(100)).await;
                                    let mut pending = pending_raw.lock().await;
                                    if let Some(header) = pending.take() {
                                        let cid = u16::from_be_bytes([header[1], header[2]]);
                                        let code = u32::from_be_bytes([
                                            header[3], header[4], header[5], header[6],
                                        ]);
                                        let response_code = (code >> 16) & 0xffff;
                                        handlers.trigger_message(cid, response_code, vec![]);
                                    }
                                });
                                return Ok(());
                            }
                            let body_len = match protobuf_message_len(&buf[RAW_HEADER_LENGTH..]) {
                                Some(n) => n,
                                None => return Ok(()),
                            };
                            let total = RAW_HEADER_LENGTH + body_len;
                            let msg = buf[..total].to_vec();
                            buf.drain(..total);
                            Some((msg, false))
                        } else {
                            let mut combined =
                                streams.get(&cid).map(|c| c.concat()).unwrap_or_default();
                            combined.extend_from_slice(&buf[RAW_HEADER_LENGTH..]);
                            match protobuf_message_len(&combined) {
                                Some(total) if total >= prefix_len => {
                                    let fin_bytes = total - prefix_len;
                                    combined.truncate(total);
                                    tracing::info!(
                                        "Complete API response (chunked): cid={cid} code={response_code} len={total} bytes"
                                    );
                                    handlers.trigger_message(cid, response_code, combined);
                                    streams.remove(&cid);
                                    drop(streams);
                                    buf.drain(..RAW_HEADER_LENGTH + fin_bytes);
                                    None
                                }
                                Some(_) => {
                                    tracing::warn!(
                                        "resync: cid={cid} chunked stream corrupt, dropping"
                                    );
                                    streams.remove(&cid);
                                    drop(streams);
                                    buf.drain(..1);
                                    None
                                }
                                None => return Ok(()),
                            }
                        }
                    }
                } else if first_byte < 127 {
                    let payload_len = first_byte as usize * 4;
                    let total = 1 + payload_len;
                    if buf.len() < total {
                        if envelope_prefix_plausible(&buf[1..]) {
                            return Ok(());
                        }
                        tracing::debug!("resync: implausible incomplete byte {first_byte:#x}");
                        buf.drain(..1);
                        None
                    } else if frame_is_valid_envelope(&buf[1..total]) {
                        let msg = buf[..total].to_vec();
                        buf.drain(..total);
                        Some((msg, false))
                    } else {
                        tracing::debug!("resync: skip misaligned byte {first_byte:#x}");
                        buf.drain(..1);
                        None
                    }
                } else if first_byte == PREFIX_EXTENDED {
                    if buf.len() < 4 {
                        return Ok(());
                    }
                    let payload_len = u32::from_le_bytes([buf[1], buf[2], buf[3], 0]) as usize * 4;
                    let total = 4 + payload_len;
                    if total > MAX_REALTIME_FRAME_LEN {
                        tracing::debug!("resync: extended frame len {total} too large");
                        buf.drain(..1);
                        None
                    } else if buf.len() < total {
                        if envelope_prefix_plausible(&buf[4..]) {
                            return Ok(());
                        }
                        tracing::debug!("resync: implausible incomplete extended frame");
                        buf.drain(..1);
                        None
                    } else if frame_is_valid_envelope(&buf[4..total]) {
                        let msg = buf[..total].to_vec();
                        buf.drain(..total);
                        Some((msg, false))
                    } else {
                        tracing::debug!("resync: skip misaligned extended byte");
                        buf.drain(..1);
                        None
                    }
                } else if first_byte == 0x82 {
                    if buf.len() < 2 {
                        return Ok(());
                    }
                    let b1 = buf[1];
                    if b1 & 0x80 != 0 {
                        tracing::debug!("resync: masked websocket frame, skipping byte");
                        buf.drain(..1);
                        None
                    } else {
                        let len7 = b1 as usize;
                        let (header, payload_len) = if len7 < 126 {
                            (2, len7)
                        } else if len7 == 126 {
                            if buf.len() < 4 {
                                return Ok(());
                            }
                            (4, u16::from_be_bytes([buf[2], buf[3]]) as usize)
                        } else {
                            if buf.len() < 10 {
                                return Ok(());
                            }
                            (
                                10,
                                u64::from_be_bytes([
                                    buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8], buf[9],
                                ]) as usize,
                            )
                        };
                        let total = header + payload_len;
                        if payload_len > MAX_REALTIME_FRAME_LEN {
                            tracing::debug!("resync: websocket frame len {payload_len} too large");
                            buf.drain(..1);
                            None
                        } else if buf.len() < total {
                            return Ok(());
                        } else {
                            let payload = buf[header..total].to_vec();
                            buf.drain(..total);
                            match mezon_proto::realtime::Envelope::decode(payload.as_slice()) {
                                Ok(envelope) => {
                                    tracing::info!(
                                        "ws-binary {} bytes -> Envelope cid={} {}",
                                        payload.len(),
                                        envelope.cid,
                                        envelope_detail(&envelope)
                                    );
                                    handlers.trigger_message(envelope.cid as u16, 0, payload)
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "ws-binary {} bytes — decode failed: {e}",
                                        payload.len()
                                    )
                                }
                            }
                            None
                        }
                    }
                } else {
                    tracing::warn!("Unexpected first byte: {:#x}, skipping", first_byte);
                    buf.drain(..1);
                    Some((vec![], true))
                }
            };

            let (data, skipped) = match msg_bytes {
                Some(v) => v,
                None => continue,
            };

            if skipped {
                continue;
            }

            tracing::info!("process_message: {} bytes", data.len());

            if data[0] == 0x00 {
                let cid = u16::from_be_bytes([data[1], data[2]]);
                tracing::info!("PONG: cid={}", cid);
                handlers.trigger_message(cid, 0, vec![]);
                continue;
            }

            if data[0] == PREFIX_RAW {
                let cid = u16::from_be_bytes([data[1], data[2]]);
                let code = u32::from_be_bytes([data[3], data[4], data[5], data[6]]);
                let response_code = (code >> 16) & 0xffff;
                let payload = data[RAW_HEADER_LENGTH..].to_vec();
                tracing::info!(
                    "Complete API response: cid={cid} code={response_code} len={} bytes",
                    payload.len()
                );
                handlers.trigger_message(cid, response_code, payload);
                continue;
            }

            let (header_size, payload_length) = if data[0] < 127 {
                tracing::info!(
                    "Standard msg: 1-byte header ({}*4={}bytes)",
                    data[0],
                    data[0] as usize * 4
                );
                (1, data[0] as usize * 4)
            } else {
                let len = u32::from_le_bytes([data[1], data[2], data[3], 0]) as usize * 4;
                tracing::info!("Extended msg: len={}", len);
                (4, len)
            };

            let framed = &data[header_size..header_size + payload_length];
            let unpadded_end = framed.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
            let payload = &framed[..unpadded_end];
            tracing::info!(
                "Std msg payload: {} bytes (framed {}) {:02x?}",
                payload.len(),
                framed.len(),
                &payload[..payload.len().min(48)]
            );

            if let Ok(envelope) = mezon_proto::realtime::Envelope::decode(payload) {
                tracing::info!(
                    "abridged -> Envelope cid={} {}",
                    envelope.cid,
                    envelope_detail(&envelope)
                );
                handlers.trigger_message(envelope.cid as u16, 0, payload.to_vec());
            } else {
                let cid = decode_cid_field(payload).unwrap_or(0);
                tracing::warn!("Failed to decode Envelope, passing raw cid={cid}");
                handlers.trigger_message(cid, 0, payload.to_vec());
            }
        }
    }

    async fn io_loop(
        mut tls: TlsStream,
        mut write_rx: mpsc::UnboundedReceiver<Vec<u8>>,
        ready_tx: oneshot::Sender<()>,
        state: IoLoopState,
    ) {
        let mut read_buf = vec![0u8; 8192];
        let mut read_count = 0u64;

        // Signal that io_loop is ready (select is polling)
        let _ = ready_tx.send(());
        tracing::info!("I/O loop running, entering select branch");

        loop {
            tracing::trace!("select iteration begin");
            tokio::select! {
                result = tls.read(&mut read_buf) => {
                    tracing::debug!("select: READ branch fired");
                    match result {
                        Ok(0) => {
                            tracing::info!("Server closed connection after {} reads", read_count);
                            *state.is_connected.lock().await = false;
                            let h = state.handlers.lock().await;
                            h.trigger_close(true);
                            break;
                        }
                        Ok(n) => {
                            read_count += 1;
                            tracing::info!(
                                "READ {} bytes [{}] (reads: {}) {:02x?}",
                                n,
                                frame_kind(read_buf[0]),
                                read_count,
                                &read_buf[..n.min(16)]
                            );

                            let data = read_buf[..n].to_vec();
                            if data.starts_with(b"HTTP/")
                                || data.starts_with(b"GET ")
                                || data.starts_with(b"POST ")
                            {
                                let preview = String::from_utf8_lossy(&data[..n.min(120)]);
                                tracing::error!(
                                    "Server returned HTTP on abridged TCP port (expected binary framing, not nginx/WSS). Response: {}",
                                    preview.lines().next().unwrap_or("?")
                                );
                                *state.is_connected.lock().await = false;
                                let h = state.handlers.lock().await;
                                h.trigger_error("server spoke HTTP instead of abridged TCP".into());
                                h.trigger_close(false);
                                break;
                            }

                            let adapter = AbridgedTcpAdapter {
                                write_tx: Arc::new(Mutex::new(None)),
                                handlers: state.handlers.clone(),
                                streams: state.streams.clone(),
                                is_connected: state.is_connected.clone(),
                                read_buffer: state.read_buffer.clone(),
                                pending_raw: state.pending_raw.clone(),
                            };
                            if let Err(e) = adapter.handle_data(data).await {
                                tracing::error!("handle_data error: {}", e);
                            }
                        }
                        Err(e) => {
                            tracing::error!("READ error: kind={:?} msg={}", e.kind(), e);
                            *state.is_connected.lock().await = false;
                            let h = state.handlers.lock().await;
                            h.trigger_error(e.to_string());
                            h.trigger_close(false);
                            break;
                        }
                    }
                }
                maybe_msg = write_rx.recv() => {
                    tracing::debug!("select: WRITE branch fired");
                    match maybe_msg {
                        Some(packet) => {
                            if packet.first() == Some(&0xef) {
                                tracing::info!("WRITE handshake frame");
                            } else {
                                tracing::info!(
                                    "WRITE {} bytes [{}] {:02x?}",
                                    packet.len(),
                                    frame_kind(packet[0]),
                                    &packet[..packet.len().min(32)]
                                );
                            }
                            match tls.write_all(&packet).await {
                                Ok(()) => tracing::info!("write_all OK"),
                                Err(e) => {
                                    tracing::error!("write_all error: {}", e);
                                    break;
                                }
                            }
                            match tls.flush().await {
                                Ok(()) => tracing::info!("flush OK"),
                                Err(e) => {
                                    tracing::error!("flush error: {}", e);
                                    break;
                                }
                            }
                        }
                        None => {
                            tracing::info!("Write channel closed, exiting I/O loop");
                            break;
                        }
                    }
                }
            }
        }

        tracing::info!("I/O loop exited (total reads: {})", read_count);
    }
}

fn decode_cid_field(payload: &[u8]) -> Option<u16> {
    if payload.first().copied()? != 0x08 {
        return None;
    }

    let mut value = 0u32;
    let mut shift = 0;
    for byte in payload.iter().copied().skip(1) {
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return u16::try_from(value).ok();
        }
        shift += 7;
        if shift >= 16 {
            return None;
        }
    }

    None
}

impl Default for AbridgedTcpAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TransportAdapter for AbridgedTcpAdapter {
    async fn connect(&mut self, host: &str, port: u16, token: &str) -> Result<()> {
        tracing::info!("=== CONNECT START: {}:{} ===", host, port);

        let config = build_client_config();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));

        let addr = format!("{}:{}", host, port);
        tracing::info!("TCP connecting to {}...", addr);
        let tcp = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            TcpStream::connect(&addr),
        )
        .await
        .map_err(|_| anyhow::anyhow!("TCP connect timed out after 15s: {addr}"))?
        .map_err(|e| anyhow::anyhow!("TCP connect failed: {e}"))?;
        let local = tcp
            .local_addr()
            .map_err(|e| anyhow::anyhow!("local_addr: {e}"))?;
        let peer = tcp
            .peer_addr()
            .map_err(|e| anyhow::anyhow!("peer_addr: {e}"))?;
        tracing::info!("TCP connected: local={} peer={}", local, peer);

        let domain = ServerName::try_from(host.to_string())
            .map_err(|e| anyhow::anyhow!("Invalid DNS name: {e}"))?;
        tracing::info!("DNS name parsed: {:?}", domain);

        tracing::info!("Starting TLS handshake with {}...", host);
        let tls = connector
            .connect(domain, tcp)
            .await
            .map_err(|e| anyhow::anyhow!("TLS handshake failed: {e}"))?;
        tracing::info!("TLS handshake complete");

        // Spawn I/O loop and wait for it to be ready
        let (ready_tx, ready_rx) = oneshot::channel();
        let (write_tx, write_rx) = mpsc::unbounded_channel();
        let state = IoLoopState {
            handlers: self.handlers.clone(),
            streams: self.streams.clone(),
            is_connected: self.is_connected.clone(),
            read_buffer: self.read_buffer.clone(),
            pending_raw: self.pending_raw.clone(),
        };

        tracing::info!("Spawning I/O loop...");
        tokio::spawn(async move {
            Self::io_loop(tls, write_rx, ready_tx, state).await;
        });

        // Wait for I/O loop to signal readiness
        tracing::info!("Waiting for I/O loop to be ready...");
        ready_rx
            .await
            .map_err(|_| anyhow::anyhow!("I/O loop panicked before starting"))?;
        tracing::info!("I/O loop confirmed READY");

        // Now send handshake — I/O loop is definitely listening
        let token_bytes = token.as_bytes();
        let padding = (4 - (token_bytes.len() % 4)) % 4;
        let mut final_token = token_bytes.to_vec();
        final_token.extend(vec![0u8; padding]);
        let len_header = (final_token.len() / 4) as u8;
        let mut handshake = vec![0xef, len_header];
        handshake.extend(&final_token);

        tracing::info!("Sending handshake");
        write_tx
            .send(handshake)
            .map_err(|_| anyhow::anyhow!("Write channel closed early"))?;
        tracing::info!("Handshake queued via mpsc channel");

        *self.write_tx.lock().await = Some(write_tx);
        *self.is_connected.lock().await = true;
        tracing::info!("Connection state: is_connected=true, write_tx set");

        {
            let h = self.handlers.lock().await;
            h.trigger_open();
        }
        tracing::info!("on_open triggered");

        tracing::info!("=== CONNECT COMPLETE ===");
        Ok(())
    }

    async fn send(&mut self, message: Vec<u8>) -> Result<()> {
        match mezon_proto::realtime::Envelope::decode(message.as_slice()) {
            Ok(envelope) => tracing::info!(
                "send() {} bytes -> Envelope cid={} {}",
                message.len(),
                envelope.cid,
                envelope_detail(&envelope)
            ),
            Err(_) => tracing::info!(
                "send() {} bytes (non-envelope) {:02x?}",
                message.len(),
                &message[..message.len().min(16)]
            ),
        }

        if !self.is_open() {
            tracing::warn!("send(): connection NOT open, rejecting");
            return Err(anyhow::anyhow!("Connection is not open"));
        }
        tracing::info!("Connection is open");

        let padding_needed = (4 - (message.len() % 4)) % 4;
        let mut final_payload = message;
        final_payload.extend(vec![0u8; padding_needed]);
        tracing::info!(
            "Padded to {} bytes (+{} padding)",
            final_payload.len(),
            padding_needed
        );

        let len_div4 = final_payload.len() / 4;
        let header = if len_div4 < 127 {
            tracing::info!("Abridged header: 1-byte ({})", len_div4);
            vec![len_div4 as u8]
        } else {
            let mut h = vec![PREFIX_EXTENDED, 0, 0, 0];
            h[1..4].copy_from_slice(&(len_div4 as u32).to_le_bytes()[..3]);
            tracing::info!("Abridged header: 4-byte extended ({})", len_div4);
            h
        };

        let mut packet = header;
        packet.extend(&final_payload);
        tracing::info!(
            "Full abridged packet: {} bytes {:02x?}",
            packet.len(),
            &packet[..packet.len().min(64)]
        );

        let guard = self.write_tx.lock().await;
        match *guard {
            Some(ref tx) => {
                tx.send(packet).map_err(|_| {
                    tracing::error!("mpsc send failed: channel closed");
                    anyhow::anyhow!("Write channel closed")
                })?;
                tracing::info!("Packet queued via mpsc channel");
            }
            None => {
                tracing::error!("Write channel not available (None)");
                return Err(anyhow::anyhow!("Write channel not available"));
            }
        }

        Ok(())
    }

    async fn send_ping(&mut self, cid: u16) -> Result<()> {
        if !self.is_open() {
            return Err(anyhow::anyhow!("Connection is not open"));
        }
        let mut buffer = vec![0x00];
        buffer.extend(&cid.to_be_bytes());
        let guard = self.write_tx.lock().await;
        if let Some(ref tx) = *guard {
            tx.send(buffer)
                .map_err(|_| anyhow::anyhow!("Write channel closed"))?;
        }
        Ok(())
    }

    fn is_open(&self) -> bool {
        self.is_connected.try_lock().map(|g| *g).unwrap_or(false)
    }
    async fn close(&mut self) -> Result<()> {
        *self.is_connected.lock().await = false;
        *self.write_tx.lock().await = None;
        Ok(())
    }

    fn set_on_message(&mut self, handler: crate::transport_adapter::MessageHandler) {
        if let Ok(mut h) = self.handlers.try_lock() {
            h.on_message = Some(handler);
        }
    }
    fn set_on_open(&mut self, handler: crate::transport_adapter::OpenHandler) {
        if let Ok(mut h) = self.handlers.try_lock() {
            h.on_open = Some(handler);
        }
    }
    fn set_on_close(&mut self, handler: crate::transport_adapter::CloseHandler) {
        if let Ok(mut h) = self.handlers.try_lock() {
            h.on_close = Some(handler);
        }
    }
    fn set_on_error(&mut self, handler: crate::transport_adapter::ErrorHandler) {
        if let Ok(mut h) = self.handlers.try_lock() {
            h.on_error = Some(handler);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delimits_ack_body_before_next_frame() {
        let stream = [
            0x10, 0x80, 0xa0, 0x80, 0xf0, 0xfe, 0xd1, 0xd3, 0xc5, 0x19, 0x0c, 0xc2, 0x01, 0x2a,
        ];
        assert_eq!(protobuf_message_len(&stream), Some(10));
    }

    #[test]
    fn consumes_whole_message_when_no_trailing_frame() {
        let msg = [0x10, 0x80, 0xa0, 0x80, 0xf0, 0xfe, 0xd1, 0xd3, 0xc5, 0x19];
        assert_eq!(protobuf_message_len(&msg), Some(10));
    }

    #[test]
    fn handles_length_delimited_field() {
        let stream = [0x0a, 0x03, b'a', b'b', b'c', 0x17, 0x32, 0x57];
        assert_eq!(protobuf_message_len(&stream), Some(5));
    }

    #[test]
    fn returns_none_on_incomplete_varint() {
        let stream = [0x10, 0x80, 0xa0];
        assert_eq!(protobuf_message_len(&stream), None);
    }

    #[test]
    fn returns_none_on_incomplete_length_delimited() {
        let stream = [0x0a, 0x05, b'a', b'b'];
        assert_eq!(protobuf_message_len(&stream), None);
    }

    #[test]
    fn accepts_real_envelope_frame() {
        let envelope = mezon_proto::realtime::Envelope {
            cid: 5,
            ..Default::default()
        };
        let mut bytes = Vec::new();
        envelope.encode(&mut bytes).unwrap();
        assert!(frame_is_valid_envelope(&bytes));
    }

    #[test]
    fn rejects_misaligned_frame() {
        let blob = [
            0x18, 0x80, 0xa0, 0x80, 0xca, 0xa5, 0xcb, 0x89, 0xad, 0x19, 0x20, 0x02,
        ];
        assert!(!frame_is_valid_envelope(&blob));
    }

    #[test]
    fn prefix_plausible_for_envelope_fields() {
        assert!(envelope_prefix_plausible(&[0x32, 0x56, 0x08]));
        assert!(envelope_prefix_plausible(&[0x08, 0x05]));
        assert!(envelope_prefix_plausible(&[0xc2, 0x01, 0x2a]));
    }

    #[test]
    fn prefix_implausible_for_misaligned_data() {
        assert!(!envelope_prefix_plausible(&[0x18, 0x80, 0xa0]));
        assert!(!envelope_prefix_plausible(&[0x1c, 0x10, 0x80]));
    }

    #[test]
    fn prefix_plausible_when_empty() {
        assert!(envelope_prefix_plausible(&[]));
    }

    #[test]
    fn caps_absurd_field_length() {
        let stream = [0x0a, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x00, 0x00];
        assert_eq!(protobuf_message_len(&stream), Some(0));
    }

    fn raw_frame(cid: u16, fin: bool, payload: &[u8]) -> Vec<u8> {
        let code: u32 = if fin { u32::from(CODE_FIN) } else { 0 };
        let mut frame = vec![PREFIX_RAW];
        frame.extend_from_slice(&cid.to_be_bytes());
        frame.extend_from_slice(&code.to_be_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    fn channel_messages_body(content_len: usize) -> Vec<u8> {
        mezon_proto::api::ChannelMessageList {
            messages: vec![mezon_proto::api::ChannelMessage {
                content: "x".repeat(content_len),
                ..Default::default()
            }],
            ..Default::default()
        }
        .encode_to_vec()
    }

    async fn captured_responses(
        adapter: &AbridgedTcpAdapter,
    ) -> Arc<std::sync::Mutex<Vec<(u16, u32, Vec<u8>)>>> {
        let received = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = received.clone();
        adapter.handlers.lock().await.on_message = Some(Arc::new(move |cid, code, bytes| {
            sink.lock().unwrap().push((cid, code, bytes));
        }));
        received
    }

    #[tokio::test]
    async fn reassembles_chunked_api_response_split_across_frames() {
        let body = channel_messages_body(6000);
        assert!(body.len() > RAW_CHUNK_PAYLOAD_LEN);

        let adapter = AbridgedTcpAdapter::new();
        let received = captured_responses(&adapter).await;

        // Mirror the real capture: each 7-byte header and its payload arrive in
        // separate TCP reads (chunk header, chunk payload, then the final frame).
        adapter.handle_data(raw_frame(6, false, &[])).await.unwrap();
        adapter
            .handle_data(body[..RAW_CHUNK_PAYLOAD_LEN].to_vec())
            .await
            .unwrap();
        adapter
            .handle_data(raw_frame(6, true, &body[RAW_CHUNK_PAYLOAD_LEN..]))
            .await
            .unwrap();

        let received = received.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].0, 6);
        assert_eq!(received[0].1, 0);
        assert_eq!(received[0].2, body);
        mezon_proto::api::ChannelMessageList::decode(received[0].2.as_slice()).unwrap();
    }

    #[tokio::test]
    async fn delivers_single_frame_api_response() {
        let body = channel_messages_body(16);
        assert!(body.len() < RAW_CHUNK_PAYLOAD_LEN);

        let adapter = AbridgedTcpAdapter::new();
        let received = captured_responses(&adapter).await;

        adapter
            .handle_data(raw_frame(7, true, &body))
            .await
            .unwrap();

        let received = received.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].0, 7);
        assert_eq!(received[0].2, body);
    }
}
