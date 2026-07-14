use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const BAKED_ENV_VARS: &[&str] = &[
    "NX_CHAT_APP_API_HOST",
    "NX_CHAT_APP_API_PORT",
    "NX_CHAT_APP_API_SECURE",
    "NX_CHAT_APP_API_KEY",
    "NX_CHAT_APP_API_GW_HOST",
    "NX_CHAT_APP_API_GW_PORT",
    "NX_CHAT_APP_TCP_PORT",
    "NX_CHAT_APP_STREAM_WS_URL",
    "NX_CHAT_APP_MEET_WS_URL",
    "NX_CHAT_APP_NOTIFICATION_WS_URL",
    "NX_CHAT_APP_OAUTH2_AUTHORIZE_URL",
    "NX_CHAT_APP_OAUTH2_CLIENT_ID",
    "NX_CHAT_APP_OAUTH2_REDIRECT_URI",
    "NX_CHAT_APP_OAUTH2_RESPONSE_TYPE",
    "NX_CHAT_APP_OAUTH2_SCOPE",
    "NX_CHAT_APP_OAUTH2_CODE_CHALLENGE_METHOD",
    "NX_CHAT_APP_OAUTH2_LOG_OUT",
    "NX_CHAT_APP_OAUTH2_LOG_OUT_CALLBACK",
    "NX_CHAT_APP_GOOGLE_CLIENT_ID",
    "NX_DOMAIN_URL",
    "NX_CHAT_APP_REDIRECT_URI",
    "NX_LOGO_MEZON",
    "NX_BASE_IMG_URL",
    "NX_PROFILE_IMG_URL",
    "NX_IMGPROXY_BASE_URL",
    "NX_IMGPROXY_KEY",
    "NX_CHAT_APP_API_KLIPY_KEY",
    "NX_CHAT_APP_API_KLIPY_URL",
    "NX_CHAT_APP_MEZON_TREASURY_URL",
    "NX_CHAT_APP_API_MEZONTREASURY_KEY",
    "NX_CHAT_APP_CONTRACT_ADDRESS",
    "NX_CHAT_APP_MEZON_TREASURY_URL_NETWORK",
    "NX_WEBRTC_ICESERVERS_URL",
    "NX_WEBRTC_ICESERVERS_USERNAME",
    "NX_WEBRTC_ICESERVERS_CREDENTIAL",
    "NX_CHAT_APP_FCM_API_KEY",
    "NX_CHAT_APP_FCM_AUTH_DOMAIN",
    "NX_CHAT_APP_FCM_PROJECT_ID",
    "NX_CHAT_APP_FCM_STORAGE_BUCKET",
    "NX_CHAT_APP_FCM_MESSAGING_SENDER_ID",
    "NX_CHAT_APP_FCM_APP_ID",
    "NX_CHAT_APP_FCM_MEASUREMENT_ID",
    "NX_CHAT_APP_FCM_VAPID_KEY",
    "NX_CHAT_APP_API_CLIENT_KEY_CUSTOM",
    "NX_CHAT_SENTRY_DSN",
    "NX_CHAT_SENTRY_DNS",
    "NX_CHAT_APP_ANNONYMOUS_USER_ID",
    "NX_MAX_LENGTH_NAME_ALLOWED",
    "NX_UPDATE_URL",
];

fn workspace_dotenv() -> Option<PathBuf> {
    let manifest = env::var("CARGO_MANIFEST_DIR").ok()?;
    let manifest_dir = PathBuf::from(manifest);
    let root = manifest_dir.parent()?.parent()?;
    let dotenv = root.join(".env");
    dotenv.is_file().then_some(dotenv)
}

fn parse_dotenv(path: &Path) -> HashMap<String, String> {
    let Ok(content) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    let mut vars = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let value = strip_quotes(raw_value.trim()).to_owned();
        vars.insert(key.to_owned(), value);
    }
    vars
}

fn strip_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
        {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn baked_value(var: &str, dotenv: &HashMap<String, String>) -> Option<String> {
    if let Ok(value) = env::var(var) {
        return Some(value);
    }
    dotenv.get(var).cloned()
}

fn main() {
    let dotenv = workspace_dotenv()
        .map(|path| {
            println!("cargo:rerun-if-changed={}", path.display());
            parse_dotenv(&path)
        })
        .unwrap_or_default();

    for var in BAKED_ENV_VARS {
        println!("cargo:rerun-if-env-changed={var}");
        if let Some(value) = baked_value(var, &dotenv) {
            println!("cargo:rustc-env={var}={value}");
        }
    }
}
