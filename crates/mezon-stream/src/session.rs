use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use flume::{Receiver, Sender};
use futures::{SinkExt, StreamExt};
use libwebrtc::audio_stream::native::NativeAudioStream;
use libwebrtc::media_stream_track::MediaStreamTrack;
use libwebrtc::peer_connection::{AnswerOptions, PeerConnection, PeerConnectionState};
use libwebrtc::peer_connection_factory::{
    ContinualGatheringPolicy, IceServer, IceTransportsType, PeerConnectionFactory, RtcConfiguration,
};
use libwebrtc::rtp_transceiver::RtpTransceiverDirection;
use libwebrtc::session_description::{SdpType, SessionDescription};
use mezon_voice::StreamAudioOutput;
use parking_lot::Mutex;
use serde_json::Value;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

pub type StreamTokenProvider =
    Arc<dyn Fn() -> futures::future::BoxFuture<'static, Result<String>> + Send + Sync + 'static>;

#[derive(Clone)]
pub struct StreamSessionConfig {
    pub ws_url: String,
    pub token: String,
    pub room: String,
    pub token_provider: StreamTokenProvider,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Live,
    NoBroadcast,
    RemoteAudio(bool),
    PlaybackBlocked,
    Error(String),
    Disconnected,
}

pub struct StreamSession {
    stop_tx: Sender<()>,
    event_rx: Receiver<StreamEvent>,
    audio: Arc<Mutex<Option<Arc<StreamAudioOutput>>>>,
}

impl StreamSession {
    pub fn start(
        config: StreamSessionConfig,
        output_device_id: Option<String>,
        volume: f32,
        muted: bool,
    ) -> Self {
        let (stop_tx, stop_rx) = flume::bounded(1);
        let (event_tx, event_rx) = flume::unbounded();
        let audio = Arc::new(Mutex::new(None));
        let audio_for_thread = audio.clone();
        std::thread::spawn(move || {
            let audio_output = match StreamAudioOutput::start(output_device_id, volume, muted) {
                Ok(output) => Arc::new(output),
                Err(_) => {
                    let _ = event_tx.send(StreamEvent::Error("Stream audio failed".into()));
                    let _ = event_tx.send(StreamEvent::Disconnected);
                    return;
                }
            };
            *audio_for_thread.lock() = Some(audio_output.clone());

            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build();
            let Ok(runtime) = runtime else {
                let _ = event_tx.send(StreamEvent::Error("stream runtime failed".into()));
                let _ = event_tx.send(StreamEvent::Disconnected);
                return;
            };
            runtime.block_on(run_session(config, audio_output, stop_rx, event_tx));
        });
        Self {
            stop_tx,
            event_rx,
            audio,
        }
    }

    pub fn audio(&self) -> Option<Arc<StreamAudioOutput>> {
        self.audio.lock().clone()
    }

    pub fn events(&self) -> &Receiver<StreamEvent> {
        &self.event_rx
    }

    pub fn disconnect(&self) {
        let _ = self.stop_tx.try_send(());
    }
}

impl Drop for StreamSession {
    fn drop(&mut self) {
        self.disconnect();
    }
}

async fn run_session(
    config: StreamSessionConfig,
    audio: Arc<StreamAudioOutput>,
    stop_rx: Receiver<()>,
    event_tx: Sender<StreamEvent>,
) {
    let mut token = config.token.clone();
    let mut reconnect_attempt = 0u32;

    loop {
        match run_session_once(
            &config,
            &token,
            audio.clone(),
            &stop_rx,
            &event_tx,
            &mut reconnect_attempt,
        )
        .await
        {
            Ok(()) => break,
            Err(_) => {
                reconnect_attempt = reconnect_attempt.saturating_add(1);
                tracing::warn!(
                    attempt = reconnect_attempt,
                    "SFU stream session reconnecting"
                );
                let Some(next_token) =
                    wait_for_refreshed_token(&config, &stop_rx, &mut reconnect_attempt).await
                else {
                    break;
                };
                token = next_token;
            }
        }
    }
    let _ = event_tx.send(StreamEvent::Disconnected);
}

async fn wait_for_refreshed_token(
    config: &StreamSessionConfig,
    stop_rx: &Receiver<()>,
    reconnect_attempt: &mut u32,
) -> Option<String> {
    loop {
        let backoff = reconnect_delay(*reconnect_attempt);
        tokio::select! {
            _ = stop_rx.recv_async() => return None,
            _ = tokio::time::sleep(backoff) => {},
        }

        let refresh = (config.token_provider)();
        let result = tokio::select! {
            _ = stop_rx.recv_async() => return None,
            result = refresh => result,
        };
        match result {
            Ok(token) if !token.is_empty() => return Some(token),
            Ok(_) | Err(_) => {
                *reconnect_attempt = reconnect_attempt.saturating_add(1);
                tracing::warn!(
                    attempt = *reconnect_attempt,
                    "SFU stream token refresh failed; retrying"
                );
            }
        }
    }
}

