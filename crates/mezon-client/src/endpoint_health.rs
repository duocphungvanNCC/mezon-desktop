use std::time::{Duration, Instant};

/// A round trip this slow is worth telling the gateway about.
const SLOW_RTT: Duration = Duration::from_millis(500);
/// One slow heartbeat is a hiccup; three in a row is the link.
const SLOW_STREAK_REQUIRED: u8 = 3;
/// How long we settle on a node before calling it slow, and the gap between reports.
const SLOW_SWITCH_COOLDOWN: Duration = Duration::from_secs(120);

/// The realtime node this session belongs to.
///
/// `/v2/healthy/endpoint` answers with exactly one node and the gateway has already
/// decided it is the right one, so this is never one option among several — it is
/// simply where we are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeEndpoint {
    /// The gateway's own id for this node. Empty when it did not send one.
    pub id: String,
    pub host: String,
    pub port: u16,
}

impl RealtimeEndpoint {
    /// The value the gateway reads back as `currentEndpointId`. It has no id for
    /// machine 0 either — proto3 omits a zero — so 0 means "you tell me".
    pub fn backend_id(&self) -> i32 {
        self.id.parse().unwrap_or_default()
    }

    pub fn label(&self) -> String {
        if self.id.is_empty() {
            format!("{}:{}", self.host, self.port)
        } else {
            format!("{} ({}:{})", self.id, self.host, self.port)
        }
    }
}

/// How the node we are on is behaving.
///
/// Nothing here ranks or picks: the gateway owns that. This only watches the link
/// the gateway handed us, so the connection manager knows when the gateway is worth
/// asking again — and, just as importantly, when it is not.
#[derive(Debug, Default)]
pub struct EndpointHealth {
    endpoint: Option<RealtimeEndpoint>,
    connected_since: Option<Instant>,
    ewma_rtt: Option<Duration>,
    slow_streak: u8,
    slow_report_suppressed_until: Option<Instant>,
    slow_reports_disabled: bool,
}

impl EndpointHealth {
    /// Point at the node the session now names. Latency history belongs to a node,
    /// so moving to a different one starts its record from scratch; being handed the
    /// same node again (every loop iteration does that) changes nothing.
    pub fn set_endpoint(&mut self, endpoint: Option<RealtimeEndpoint>) {
        if self.endpoint == endpoint {
            return;
        }
        self.endpoint = endpoint;
        self.connected_since = None;
        self.ewma_rtt = None;
        self.slow_streak = 0;
        self.slow_report_suppressed_until = None;
        self.slow_reports_disabled = false;
    }

    pub fn endpoint(&self) -> Option<&RealtimeEndpoint> {
        self.endpoint.as_ref()
    }

    /// The node id while a connection is live — `None` once it drops, so a callback
    /// from a retired socket can tell that its observation no longer applies.
    pub fn connected_id(&self) -> Option<&str> {
        self.connected_since?;
        self.endpoint.as_ref().map(|endpoint| endpoint.id.as_str())
    }

    pub fn connected_endpoint(&self) -> Option<RealtimeEndpoint> {
        self.connected_since?;
        self.endpoint.clone()
    }

    pub fn record_connected(&mut self, now: Instant) {
        self.connected_since = Some(now);
        self.slow_streak = 0;
        self.slow_reports_disabled = false;
    }

    pub fn record_disconnected(&mut self) {
        self.connected_since = None;
        self.slow_streak = 0;
    }

    /// Fold one heartbeat round trip in. Returns whether the gateway should hear
    /// about it: only after we have settled on the node, and only for a run of slow
    /// samples, because every report costs the user one of ten session slots.
    pub fn record_active_probe(&mut self, rtt: Duration, now: Instant) -> bool {
        let Some(connected_since) = self.connected_since else {
            return false;
        };
        update_ewma(&mut self.ewma_rtt, rtt);
        if self.slow_reports_disabled
            || self
                .slow_report_suppressed_until
                .is_some_and(|until| until > now)
        {
            self.slow_streak = 0;
            return false;
        }
        let settled = now.saturating_duration_since(connected_since) >= SLOW_SWITCH_COOLDOWN;
        if settled && self.ewma_rtt.is_some_and(|sample| sample >= SLOW_RTT) {
            self.slow_streak = self.slow_streak.saturating_add(1);
        } else {
            self.slow_streak = 0;
        }
        if self.slow_streak < SLOW_STREAK_REQUIRED {
            return false;
        }
        self.slow_streak = 0;
        self.slow_report_suppressed_until = Some(now + SLOW_SWITCH_COOLDOWN);
        true
    }

