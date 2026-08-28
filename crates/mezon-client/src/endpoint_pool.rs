use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

const FAILURE_COOLDOWN: Duration = Duration::from_secs(5);
const FAILURE_COOLDOWN_CAP: Duration = Duration::from_secs(60);
const SLOW_RTT: Duration = Duration::from_millis(500);
const SLOW_ADVANTAGE: Duration = Duration::from_millis(100);
const SLOW_RATIO_PERCENT: u128 = 70;
const SLOW_STREAK_REQUIRED: u8 = 3;
const SLOW_SWITCH_COOLDOWN: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointCandidate {
    pub id: String,
    pub region: String,
    pub api_url: Option<String>,
    pub host: String,
    pub port: u16,
    pub priority: u32,
}

#[derive(Debug, Clone, Default)]
struct EndpointHealth {
    consecutive_failures: u32,
    ewma_rtt: Option<Duration>,
    circuit_open_until: Option<Instant>,
    better_streak: u8,
    slow_report_suppressed_until: Option<Instant>,
}

#[derive(Debug, Default)]
pub struct EndpointPool {
    endpoints: Vec<EndpointCandidate>,
    health: HashMap<String, EndpointHealth>,
    active_id: Option<String>,
    active_since: Option<Instant>,
}

impl EndpointPool {
    pub fn replace(&mut self, endpoints: Vec<EndpointCandidate>) {
        let ids = endpoints
            .iter()
            .map(|endpoint| endpoint.id.as_str())
            .collect::<HashSet<_>>();
        self.health.retain(|id, _| ids.contains(id.as_str()));
        if self
            .active_id
            .as_ref()
            .is_some_and(|id| !ids.contains(id.as_str()))
        {
            self.active_id = None;
            self.active_since = None;
        }
        self.endpoints = endpoints;
        for endpoint in &self.endpoints {
            self.health.entry(endpoint.id.clone()).or_default();
        }
    }

    pub fn select(&self, now: Instant) -> Option<EndpointCandidate> {
        self.endpoints
            .iter()
            .filter(|endpoint| self.is_available(&endpoint.id, now))
            .min_by_key(|endpoint| {
                let health = self.health.get(&endpoint.id);
                let active_rank = u8::from(self.active_id.as_deref() != Some(endpoint.id.as_str()));
                let rtt = health
                    .and_then(|health| health.ewma_rtt)
                    .unwrap_or(Duration::MAX);
                (active_rank, endpoint.priority, rtt)
            })
            .cloned()
    }

    pub fn has_available(&self, now: Instant) -> bool {
        self.endpoints
            .iter()
            .any(|endpoint| self.is_available(&endpoint.id, now))
    }

    pub fn next_available_in(&self, now: Instant) -> Option<Duration> {
        self.endpoints
            .iter()
            .filter_map(|endpoint| {
                self.health
                    .get(&endpoint.id)
                    .and_then(|health| health.circuit_open_until)
                    .map(|until| until.saturating_duration_since(now))
            })
            .min()
    }

    pub fn record_connected(&mut self, id: &str, now: Instant) {
        self.active_id = Some(id.to_string());
        self.active_since = Some(now);
        if let Some(health) = self.health.get_mut(id) {
            health.consecutive_failures = 0;
            health.circuit_open_until = None;
            health.better_streak = 0;
        }
    }

    pub fn record_active_rtt(&mut self, rtt: Duration) {
        let Some(id) = self.active_id.as_deref() else {
            return;
        };
        if let Some(health) = self.health.get_mut(id) {
            update_ewma(&mut health.ewma_rtt, rtt);
        }
    }

    pub fn record_active_probe(&mut self, rtt: Duration, now: Instant) -> bool {
        let dwell_elapsed = self
            .active_since
            .is_some_and(|since| now.saturating_duration_since(since) >= SLOW_SWITCH_COOLDOWN);
        let Some(id) = self.active_id.as_deref() else {
            return false;
        };
        let Some(health) = self.health.get_mut(id) else {
            return false;
        };
        update_ewma(&mut health.ewma_rtt, rtt);
        if health
            .slow_report_suppressed_until
            .is_some_and(|until| until > now)
        {
            health.better_streak = 0;
            return false;
        }
        if dwell_elapsed && health.ewma_rtt.is_some_and(|sample| sample >= SLOW_RTT) {
            health.better_streak = health.better_streak.saturating_add(1);
        } else {
            health.better_streak = 0;
        }
        if health.better_streak < SLOW_STREAK_REQUIRED {
            return false;
        }
        health.better_streak = 0;
        health.slow_report_suppressed_until = Some(now + SLOW_SWITCH_COOLDOWN);
        true
    }

