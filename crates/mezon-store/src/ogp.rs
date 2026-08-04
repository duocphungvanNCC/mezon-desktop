pub use mezon_client::transport::OutgoingOgp;
pub use mezon_client::{OgpResult, fetch_invite_preview, fetch_ogp};

const EXCLUDED_HOSTS: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "facebook.com",
    "fb.watch",
    "fb.com",
    "tiktok.com",
];

const TRAILING_PUNCTUATION: &[char] = &['.', ',', ';', ':', '!', '?', ')', ']', '}', '\'', '"'];

pub fn invite_id_from_url(url: &str) -> Option<String> {
    const NEEDLE: &[u8] = b"/invite/";
    let idx = url
        .as_bytes()
        .windows(NEEDLE.len())
        .position(|window| window.eq_ignore_ascii_case(NEEDLE))?;
    let rest = &url[idx + NEEDLE.len()..];
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(rest.len());
    let id = &rest[..end];
    (!id.is_empty()).then(|| id.to_string())
}

/// The first URL in `text` eligible for an OGP link preview, or `None`.
///
/// Mirrors the mezon-react rule: the first `http`/`https` link that is not an
/// internal-domain link and not a YouTube/Facebook/TikTok link.
pub fn first_previewable_url(text: &str, internal_domain: Option<&str>) -> Option<String> {
    extract_urls(text)
        .into_iter()
        .find(|url| is_previewable(url, internal_domain))
}

fn extract_urls(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut rest = text;
    while let Some(pos) = next_scheme(rest) {
        let from = &rest[pos..];
        let end = from
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '|' | '`'))
            .unwrap_or(from.len());
        let token = from[..end].trim_end_matches(TRAILING_PUNCTUATION);
        if token.len() > "https://".len() {
            urls.push(token.to_string());
        }
        let advance = pos + end.max(1);
        if advance >= rest.len() {
            break;
        }
        rest = &rest[advance..];
    }
    urls
}

fn next_scheme(text: &str) -> Option<usize> {
    match (text.find("http://"), text.find("https://")) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn is_previewable(url: &str, internal_domain: Option<&str>) -> bool {
    let Some(host) = host_of(url) else {
        return false;
    };
    if EXCLUDED_HOSTS
        .iter()
        .any(|domain| host_matches(&host, domain))
    {
        return false;
    }
    if let Some(internal) = internal_domain {
        let internal = internal
            .trim()
            .trim_start_matches("www.")
            .to_ascii_lowercase();
        if !internal.is_empty() && host_matches(&host, &internal) {
            return false;
        }
    }
    true
}

fn host_of(url: &str) -> Option<String> {
    let after = url.split_once("://")?.1;
    let authority = after.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn host_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_first_plain_url() {
        assert_eq!(
            first_previewable_url("check this https://example.com/post out", None).as_deref(),
            Some("https://example.com/post")
        );
    }

    #[test]
    fn strips_trailing_punctuation() {
        assert_eq!(
            first_previewable_url("see https://example.com/a).", None).as_deref(),
            Some("https://example.com/a")
        );
    }

    #[test]
    fn skips_social_links() {
        assert_eq!(
            first_previewable_url("https://www.youtube.com/watch?v=1", None),
            None
        );
        assert_eq!(first_previewable_url("https://youtu.be/abc", None), None);
        assert_eq!(
            first_previewable_url("a https://tiktok.com/@x https://example.com/b", None).as_deref(),
            Some("https://example.com/b")
        );
    }

    #[test]
    fn skips_internal_domain() {
        assert_eq!(
            first_previewable_url("https://mezon.ai/channels/1", Some("mezon.ai")),
            None
        );
        assert_eq!(
            first_previewable_url("https://app.mezon.ai/x", Some("mezon.ai")),
            None
        );
    }

    #[test]
    fn none_when_no_url() {
        assert_eq!(first_previewable_url("just some text", None), None);
    }

    #[test]
    fn extracts_invite_id() {
        assert_eq!(
            invite_id_from_url("https://mezon.ai/invite/1840670747886882816").as_deref(),
            Some("1840670747886882816")
        );
        assert_eq!(
            invite_id_from_url("https://mezon.ai/invite/abc-DEF_123?ref=x").as_deref(),
            Some("abc-DEF_123")
        );
        assert_eq!(
            invite_id_from_url("https://mezon.ai/channels/1").as_deref(),
            None
        );
        assert_eq!(
            invite_id_from_url("https://mezon.ai/invite/").as_deref(),
            None
        );
    }
}
