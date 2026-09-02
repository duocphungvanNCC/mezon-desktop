use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

use crate::EndpointCandidate;

#[cfg(debug_assertions)]
const DEBUG_FAILOVER_SIMULATION_ENV: &str = "MEZON_DEBUG_FAILOVER_SIMULATION";
#[cfg(debug_assertions)]
const DEBUG_FAILOVER_UNREACHABLE_PRIMARY: &str = "unreachable-primary";
#[cfg(debug_assertions)]
const DEBUG_FAILOVER_FULL_CYCLE: &str = "full-cycle";
#[cfg(debug_assertions)]
const DEBUG_FAILOVER_SLOW_SWITCH: &str = "slow-switch";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct ServiceEndpoint {
    pub id: String,
    pub region: String,
    pub api_url: Option<String>,
    pub ws_url: Option<String>,
    pub tcp_url: Option<String>,
    pub priority: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum HealthyEndpointReason {
    Unreachable = 1,
    HighLatency = 2,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct HealthyEndpointSession {
    #[serde(alias = "userId")]
    pub user_id: String,
    #[serde(alias = "sessionId")]
    pub session_id: String,
    #[serde(alias = "apiUrl")]
    pub api_url: Option<String>,
    #[serde(alias = "wsUrl")]
    pub ws_url: Option<String>,
    #[serde(alias = "tcpUrl")]
    pub tcp_url: Option<String>,
    #[serde(alias = "endpointId")]
    pub endpoint_id: i32,
    /// Refreshed catalog. Absent on older gateways, in which case the pool we
    /// already hold stays as it is rather than collapsing to this one endpoint.
    #[serde(default)]
    pub endpoints: Vec<ServiceEndpoint>,
}

impl HealthyEndpointSession {
    pub fn realtime_endpoint(
        &self,
        region: &str,
        default_port: Option<u16>,
    ) -> Option<EndpointCandidate> {
        let (tcp_host, tcp_port, _) = parse_endpoint(self.tcp_url.as_deref());
        let (ws_host, ws_port, _) = parse_endpoint(self.ws_url.as_deref());
        let host = tcp_host.or(ws_host)?;
        let port = tcp_port.or(ws_port).or(default_port).unwrap_or(443);
        Some(EndpointCandidate {
            id: if self.endpoint_id > 0 {
                self.endpoint_id.to_string()
            } else {
                format!("backend:{host}:{port}")
            },
            region: region.to_string(),
            api_url: self.api_url.clone().filter(|url| !url.is_empty()),
            host,
            port,
            priority: 0,
        })
    }
}

/// Authenticated session returned after login.
/// Mirrors the mezon-js Session object.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Session {
    /// Bearer token for API requests
    pub token: String,
    /// Refresh token for obtaining a new token
    pub refresh_token: String,
    /// OpenID id_token (JWT) used for MMN zk-proof generation
    #[serde(default)]
    pub id_token: String,
    /// Socket credential (`AAA…`) — preferred for WebSocket connect (mezon-js uses this over JWT).
    #[serde(default)]
    pub session_id: String,
    /// Unix timestamp (seconds) when the token expires
    pub expires_at: u64,
    pub is_remember: bool,
    /// The WebSocket endpoint URL returned by the server after auth
    pub ws_url: Option<String>,
    /// Parsed WebSocket host returned by the server after auth
    pub ws_host: Option<String>,
    /// Parsed WebSocket port returned by the server after auth
    pub ws_port: Option<u16>,
    /// Whether WebSocket endpoint uses TLS
    pub ws_secure: Option<bool>,
    /// The REST API endpoint URL returned by the server after auth
    pub api_url: Option<String>,
    /// Parsed REST API host returned by the server after auth
    pub api_host: Option<String>,
    /// Parsed REST API port returned by the server after auth
    pub api_port: Option<u16>,
    /// Whether REST API endpoint uses TLS
    pub api_secure: Option<bool>,
    /// The TCP endpoint URL returned by the server after auth
    pub tcp_url: Option<String>,
    /// Parsed TCP host returned by the server after auth
    pub tcp_host: Option<String>,
    /// Parsed TCP port returned by the server after auth
    pub tcp_port: Option<u16>,
    #[serde(default)]
    pub endpoints: Vec<ServiceEndpoint>,
    /// User ID
    pub user_id: String,
    /// Username
    pub username: String,
}

impl Session {
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.expires_at == 0 || now >= self.expires_at
    }

    /// Credential for `wss://…?token=…` — matches mezon-js (`session_id` first, else JWT).
    pub fn ws_credential(&self) -> &str {
        if !self.session_id.is_empty() {
            &self.session_id
        } else {
            &self.token
        }
    }

    /// Apply a server-pushed `refresh_session_event` (mezon-js `onrefreshsession`):
    /// adopt the new token / session_id and recompute expiry from the new JWT.
    pub fn apply_refresh(
        &mut self,
        token: &str,
        refresh_token: &str,
        session_id: &str,
        id_token: &str,
    ) {
        if !token.is_empty() {
            let (user_id, username, expires_at) = decode_jwt_claims(token);
            self.token = token.to_string();
            if let Some(exp) = expires_at {
                self.expires_at = exp;
            }
            if !user_id.is_empty() {
                self.user_id = user_id;
            }
            if !username.is_empty() {
                self.username = username;
            }
        }
        if !refresh_token.is_empty() {
            self.refresh_token = refresh_token.to_string();
        }
        if !session_id.is_empty() {
            self.session_id = session_id.to_string();
        }
        if !id_token.is_empty() {
            self.id_token = id_token.to_string();
        }
    }

    pub fn apply_healthy_endpoint(
        &mut self,
        endpoint: &HealthyEndpointSession,
        region: &str,
        default_port: Option<u16>,
        endpoint_id: Option<&str>,
    ) -> bool {
        let Some(candidate) = endpoint.realtime_endpoint(region, default_port) else {
            return false;
        };
        if !endpoint.user_id.is_empty() && endpoint.user_id != self.user_id {
            return false;
        }
        if !endpoint.session_id.is_empty() {
            self.session_id = endpoint.session_id.clone();
        }

        let api_url = endpoint
            .api_url
            .clone()
            .filter(|url| !url.is_empty())
            .or_else(|| self.api_url.clone());
        let ws_url = endpoint
            .ws_url
            .clone()
            .filter(|url| !url.is_empty())
            .or_else(|| self.ws_url.clone());
        let tcp_url = endpoint
            .tcp_url
            .clone()
            .filter(|url| !url.is_empty())
            .or_else(|| self.tcp_url.clone());
        let (api_host, api_port, api_secure) = parse_endpoint(api_url.as_deref());
        let (ws_host, ws_port, ws_secure) = parse_endpoint(ws_url.as_deref());
        let (tcp_host, tcp_port, _) = parse_endpoint(tcp_url.as_deref());

        self.api_url = api_url.clone();
        self.api_host = api_host;
        self.api_port = api_port;
        self.api_secure = api_secure;
        self.ws_url = ws_url.clone();
        self.ws_host = ws_host;
        self.ws_port = ws_port;
        self.ws_secure = ws_secure;
        self.tcp_url = tcp_url.clone();
        self.tcp_host = tcp_host;
        self.tcp_port = tcp_port;
        let current = ServiceEndpoint {
            id: endpoint_id.unwrap_or(&candidate.id).to_string(),
            region: candidate.region,
            api_url,
            ws_url,
            tcp_url,
            priority: 0,
        };
        // A fresh catalog replaces the old one wholesale. Without one, keep every
        // endpoint we already knew about and only update the entry we just moved
        // to — overwriting the list with a single entry would delete the
        // alternates and leave the next outage with nowhere to go.
        if !endpoint.endpoints.is_empty() {
            self.endpoints = endpoint.endpoints.clone();
        } else if let Some(slot) = self
            .endpoints
            .iter_mut()
            .find(|existing| existing.id == current.id)
        {
            *slot = current;
        } else {
            self.endpoints.insert(0, current);
        }
        true
    }

    pub fn realtime_endpoints(
        &self,
        default_host: &str,
        default_port: Option<u16>,
    ) -> Vec<EndpointCandidate> {
        let mut candidates = Vec::new();
        let mut addresses = std::collections::HashSet::new();
        let mut ids = std::collections::HashSet::new();

        for (index, endpoint) in self.endpoints.iter().enumerate() {
            let (tcp_host, tcp_port, _) = parse_endpoint(endpoint.tcp_url.as_deref());
            let (ws_host, ws_port, _) = parse_endpoint(endpoint.ws_url.as_deref());
            let Some(host) = tcp_host.or(ws_host) else {
                continue;
            };
            let port = tcp_port.or(ws_port).or(default_port).unwrap_or(443);
            if !addresses.insert((host.clone(), port)) {
                continue;
            }
            let mut id = if endpoint.id.is_empty() {
                format!("endpoint-{index}")
            } else {
                endpoint.id.clone()
            };
            if !ids.insert(id.clone()) {
                id = format!("{id}-{index}");
                ids.insert(id.clone());
            }
            candidates.push(EndpointCandidate {
                id,
                region: endpoint.region.clone(),
                api_url: endpoint.api_url.clone().filter(|url| !url.is_empty()),
                host,
                port,
                priority: endpoint.priority,
            });
        }

        if candidates.is_empty() {
            let host = self
                .tcp_host
                .clone()
                .or_else(|| self.ws_host.clone())
                .unwrap_or_else(|| default_host.to_string());
            let port = self
                .tcp_port
                .or(self.ws_port)
                .or(default_port)
                .unwrap_or(443);
            candidates.push(EndpointCandidate {
                id: "legacy".to_string(),
                region: String::new(),
                api_url: self.api_url.clone().filter(|url| !url.is_empty()),
                host,
                port,
                priority: 0,
            });
        }

        #[cfg(debug_assertions)]
        match std::env::var(DEBUG_FAILOVER_SIMULATION_ENV).as_deref() {
            Ok(DEBUG_FAILOVER_UNREACHABLE_PRIMARY) => {
                inject_debug_unreachable_primary(&mut candidates);
            }
            Ok(DEBUG_FAILOVER_FULL_CYCLE | DEBUG_FAILOVER_SLOW_SWITCH) => {
                inject_debug_full_cycle(&mut candidates);
            }
            _ => {}
        }

        candidates.sort_by_key(|candidate| candidate.priority);
        candidates
    }
}

