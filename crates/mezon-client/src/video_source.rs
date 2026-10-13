use std::time::Duration;

use crate::app_api::CDN_MEZON_MINIO;

const HLS_MASTER_PLAYLIST: &str = "master.m3u8";
const HLS_PLAYLIST_TAG: &[u8] = b"#EXTM3U";
const HLS_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn transcoded_hls_playlist(url: &str) -> Option<String> {
    hls_playlist_on(url, CDN_MEZON_MINIO).await
}

async fn hls_playlist_on(url: &str, transcoding_origin: &str) -> Option<String> {
    let playlist = hls_playlist_candidate(url, transcoding_origin)?;
    let head = crate::transport_runtime::fetch_prefix(
        &playlist,
        HLS_PLAYLIST_TAG.len(),
        HLS_PROBE_TIMEOUT,
    )
    .await?;
    (head == HLS_PLAYLIST_TAG).then_some(playlist)
}

fn hls_playlist_candidate(url: &str, transcoding_origin: &str) -> Option<String> {
    let path = url.strip_prefix(transcoding_origin)?.strip_prefix('/')?;
    if path.contains(['?', '#']) {
        return None;
    }
    let file = path.rsplit('/').next()?;
    if file.is_empty() || file.to_ascii_lowercase().ends_with(".m3u8") {
        return None;
    }
    Some(format!("{url}/{HLS_MASTER_PLAYLIST}"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::{CDN_MEZON_MINIO, hls_playlist_candidate, hls_playlist_on};

    const VIDEO: &str = "1cf93b197d401000/2080000000000000001_clip.mp4";

    async fn fake_minio(objects: Vec<(String, &'static str)>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let objects: Arc<HashMap<String, &'static str>> = Arc::new(objects.into_iter().collect());
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let objects = objects.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let Ok(n) = stream.read(&mut buf).await else {
                        return;
                    };
                    let head = String::from_utf8_lossy(&buf[..n]).to_string();
                    let target = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    let (status, body) = match objects.get(&target) {
                        Some(body) => ("200 OK", *body),
                        None => ("404 Not Found", "<Error><Code>NoSuchKey</Code></Error>"),
                    };
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        origin
    }

    #[test]
    fn a_video_on_the_transcoding_cdn_points_at_its_master_playlist() {
        let url = format!("{CDN_MEZON_MINIO}/{VIDEO}");
        assert_eq!(
            hls_playlist_candidate(&url, CDN_MEZON_MINIO),
            Some(format!("{url}/master.m3u8"))
        );
    }

    #[test]
    fn videos_the_transcoder_never_sees_are_not_probed() {
        for url in [
            format!("https://cdn.komu.vn/{VIDEO}"),
            format!("{CDN_MEZON_MINIO}.evil.example/{VIDEO}"),
            format!("{CDN_MEZON_MINIO}/{VIDEO}?1700000000-mac"),
            format!("{CDN_MEZON_MINIO}/{VIDEO}#t=3"),
            format!("{CDN_MEZON_MINIO}/{VIDEO}/master.m3u8"),
            format!("{CDN_MEZON_MINIO}/1cf93b197d401000/"),
            CDN_MEZON_MINIO.to_string(),
        ] {
            assert_eq!(hls_playlist_candidate(&url, CDN_MEZON_MINIO), None, "{url}");
        }
    }

    #[tokio::test]
    async fn a_transcoded_video_plays_from_its_playlist() {
        let master = "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-STREAM-INF:BANDWIDTH=3137344,RESOLUTION=1280x720\nstream_0/playlist.m3u8\n";
        let origin = fake_minio(vec![(format!("/{VIDEO}/master.m3u8"), master)]).await;
        let url = format!("{origin}/{VIDEO}");
        assert_eq!(
            hls_playlist_on(&url, &origin).await,
            Some(format!("{url}/master.m3u8"))
        );
    }

    #[tokio::test]
    async fn a_video_not_transcoded_yet_keeps_its_original_url() {
        let origin = fake_minio(vec![(format!("/{VIDEO}"), "mp4 bytes")]).await;
        assert_eq!(
            hls_playlist_on(&format!("{origin}/{VIDEO}"), &origin).await,
            None
        );
    }

    #[tokio::test]
    async fn a_playlist_url_that_answers_something_else_is_ignored() {
        let origin = fake_minio(vec![(format!("/{VIDEO}/master.m3u8"), "<html>ok</html>")]).await;
        assert_eq!(
            hls_playlist_on(&format!("{origin}/{VIDEO}"), &origin).await,
            None
        );
    }
}