    pub fn record_unreachable(&mut self, id: &str, now: Instant) {
        let health = self.health.entry(id.to_string()).or_default();
        health.consecutive_failures = health.consecutive_failures.saturating_add(1);
        let shift = health.consecutive_failures.saturating_sub(1).min(6);
        let multiplier = 1u32 << shift;
        let cooldown = FAILURE_COOLDOWN
            .saturating_mul(multiplier)
            .min(FAILURE_COOLDOWN_CAP);
        health.circuit_open_until = Some(now + cooldown);
        health.better_streak = 0;
        if self.active_id.as_deref() == Some(id) {
            self.active_id = None;
            self.active_since = None;
        }
    }

    pub fn retry_now(&mut self, id: &str) {
        if let Some(health) = self.health.get_mut(id) {
            health.circuit_open_until = None;
        }
    }

    pub fn probe_endpoints(&self, now: Instant) -> Vec<EndpointCandidate> {
        let mut endpoints = self
            .endpoints
            .iter()
            .filter(|endpoint| self.is_available(&endpoint.id, now) && endpoint.api_url.is_some())
            .cloned()
            .collect::<Vec<_>>();
        endpoints.sort_by_key(|endpoint| {
            u8::from(self.active_id.as_deref() != Some(endpoint.id.as_str()))
        });
        endpoints
    }

    pub fn record_probe(&mut self, id: &str, rtt: Duration, now: Instant) -> bool {
        if self.active_id.as_deref() == Some(id) {
            self.record_active_probe(rtt, now);
            return false;
        }

        let Some(active_id) = self.active_id.clone() else {
            return false;
        };
        let active_rtt = self
            .health
            .get(&active_id)
            .and_then(|health| health.ewma_rtt);
        let candidate_health = self.health.entry(id.to_string()).or_default();
        update_ewma(&mut candidate_health.ewma_rtt, rtt);
        let candidate_rtt = candidate_health.ewma_rtt;

        let dwell_elapsed = self
            .active_since
            .is_some_and(|since| now.saturating_duration_since(since) >= SLOW_SWITCH_COOLDOWN);
        let better = active_rtt
            .zip(candidate_rtt)
            .is_some_and(|(active, candidate)| {
                active >= SLOW_RTT
                    && active.saturating_sub(candidate) >= SLOW_ADVANTAGE
                    && candidate.as_millis().saturating_mul(100)
                        <= active.as_millis().saturating_mul(SLOW_RATIO_PERCENT)
            });

        if better && dwell_elapsed {
            candidate_health.better_streak = candidate_health.better_streak.saturating_add(1);
        } else {
            candidate_health.better_streak = 0;
        }

        if candidate_health.better_streak < SLOW_STREAK_REQUIRED {
            return false;
        }

        candidate_health.better_streak = 0;
        if let Some(active_health) = self.health.get_mut(&active_id) {
            active_health.circuit_open_until = Some(now + SLOW_SWITCH_COOLDOWN);
        }
        self.active_id = None;
        self.active_since = None;
        true
    }

    pub fn active_id(&self) -> Option<&str> {
        self.active_id.as_deref()
    }

    pub fn active_endpoint(&self) -> Option<EndpointCandidate> {
        let active_id = self.active_id.as_deref()?;
        self.endpoints
            .iter()
            .find(|endpoint| endpoint.id == active_id)
            .cloned()
    }