#[cfg(debug_assertions)]
fn inject_debug_unreachable_primary(candidates: &mut Vec<EndpointCandidate>) {
    for candidate in candidates.iter_mut() {
        candidate.priority = candidate.priority.saturating_add(1);
    }
    candidates.insert(
        0,
        EndpointCandidate {
            id: "debug-unreachable-primary".to_string(),
            region: "debug".to_string(),
            api_url: None,
            host: "127.0.0.1".to_string(),
            port: 1,
            priority: 0,
        },
    );
}

#[cfg(debug_assertions)]
fn inject_debug_full_cycle(candidates: &mut Vec<EndpointCandidate>) {
    let Some(mut primary) = candidates.first().cloned() else {
        return;
    };
    primary.id = "debug-primary".to_string();
    primary.region = "debug".to_string();
    primary.priority = 0;
    let mut secondary = primary.clone();
    secondary.id = "debug-secondary".to_string();
    secondary.priority = 1;
    *candidates = vec![primary, secondary];
}

pub(crate) fn parse_endpoint(
    endpoint: Option<&str>,
) -> (Option<String>, Option<u16>, Option<bool>) {
    let Some(endpoint) = endpoint else {
        return (None, None, None);
    };

    let endpoint = if endpoint.contains("://") {
        endpoint.to_owned()
    } else if endpoint.contains(':') {
        format!("tcp://{endpoint}")
    } else {
        return (Some(endpoint.to_owned()), None, Some(true));
    };

    let Ok(parsed) = url::Url::parse(&endpoint) else {
        return (None, None, None);
    };

    let secure = match parsed.scheme() {
        "https" | "wss" => Some(true),
        "http" | "ws" | "tcp" => Some(false),
        _ => None,
    };

    (
        parsed.host_str().map(str::to_owned),
        parsed.port_or_known_default(),
        secure,
    )
}