    /// Stop reporting high latency until the next successful connect. Called when the
    /// gateway answers a slow-link report with the same node: it has decided this is
    /// where we belong, and asking again every two minutes only burns session slots
    /// for a link that is slow but working.
    pub fn disable_slow_reports(&mut self) {
        self.slow_reports_disabled = true;
        self.slow_streak = 0;
    }
}

fn update_ewma(slot: &mut Option<Duration>, sample: Duration) {
    *slot = Some(match *slot {
        Some(previous) => {
            let millis = previous
                .as_millis()
                .saturating_mul(3)
                .saturating_add(sample.as_millis())
                / 4;
            Duration::from_millis(u64::try_from(millis).unwrap_or(u64::MAX))
        }
        None => sample,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(id: &str) -> RealtimeEndpoint {
        RealtimeEndpoint {
            id: id.into(),
            host: format!("sock-{id}.example.com"),
            port: 4433,
        }
    }

    #[test]
    fn a_slow_link_is_reported_only_after_settling_and_three_samples() {
        let now = Instant::now();
        let mut health = EndpointHealth::default();
        health.set_endpoint(Some(endpoint("1")));
        health.record_connected(now);

        assert!(!health.record_active_probe(Duration::from_millis(800), now));
        let settled = now + SLOW_SWITCH_COOLDOWN;
        assert!(!health.record_active_probe(Duration::from_millis(800), settled));
        assert!(!health.record_active_probe(Duration::from_millis(800), settled));
        assert!(health.record_active_probe(Duration::from_millis(800), settled));
        // Reporting arms its own cooldown, so the next samples stay quiet.
        assert!(!health.record_active_probe(Duration::from_millis(800), settled));
        assert!(
            !health.record_active_probe(Duration::from_millis(800), settled + SLOW_SWITCH_COOLDOWN)
        );
    }

    #[test]
    fn a_gateway_confirmed_slow_node_stops_reporting_until_the_next_connect() {
        let now = Instant::now();
        let mut health = EndpointHealth::default();
        health.set_endpoint(Some(endpoint("1")));
        health.record_connected(now);

        let settled = now + SLOW_SWITCH_COOLDOWN;
        for _ in 0..2 {
            assert!(!health.record_active_probe(Duration::from_millis(800), settled));
        }
        assert!(health.record_active_probe(Duration::from_millis(800), settled));

        // The gateway answered with this same node, so stop asking.
        health.disable_slow_reports();
        let much_later = settled + SLOW_SWITCH_COOLDOWN * 10;
        for _ in 0..10 {
            assert!(!health.record_active_probe(Duration::from_millis(800), much_later));
        }

        // A fresh connect re-arms it — the link may genuinely have changed.
        health.record_connected(much_later);
        let resettled = much_later + SLOW_SWITCH_COOLDOWN;
        for _ in 0..2 {
            assert!(!health.record_active_probe(Duration::from_millis(800), resettled));
        }
        assert!(health.record_active_probe(Duration::from_millis(800), resettled));
    }

    #[test]
    fn moving_to_another_node_starts_its_history_from_scratch() {
        let now = Instant::now();
        let mut health = EndpointHealth::default();
        health.set_endpoint(Some(endpoint("1")));
        health.record_connected(now);
        health.disable_slow_reports();

        health.set_endpoint(Some(endpoint("2")));
        assert_eq!(
            health.connected_id(),
            None,
            "a new node is not connected yet"
        );
        health.record_connected(now);

        let settled = now + SLOW_SWITCH_COOLDOWN;
        for _ in 0..2 {
            assert!(!health.record_active_probe(Duration::from_millis(800), settled));
        }
        assert!(
            health.record_active_probe(Duration::from_millis(800), settled),
            "the previous node's suppression must not follow us"
        );
    }

    #[test]
    fn being_handed_the_same_node_again_keeps_the_live_connection() {
        let now = Instant::now();
        let mut health = EndpointHealth::default();
        health.set_endpoint(Some(endpoint("1")));
        health.record_connected(now);

        health.set_endpoint(Some(endpoint("1")));

        assert_eq!(health.connected_id(), Some("1"));
    }

    #[test]
    fn a_dropped_connection_retires_the_observation() {
        let now = Instant::now();
        let mut health = EndpointHealth::default();
        health.set_endpoint(Some(endpoint("1")));
        health.record_connected(now);
        health.record_disconnected();

        assert_eq!(health.connected_id(), None);
        assert!(!health.record_active_probe(Duration::from_millis(800), now));
    }

    #[test]
    fn a_missing_gateway_id_reads_back_as_unknown() {
        assert_eq!(endpoint("2").backend_id(), 2);
        assert_eq!(endpoint("").backend_id(), 0);
        assert_eq!(endpoint("legacy").backend_id(), 0);
    }
}