    fn is_available(&self, id: &str, now: Instant) -> bool {
        self.health
            .get(id)
            .and_then(|health| health.circuit_open_until)
            .is_none_or(|until| until <= now)
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

    fn endpoint(id: &str, priority: u32) -> EndpointCandidate {
        EndpointCandidate {
            id: id.into(),
            region: id.into(),
            api_url: Some(format!("https://api-{id}.example.com")),
            host: format!("sock-{id}.example.com"),
            port: 443,
            priority,
        }
    }

    #[test]
    fn unreachable_active_endpoint_moves_selection_to_the_alternate() {
        let now = Instant::now();
        let mut pool = EndpointPool::default();
        pool.replace(vec![endpoint("a", 0), endpoint("b", 1)]);
        pool.record_connected("a", now);
        pool.record_unreachable("a", now);
        assert_eq!(
            pool.select(now).map(|endpoint| endpoint.id),
            Some("b".into())
        );
    }

    #[test]
    fn endpoint_health_survives_an_unchanged_session_refresh() {
        let now = Instant::now();
        let mut pool = EndpointPool::default();
        let endpoints = vec![endpoint("a", 0), endpoint("b", 1)];
        pool.replace(endpoints.clone());
        pool.record_unreachable("a", now);
        pool.replace(endpoints);
        assert_eq!(
            pool.select(now).map(|endpoint| endpoint.id),
            Some("b".into())
        );
    }

    #[test]
    fn slow_switch_requires_dwell_hysteresis_and_three_samples() {
        let now = Instant::now();
        let mut pool = EndpointPool::default();
        pool.replace(vec![endpoint("a", 0), endpoint("b", 1)]);
        pool.record_connected("a", now);
        pool.record_active_rtt(Duration::from_millis(800));
        let after_dwell = now + SLOW_SWITCH_COOLDOWN;
        assert!(!pool.record_probe("b", Duration::from_millis(100), after_dwell));
        assert!(!pool.record_probe("b", Duration::from_millis(100), after_dwell));
        assert!(pool.record_probe("b", Duration::from_millis(100), after_dwell));
        assert_eq!(pool.active_id(), None);
        assert_eq!(
            pool.select(after_dwell).map(|endpoint| endpoint.id),
            Some("b".into())
        );
    }

    #[test]
    fn active_slow_endpoint_requests_backend_only_after_dwell_and_three_samples() {
        let now = Instant::now();
        let mut pool = EndpointPool::default();
        pool.replace(vec![endpoint("a", 0)]);
        pool.record_connected("a", now);
        assert!(!pool.record_active_probe(Duration::from_millis(800), now));
        let after_dwell = now + SLOW_SWITCH_COOLDOWN;
        assert!(!pool.record_active_probe(Duration::from_millis(800), after_dwell));
        assert!(!pool.record_active_probe(Duration::from_millis(800), after_dwell));
        assert!(pool.record_active_probe(Duration::from_millis(800), after_dwell));
        assert!(!pool.record_active_probe(Duration::from_millis(800), after_dwell));
        assert!(!pool.record_active_probe(
            Duration::from_millis(800),
            after_dwell + SLOW_SWITCH_COOLDOWN
        ));
        assert_eq!(pool.active_id(), Some("a"));
    }

    #[test]
    fn all_failed_endpoints_expose_the_earliest_retry_delay() {
        let now = Instant::now();
        let mut pool = EndpointPool::default();
        pool.replace(vec![endpoint("a", 0), endpoint("b", 1)]);
        pool.record_unreachable("a", now);
        pool.record_unreachable("b", now);
        assert!(!pool.has_available(now));
        assert_eq!(pool.next_available_in(now), Some(FAILURE_COOLDOWN));
    }

    #[test]
    fn backend_confirmation_allows_an_immediate_retry_after_long_cooldown() {
        let now = Instant::now();
        let mut pool = EndpointPool::default();
        pool.replace(vec![endpoint("a", 0)]);
        for _ in 0..8 {
            pool.record_unreachable("a", now);
        }
        assert_eq!(pool.next_available_in(now), Some(FAILURE_COOLDOWN_CAP));

        pool.retry_now("a");

        assert_eq!(
            pool.select(now).map(|endpoint| endpoint.id),
            Some("a".into())
        );
    }

    #[test]
    fn quality_probe_measures_the_active_endpoint_before_alternates() {
        let now = Instant::now();
        let mut pool = EndpointPool::default();
        pool.replace(vec![endpoint("a", 0), endpoint("b", 1)]);
        pool.record_connected("b", now);
        assert_eq!(
            pool.probe_endpoints(now)
                .into_iter()
                .map(|endpoint| endpoint.id)
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );
    }
}