pub fn jwt_expires_at(token: &str) -> Option<u64> {
    decode_jwt_claims(token).2
}

pub(crate) fn decode_jwt_claims(token: &str) -> (String, String, Option<u64>) {
    let payload = token.split('.').nth(1).unwrap_or("");
    let decoded = URL_SAFE_NO_PAD.decode(payload).unwrap_or_default();
    let json: serde_json::Value = serde_json::from_slice(&decoded).unwrap_or_default();

    let user_id = json
        .get("uid")
        .and_then(|v| {
            v.as_str()
                .map(str::to_owned)
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        })
        .unwrap_or_default();
    let username = json
        .get("usn")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let expires_at = json
        .get("exp")
        .and_then(|v| v.as_u64())
        .filter(|&exp| exp > 0);

    (user_id, username, expires_at)
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_healthy_endpoint_without_a_catalog_keeps_the_alternates_we_already_knew() {
        let mut session = Session {
            user_id: "7".into(),
            endpoints: vec![
                ServiceEndpoint {
                    id: "0".into(),
                    tcp_url: Some("sock-a.example.com:4433".into()),
                    ..Default::default()
                },
                ServiceEndpoint {
                    id: "1".into(),
                    tcp_url: Some("sock-b.example.com:4433".into()),
                    priority: 1,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let moved = HealthyEndpointSession {
            user_id: "7".into(),
            tcp_url: Some("sock-b.example.com:4433".into()),
            endpoint_id: 1,
            ..Default::default()
        };
        assert!(session.apply_healthy_endpoint(&moved, "", Some(4433), None));

        // Both nodes must survive: collapsing to the one we moved to would leave
        // the next outage with nowhere to fail over.
        assert_eq!(session.endpoints.len(), 2);
        let ids = session
            .endpoints
            .iter()
            .map(|endpoint| endpoint.id.as_str())
            .collect::<Vec<_>>();
        assert!(ids.contains(&"0") && ids.contains(&"1"), "got {ids:?}");
    }

    #[test]
    fn a_healthy_endpoint_catalog_replaces_the_pool() {
        let mut session = Session {
            user_id: "7".into(),
            endpoints: vec![ServiceEndpoint {
                id: "0".into(),
                tcp_url: Some("old.example.com:4433".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let refreshed = HealthyEndpointSession {
            user_id: "7".into(),
            tcp_url: Some("new-a.example.com:4433".into()),
            endpoint_id: 5,
            endpoints: vec![
                ServiceEndpoint {
                    id: "5".into(),
                    tcp_url: Some("new-a.example.com:4433".into()),
                    ..Default::default()
                },
                ServiceEndpoint {
                    id: "6".into(),
                    tcp_url: Some("new-b.example.com:4433".into()),
                    priority: 1,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert!(session.apply_healthy_endpoint(&refreshed, "", Some(4433), None));
        assert_eq!(session.endpoints.len(), 2);
        assert_eq!(session.endpoints[0].id, "5");
    }
    use super::*;

    #[test]
    fn test_session_is_expired() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let session = Session {
            expires_at: 0,
            ..Default::default()
        };
        assert!(
            session.is_expired(),
            "missing expiry (0) must be treated as expired"
        );

        let session = Session {
            expires_at: now + 1000,
            ..Default::default()
        };
        assert!(!session.is_expired());

        let session = Session {
            expires_at: now - 10,
            ..Default::default()
        };
        assert!(session.is_expired());
    }

    fn fake_jwt(user_id: &str, username: &str, exp: u64) -> String {
        let claims = format!(r#"{{"uid":"{user_id}","usn":"{username}","exp":{exp}}}"#);
        format!("header.{}.signature", URL_SAFE_NO_PAD.encode(claims))
    }

    #[test]
    fn apply_refresh_adopts_the_new_token_and_its_expiry() {
        let mut session = Session {
            token: fake_jwt("7", "ngoc", 1000),
            refresh_token: "old-refresh".into(),
            session_id: "old-sid".into(),
            expires_at: 1000,
            user_id: "7".into(),
            username: "ngoc".into(),
            ..Default::default()
        };

        let renewed = fake_jwt("7", "ngoc", 9000);
        session.apply_refresh(&renewed, "new-refresh", "new-sid", "new-id-token");

        assert_eq!(session.token, renewed);
        assert_eq!(session.refresh_token, "new-refresh");
        assert_eq!(session.session_id, "new-sid");
        assert_eq!(session.expires_at, 9000);
        assert_eq!(session.id_token, "new-id-token");
    }

    #[test]
    fn apply_refresh_keeps_the_previous_id_token_when_the_server_sends_none() {
        let mut session = Session {
            id_token: "login-id-token".into(),
            ..Default::default()
        };

        session.apply_refresh("", "", "new-sid", "");

        assert_eq!(
            session.id_token, "login-id-token",
            "a refresh without an id_token must not strip the one zk proofs are minted from"
        );
    }

    #[test]
    fn apply_refresh_keeps_fields_the_server_left_empty() {
        let original = fake_jwt("7", "ngoc", 1000);
        let mut session = Session {
            token: original.clone(),
            refresh_token: "old-refresh".into(),
            session_id: "old-sid".into(),
            expires_at: 1000,
            ..Default::default()
        };

        session.apply_refresh("", "", "new-sid", "");

        assert_eq!(
            session.token, original,
            "an sid-only push must not clear the token"
        );
        assert_eq!(session.refresh_token, "old-refresh");
        assert_eq!(session.session_id, "new-sid");
        assert_eq!(session.expires_at, 1000);
    }

    #[test]
    fn endpoint_list_is_preferred_over_the_legacy_target() {
        let session = Session {
            tcp_host: Some("legacy.example.com".into()),
            endpoints: vec![
                ServiceEndpoint {
                    id: "secondary".into(),
                    tcp_url: Some("secondary.example.com:4433".into()),
                    priority: 2,
                    ..Default::default()
                },
                ServiceEndpoint {
                    id: "primary".into(),
                    api_url: Some("https://api-primary.example.com".into()),
                    ws_url: Some("wss://primary.example.com/socket".into()),
                    priority: 1,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let candidates = session.realtime_endpoints("default.example.com", Some(7349));
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].id, "primary");
        assert_eq!(candidates[0].host, "primary.example.com");
        assert_eq!(candidates[0].port, 443);
        assert_eq!(candidates[1].id, "secondary");
        assert_eq!(candidates[1].port, 4433);
    }

    #[test]
    fn legacy_session_still_produces_one_candidate() {
        let session = Session {
            api_url: Some("https://api.example.com".into()),
            tcp_host: Some("socket.example.com".into()),
            tcp_port: Some(4433),
            ..Default::default()
        };

        let candidates = session.realtime_endpoints("default.example.com", Some(7349));
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, "legacy");
        assert_eq!(candidates[0].host, "socket.example.com");
        assert_eq!(candidates[0].port, 4433);
    }

    #[test]
    fn healthy_endpoint_replaces_urls_and_socket_credential() {
        let mut session = Session {
            user_id: "7".into(),
            session_id: "old-sid".into(),
            api_url: Some("https://old-api.example.com".into()),
            tcp_url: Some("old-sock.example.com:4433".into()),
            tcp_host: Some("old-sock.example.com".into()),
            tcp_port: Some(4433),
            ..Default::default()
        };
        let response = HealthyEndpointSession {
            user_id: "7".into(),
            session_id: "new-sid".into(),
            api_url: Some("https://new-api.example.com".into()),
            ws_url: Some("wss://new-sock.example.com".into()),
            tcp_url: Some("new-sock.example.com:4433".into()),
            endpoint_id: 2,
            ..Default::default()
        };

        assert!(session.apply_healthy_endpoint(&response, "vn-south", Some(4433), None));
        assert_eq!(session.session_id, "new-sid");
        assert_eq!(session.tcp_host.as_deref(), Some("new-sock.example.com"));
        assert_eq!(session.endpoints.len(), 1);
        assert_eq!(session.endpoints[0].id, "2");
        assert_eq!(session.endpoints[0].region, "vn-south");
    }

    #[test]
    fn healthy_endpoint_rejects_another_users_session() {
        let mut session = Session {
            user_id: "7".into(),
            session_id: "old-sid".into(),
            ..Default::default()
        };
        let response = HealthyEndpointSession {
            user_id: "8".into(),
            session_id: "new-sid".into(),
            tcp_url: Some("new-sock.example.com:4433".into()),
            endpoint_id: 2,
            ..Default::default()
        };

        assert!(!session.apply_healthy_endpoint(&response, "", Some(4433), None));
        assert_eq!(session.session_id, "old-sid");
    }

    #[cfg(debug_assertions)]
    #[test]
    fn debug_failover_simulation_injects_an_unreachable_primary() {
        let mut candidates = vec![EndpointCandidate {
            id: "legacy".into(),
            region: String::new(),
            api_url: Some("https://api.example.com".into()),
            host: "socket.example.com".into(),
            port: 4433,
            priority: 0,
        }];

        inject_debug_unreachable_primary(&mut candidates);

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].id, "debug-unreachable-primary");
        assert_eq!(candidates[0].priority, 0);
        assert_eq!(candidates[1].id, "legacy");
        assert_eq!(candidates[1].priority, 1);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn debug_full_cycle_duplicates_the_live_endpoint() {
        let mut candidates = vec![EndpointCandidate {
            id: "legacy".into(),
            region: String::new(),
            api_url: Some("https://api.example.com".into()),
            host: "socket.example.com".into(),
            port: 4433,
            priority: 0,
        }];

        inject_debug_full_cycle(&mut candidates);

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].id, "debug-primary");
        assert_eq!(candidates[1].id, "debug-secondary");
        assert_eq!(candidates[0].host, candidates[1].host);
        assert_eq!(candidates[0].port, candidates[1].port);
    }
}
