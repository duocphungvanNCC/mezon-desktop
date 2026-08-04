use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use futures::AsyncReadExt as _;
use http_client::{AsyncBody, HttpClient, http};
use prost::Message as _;
use url::Url;

use crate::transport_runtime::{http_client, runtime};

const OGP_TIMEOUT: Duration = Duration::from_secs(10);
const OGP_MAX_HTML_BYTES: usize = 512 * 1024;
const OGP_MAX_DECOMPRESSED: usize = 4 * 1024 * 1024;
const OGP_USER_AGENT: &str =
    "Mozilla/5.0 (compatible; MezonDesktop/1.0; +https://mezon.ai) OpenGraphPreview";
const INVITE_PREVIEW_MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OgpResult {
    pub url: String,
    pub title: String,
    pub description: String,
    pub image: String,
    pub is_image: bool,
    pub banner: String,
    pub member_count: Option<i64>,
    pub is_community: bool,
    pub clan_id: Option<String>,
}

impl OgpResult {
    pub fn to_outgoing(&self) -> crate::transport::OutgoingOgp {
        crate::transport::OutgoingOgp {
            url: self.url.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            image: self.image.clone(),
            banner: self.banner.clone(),
            member_count: self.member_count,
            is_community: self.is_community,
            clan_id: self.clan_id.clone(),
        }
    }
}