fn reconnect_delay(attempt: u32) -> Duration {
    Duration::from_millis(1_000u64.saturating_mul(2u64.saturating_pow(attempt.min(4))))
        .min(Duration::from_secs(15))
}

async fn run_session_once(
    config: &StreamSessionConfig,
    token: &str,
    audio: Arc<StreamAudioOutput>,
    stop_rx: &Receiver<()>,
    event_tx: &Sender<StreamEvent>,
    reconnect_attempt: &mut u32,
) -> Result<()> {
    let url = build_ws_url(&config.ws_url, token)?;
    let (ws_stream, _) = tokio::select! {
        _ = stop_rx.recv_async() => return Ok(()),
        result = connect_async(url.as_str()) => result.context("SFU websocket connect failed")?,
    };
    let (mut ws_tx, mut ws_rx) = ws_stream.split();
    send_json(
        &mut ws_tx,
        serde_json::json!({
            "type": "join",
            "room": config.room,
            "token": token,
            "role": "audience"
        }),
    )
    .await?;

    let (connection_state_tx, connection_state_rx) = flume::unbounded::<PeerConnectionState>();
    let mut _factory: Option<PeerConnectionFactory> = None;
    let mut pc = PeerConnectionGuard::default();
    let mut live = false;

    loop {
        tokio::select! {
            _ = stop_rx.recv_async() => break,
            state = connection_state_rx.recv_async() => {
                match state {
                    Ok(PeerConnectionState::Connected) => {
                        *reconnect_attempt = 0;
                        if !live {
                            live = true;
                            let _ = event_tx.send(StreamEvent::Live);
                        }
                    }
                    Ok(PeerConnectionState::Failed) => {
                        return Err(anyhow!("SFU peer connection failed"));
                    }
                    Ok(PeerConnectionState::Closed) => {
                        return Err(anyhow!("SFU peer connection closed"));
                    }
                    Ok(PeerConnectionState::New | PeerConnectionState::Connecting | PeerConnectionState::Disconnected) => {}
                    Err(_) => {}
                }
            }
            incoming = ws_rx.next() => {
                let Some(frame) = incoming else {
                    return Err(anyhow!("SFU websocket closed"));
                };
                let frame = frame.context("SFU websocket read failed")?;
                let text = match frame {
                    Message::Text(text) => text,
                    Message::Ping(payload) => {
                        ws_tx.send(Message::Pong(payload)).await?;
                        continue;
                    }
                    Message::Close(_) => return Err(anyhow!("SFU closed stream session")),
                    _ => continue,
                };
                let message: Value = serde_json::from_str(&text).context("invalid SFU JSON")?;
                match message.get("type").and_then(Value::as_str).unwrap_or_default() {
                    "ping" => send_json(&mut ws_tx, serde_json::json!({"type":"pong"})).await?,
                    "pong" => {}
                    "joined" => {
                        if pc.0.is_none() {
                            let created_factory = PeerConnectionFactory::default();
                            pc.0 = Some(create_peer_connection(
                                &created_factory,
                                &message,
                                &connection_state_tx,
                                audio.clone(),
                                event_tx,
                            )?);
                            _factory = Some(created_factory);
                        }
                    }
                    "offer" => {
                        let generation = message.get("offer_generation").and_then(Value::as_u64)
                            .ok_or_else(|| anyhow!("SFU offer is missing offer_generation"))?;
                        let sdp = message.get("sdp").and_then(Value::as_str)
                            .ok_or_else(|| anyhow!("SFU offer is missing sdp"))?;
                        let peer_connection = pc
                            .0
                            .as_ref()
                            .ok_or_else(|| anyhow!("SFU offer arrived before joined"))?;
                        negotiate(peer_connection, generation, sdp, &mut ws_tx).await?;
                    }
                    "error" => {
                        let reason = message.get("message").and_then(Value::as_str).unwrap_or("SFU error");
                        return Err(anyhow!("SFU error: {reason}"));
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

#[derive(Default)]
struct PeerConnectionGuard(Option<PeerConnection>);

impl Drop for PeerConnectionGuard {
    fn drop(&mut self) {
        if let Some(peer_connection) = self.0.take() {
            peer_connection.close();
        }
    }
}

fn create_peer_connection(
    factory: &PeerConnectionFactory,
    joined: &Value,
    connection_state_tx: &Sender<PeerConnectionState>,
    audio: Arc<StreamAudioOutput>,
    event_tx: &Sender<StreamEvent>,
) -> Result<libwebrtc::peer_connection::PeerConnection> {
    let ice_servers = joined
        .get("iceServers")
        .map(parse_ice_servers)
        .unwrap_or_default();
    let pc = factory
        .create_peer_connection(RtcConfiguration {
            ice_servers: if ice_servers.is_empty() {
                vec![IceServer {
                    urls: vec!["stun:stun.l.google.com:19302".into()],
                    username: String::new(),
                    password: String::new(),
                }]
            } else {
                ice_servers
            },
            continual_gathering_policy: ContinualGatheringPolicy::GatherContinually,
            ice_transport_type: IceTransportsType::All,
        })
        .context("create SFU peer connection")?;

    let connection_state_tx = connection_state_tx.clone();
    pc.on_connection_state_change(Some(Box::new(move |state| {
        let _ = connection_state_tx.send(state);
    })));

    let events = event_tx.clone();
    pc.on_track(Some(Box::new(move |track_event| match track_event.track {
        MediaStreamTrack::Audio(audio_track) => {
            let _ = events.send(StreamEvent::RemoteAudio(true));
            let player = audio.clone();
            let events_for_thread = events.clone();
            std::thread::spawn(move || {
                if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    runtime.block_on(async move {
                        let format = player.format();
                        let mut stream = NativeAudioStream::new(
                            audio_track,
                            format.sample_rate as i32,
                            format.channels as i32,
                        );
                        while let Some(frame) = stream.next().await {
                            player.push(&frame.data);
                        }
                        player.clear();
                    });
                }
                let _ = events_for_thread.send(StreamEvent::RemoteAudio(false));
            });
        }
        MediaStreamTrack::Video(_) => {}
    })));

    Ok(pc)
}

async fn negotiate(
    pc: &libwebrtc::peer_connection::PeerConnection,
    generation: u64,
    offer_sdp: &str,
    ws_tx: &mut futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
) -> Result<()> {
    let offer = SessionDescription::parse(offer_sdp, SdpType::Offer)
        .map_err(|e| anyhow!("parse SFU offer: {} {}", e.line, e.description))?;
    pc.set_remote_description(offer)
        .await
        .context("set SFU remote offer")?;

    let offered_sections = media_sections(offer_sdp);
    let transceivers = pc.transceivers();
    if transceivers.len() != offered_sections.len() {
        return Err(anyhow!("SFU transceiver count does not match offer"));
    }
    for (index, transceiver) in transceivers.iter().enumerate() {
        let offered = &offered_sections[index];
        if transceiver.mid().as_deref() != Some(offered.mid.as_str()) {
            return Err(anyhow!("SFU transceiver mid order changed"));
        }
        let direction = if offered.kind == "video" {
            RtpTransceiverDirection::Inactive
        } else {
            RtpTransceiverDirection::RecvOnly
        };
        transceiver
            .set_direction(direction)
            .map_err(|error| anyhow!("set SFU transceiver direction: {error}"))?;
    }

    let answer = pc
        .create_answer(AnswerOptions::default())
        .await
        .context("create SFU answer")?;
    pc.set_local_description(answer)
        .await
        .context("set SFU local answer")?;
    let local_sdp = pc
        .current_local_description()
        .map(|description| description.to_string())
        .context("SFU local answer missing")?;
    validate_full_sdp_layout(offer_sdp, &local_sdp)?;
    send_json(
        ws_tx,
        serde_json::json!({
            "type": "answer",
            "sdp": local_sdp,
            "offer_generation": generation
        }),
    )
    .await
}

#[derive(Debug, PartialEq, Eq)]
struct MediaSection {
    kind: String,
    mid: String,
    direction: Option<String>,
}

fn media_sections(sdp: &str) -> Vec<MediaSection> {
    let mut sections = Vec::new();
    let mut current: Option<MediaSection> = None;
    for line in sdp.lines().map(|line| line.trim_end_matches('\r')) {
        if let Some(media) = line.strip_prefix("m=") {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            let kind = media
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            current = Some(MediaSection {
                kind,
                mid: String::new(),
                direction: None,
            });
        } else if let Some(mid) = line.strip_prefix("a=mid:") {
            if let Some(section) = current.as_mut() {
                section.mid = mid.to_owned();
            }
        } else if matches!(
            line,
            "a=sendrecv" | "a=sendonly" | "a=recvonly" | "a=inactive"
        ) {
            if let Some(section) = current.as_mut() {
                section.direction = Some(line.to_owned());
            }
        }
    }
    if let Some(section) = current {
        sections.push(section);
    }
    sections
}

fn validate_full_sdp_layout(offer_sdp: &str, answer_sdp: &str) -> Result<()> {
    let offer = media_sections(offer_sdp);
    let answer = media_sections(answer_sdp);
    if offer.len() != answer.len() {
        return Err(anyhow!("SFU answer changed m-line count"));
    }
    for (index, offered) in offer.iter().enumerate() {
        let actual = &answer[index];
        if offered.mid.is_empty()
            || actual.mid.is_empty()
            || offered.kind != actual.kind
            || offered.mid != actual.mid
        {
            return Err(anyhow!("SFU answer changed m-line order or mid"));
        }
        match offered.kind.as_str() {
            "video" if actual.direction.as_deref() != Some("a=inactive") => {
                return Err(anyhow!("SFU answer activated a video m-line"));
            }
            "audio"
                if !matches!(
                    actual.direction.as_deref(),
                    Some("a=recvonly") | Some("a=inactive")
                ) =>
            {
                return Err(anyhow!("SFU answer activated an audio sender"));
            }
            _ => {}
        }
    }
    Ok(())
}
fn parse_ice_servers(value: &Value) -> Vec<IceServer> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let urls = match item.get("urls") {
                Some(Value::String(url)) => vec![url.clone()],
                Some(Value::Array(urls)) => urls
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
                _ => Vec::new(),
            };
            (!urls.is_empty()).then(|| IceServer {
                urls,
                username: item
                    .get("username")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                password: item
                    .get("credential")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            })
        })
        .collect()
}

async fn send_json<S>(sink: &mut S, value: Value) -> Result<()>
where
    S: SinkExt<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    sink.send(Message::Text(value.to_string().into()))
        .await
        .map_err(|e| anyhow!("SFU websocket send failed: {e}"))
}

fn build_ws_url(base: &str, token: &str) -> Result<url::Url> {
    let mut url = url::Url::parse(base.trim())?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("access_token", token);
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFFER: &str = concat!(
        "v=0\r\n",
        "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
        "a=mid:0\r\n",
        "a=sendrecv\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 96\r\n",
        "a=mid:1\r\n",
        "a=sendrecv\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 96\r\n",
        "a=mid:2\r\n",
        "a=sendrecv\r\n",
    );

    const ANSWER: &str = concat!(
        "v=0\r\n",
        "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
        "a=mid:0\r\n",
        "a=recvonly\r\n",
        "m=video 0 UDP/TLS/RTP/SAVPF 96\r\n",
        "a=mid:1\r\n",
        "a=inactive\r\n",
        "m=video 0 UDP/TLS/RTP/SAVPF 96\r\n",
        "a=mid:2\r\n",
        "a=inactive\r\n",
    );

    #[test]
    fn accepts_full_sdp_with_audio_only_answer_directions() {
        assert!(validate_full_sdp_layout(OFFER, ANSWER).is_ok());
        let sections = media_sections(ANSWER);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].direction.as_deref(), Some("a=recvonly"));
        assert_eq!(sections[1].direction.as_deref(), Some("a=inactive"));
        assert_eq!(sections[2].direction.as_deref(), Some("a=inactive"));
    }

    #[test]
    fn rejects_changed_m_line_layout_or_active_video() {
        let wrong_order = ANSWER.replace("a=mid:1", "a=mid:9");
        assert!(validate_full_sdp_layout(OFFER, &wrong_order).is_err());

        let missing_mid = ANSWER.replace("a=mid:1", "a=mid:");
        assert!(validate_full_sdp_layout(OFFER, &missing_mid).is_err());

        let active_video = ANSWER.replace("a=inactive", "a=recvonly");
        assert!(validate_full_sdp_layout(OFFER, &active_video).is_err());

        let active_audio = ANSWER.replace("a=recvonly", "a=sendrecv");
        assert!(validate_full_sdp_layout(OFFER, &active_audio).is_err());
    }

    #[test]
    fn appends_access_token_without_replacing_existing_query() {
        let url = build_ws_url("wss://sfu.example/ws?transport=websocket", "opaque-token").unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(
            query.get("transport").map(String::as_str),
            Some("websocket")
        );
        assert_eq!(
            query.get("access_token").map(String::as_str),
            Some("opaque-token")
        );
    }
}