pub async fn fetch_invite_preview(
    gw_host: &str,
    gw_port: u16,
    secure: bool,
    server_key: &str,
    url: &str,
    invite_id: &str,
) -> Result<OgpResult> {
    let scheme = if secure { "https" } else { "http" };
    let request_url = format!("{scheme}://{gw_host}:{gw_port}/v2/invite/{invite_id}");
    let auth_header = format!("Basic {}", B64.encode(format!("{server_key}:")));
    let page_url = url.to_string();
    runtime()
        .spawn(async move {
            let request = http::Request::builder()
                .method(http::Method::GET)
                .uri(&request_url)
                .header(http::header::AUTHORIZATION, auth_header)
                .header(http::header::ACCEPT, "application/x-protobuf")
                .body(AsyncBody::empty())?;
            let outcome = tokio::time::timeout(OGP_TIMEOUT, async move {
                let mut response = http_client().send(request).await?;
                if !response.status().is_success() {
                    anyhow::bail!("HTTP GET failed with status {}", response.status());
                }
                let content_encoding = response
                    .headers()
                    .get(http::header::CONTENT_ENCODING)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                let mut bytes = Vec::new();
                let mut limited = response.body_mut().take(INVITE_PREVIEW_MAX_BYTES as u64);
                limited.read_to_end(&mut bytes).await?;
                let decoded = decompress_body(&bytes, &content_encoding);
                let payload = decoded.as_deref().unwrap_or(&bytes);
                let res = mezon_proto::api::InviteUserRes::decode(payload)?;
                Ok(OgpResult {
                    url: page_url,
                    title: res.clan_name,
                    image: res.clan_logo,
                    banner: res.banner,
                    member_count: Some(res.member_count as i64),
                    is_community: res.is_community,
                    clan_id: (res.clan_id != 0).then(|| res.clan_id.to_string()),
                    ..Default::default()
                })
            })
            .await;
            match outcome {
                Ok(inner) => inner,
                Err(_) => {
                    anyhow::bail!(
                        "invite preview fetch timed out after {}s",
                        OGP_TIMEOUT.as_secs()
                    )
                }
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("invite preview fetch task failed: {e}"))?
}

pub async fn fetch_ogp(url: &str) -> Result<OgpResult> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        anyhow::bail!("unsupported url scheme");
    }
    let page_url = url.to_string();
    runtime()
        .spawn(async move {
            let request = http::Request::builder()
                .method(http::Method::GET)
                .uri(&page_url)
                .header(http::header::USER_AGENT, OGP_USER_AGENT)
                .header(
                    http::header::ACCEPT,
                    "text/html,application/xhtml+xml,image/*;q=0.8",
                )
                .header(http::header::ACCEPT_ENCODING, "gzip, deflate")
                .body(AsyncBody::empty())?;
            let outcome = tokio::time::timeout(OGP_TIMEOUT, async move {
                let mut response = http_client().send(request).await?;
                if !response.status().is_success() {
                    anyhow::bail!("HTTP GET failed with status {}", response.status());
                }
                let content_type = response
                    .headers()
                    .get(http::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if content_type.starts_with("image/") {
                    return Ok(OgpResult {
                        url: page_url.clone(),
                        image: page_url,
                        is_image: true,
                        ..Default::default()
                    });
                }
                if !content_type.is_empty() && !content_type.contains("html") {
                    anyhow::bail!("unsupported content-type: {content_type}");
                }
                let content_encoding = response
                    .headers()
                    .get(http::header::CONTENT_ENCODING)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                let mut bytes = Vec::new();
                let mut limited = response.body_mut().take(OGP_MAX_HTML_BYTES as u64);
                limited.read_to_end(&mut bytes).await?;
                let decoded = decompress_body(&bytes, &content_encoding);
                let html = match &decoded {
                    Some(decoded) => String::from_utf8_lossy(decoded),
                    None => String::from_utf8_lossy(&bytes),
                };
                let mut result = parse_ogp_html(&html);
                result.image = resolve_url(&page_url, &result.image);
                result.url = page_url;
                if result.title.is_empty()
                    && result.description.is_empty()
                    && result.image.is_empty()
                {
                    anyhow::bail!("no OGP metadata found");
                }
                Ok(result)
            })
            .await;
            match outcome {
                Ok(inner) => inner,
                Err(_) => anyhow::bail!("OGP fetch timed out after {}s", OGP_TIMEOUT.as_secs()),
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("ogp fetch task failed: {e}"))?
}

fn decompress_body(bytes: &[u8], content_encoding: &str) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let mut out = Vec::new();
    match content_encoding {
        "gzip" | "x-gzip" => {
            let _ = flate2::read::GzDecoder::new(bytes)
                .take(OGP_MAX_DECOMPRESSED as u64)
                .read_to_end(&mut out);
        }
        "deflate" => {
            let _ = flate2::read::ZlibDecoder::new(bytes)
                .take(OGP_MAX_DECOMPRESSED as u64)
                .read_to_end(&mut out);
        }
        _ => return None,
    }
    Some(out)
}

fn parse_ogp_html(html: &str) -> OgpResult {
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();

    let mut og_title = String::new();
    let mut og_description = String::new();
    let mut og_image = String::new();
    let mut twitter_title = String::new();
    let mut twitter_description = String::new();
    let mut twitter_image = String::new();
    let mut meta_description = String::new();

    for (start, _) in lower.match_indices("<meta") {
        let tag_start = start + "<meta".len();
        let mut end = tag_start;
        let mut quote = 0u8;
        while end < bytes.len() {
            let byte = bytes[end];
            if quote != 0 {
                if byte == quote {
                    quote = 0;
                }
            } else if byte == b'"' || byte == b'\'' {
                quote = byte;
            } else if byte == b'>' {
                break;
            }
            end += 1;
        }
        let attrs = parse_attributes(&html[tag_start..end.min(bytes.len())]);
        let Some(key) = attrs
            .get("property")
            .or_else(|| attrs.get("name"))
            .map(|value| value.to_ascii_lowercase())
        else {
            continue;
        };
        let Some(content) = attrs.get("content") else {
            continue;
        };
        let content = decode_entities(content.trim());
        if content.is_empty() {
            continue;
        }
        match key.as_str() {
            "og:title" if og_title.is_empty() => og_title = content,
            "og:description" if og_description.is_empty() => og_description = content,
            "og:image" | "og:image:url" | "og:image:secure_url" if og_image.is_empty() => {
                og_image = content
            }
            "twitter:title" if twitter_title.is_empty() => twitter_title = content,
            "twitter:description" if twitter_description.is_empty() => {
                twitter_description = content
            }
            "twitter:image" | "twitter:image:src" if twitter_image.is_empty() => {
                twitter_image = content
            }
            "description" if meta_description.is_empty() => meta_description = content,
            _ => {}
        }
    }

    OgpResult {
        title: first_nonempty([og_title, twitter_title, extract_title(html, &lower)]),
        description: first_nonempty([og_description, twitter_description, meta_description]),
        image: first_nonempty([og_image, twitter_image]),
        ..Default::default()
    }
}

fn parse_attributes(tag: &str) -> HashMap<String, String> {
    let mut attributes = HashMap::new();
    let bytes = tag.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len()
            && bytes[i] != b'='
            && bytes[i] != b'>'
            && bytes[i] != b'/'
            && !bytes[i].is_ascii_whitespace()
        {
            i += 1;
        }
        if i == name_start {
            i += 1;
            continue;
        }
        let name = tag[name_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let value_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                value = tag[value_start..i.min(bytes.len())].to_string();
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                let value_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                value = tag[value_start..i].to_string();
            }
        }
        attributes.entry(name).or_insert(value);
    }
    attributes
}

fn extract_title(html: &str, lower: &str) -> String {
    let Some(open) = lower.find("<title") else {
        return String::new();
    };
    let Some(gt) = lower[open..].find('>') else {
        return String::new();
    };
    let content_start = open + gt + 1;
    let Some(close) = lower[content_start..].find("</title>") else {
        return String::new();
    };
    decode_entities(html[content_start..content_start + close].trim())
}

fn decode_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        if let Some(semi) = after.find(';').filter(|semi| *semi <= 8)
            && let Some(decoded) = decode_entity(&after[..semi])
        {
            out.push(decoded);
            rest = &after[semi + 1..];
            continue;
        }
        out.push('&');
        rest = after;
    }
    out.push_str(rest);
    out
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{00a0}'),
        _ => {
            if let Some(hex) = entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
            {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else if let Some(dec) = entity.strip_prefix('#') {
                dec.parse::<u32>().ok().and_then(char::from_u32)
            } else {
                None
            }
        }
    }
}

fn resolve_url(base: &str, target: &str) -> String {
    let target = target.trim();
    if target.is_empty() {
        return String::new();
    }
    if let Ok(absolute) = Url::parse(target) {
        return match absolute.scheme() {
            "http" | "https" => absolute.to_string(),
            _ => String::new(),
        };
    }
    if let Ok(base_url) = Url::parse(base)
        && let Ok(joined) = base_url.join(target)
        && matches!(joined.scheme(), "http" | "https")
    {
        return joined.to_string();
    }
    String::new()
}

fn first_nonempty<const N: usize>(candidates: [String; N]) -> String {
    candidates
        .into_iter()
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_open_graph_tags() {
        let html = r#"<html><head>
            <title>Fallback Title</title>
            <meta property="og:title" content="Real Title &amp; More" />
            <meta property="og:description" content='A great description'>
            <meta property="og:image" content="/img/cover.png">
        </head></html>"#;
        let result = parse_ogp_html(html);
        assert_eq!(result.title, "Real Title & More");
        assert_eq!(result.description, "A great description");
        assert_eq!(result.image, "/img/cover.png");
    }

    #[test]
    fn falls_back_to_twitter_and_title() {
        let html = r#"<head>
            <title>Page &#39;Title&#39;</title>
            <meta name="twitter:image" content="https://cdn.example.com/a.jpg">
            <meta name="description" content="meta desc">
        </head>"#;
        let result = parse_ogp_html(html);
        assert_eq!(result.title, "Page 'Title'");
        assert_eq!(result.description, "meta desc");
        assert_eq!(result.image, "https://cdn.example.com/a.jpg");
    }

    #[test]
    fn resolves_relative_image_against_base() {
        assert_eq!(
            resolve_url("https://example.com/blog/post", "/img/cover.png"),
            "https://example.com/img/cover.png"
        );
        assert_eq!(
            resolve_url("https://example.com/blog/post", "//cdn.example.com/x.png"),
            "https://cdn.example.com/x.png"
        );
        assert_eq!(
            resolve_url("https://example.com/", "https://other.com/y.png"),
            "https://other.com/y.png"
        );
        assert_eq!(
            resolve_url("https://example.com/", "javascript:void(0)"),
            ""
        );
    }

    #[test]
    fn empty_when_no_metadata() {
        let result = parse_ogp_html("<html><body>nothing here</body></html>");
        assert!(result.title.is_empty());
        assert!(result.description.is_empty());
        assert!(result.image.is_empty());
    }

    #[test]
    fn decompresses_gzip_and_parses_tags() {
        use std::io::Write as _;
        let html = r#"<head><meta property="og:title" content="Zed"><meta property="og:image" content="https://z.dev/a.png"></head>"#;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(html.as_bytes()).unwrap();
        let gz = encoder.finish().unwrap();

        let decoded = decompress_body(&gz, "gzip").expect("gzip decodes");
        assert_eq!(decoded, html.as_bytes());
        assert!(decompress_body(html.as_bytes(), "").is_none());

        let result = parse_ogp_html(&String::from_utf8_lossy(&decoded));
        assert_eq!(result.title, "Zed");
        assert_eq!(result.image, "https://z.dev/a.png");
    }
}
