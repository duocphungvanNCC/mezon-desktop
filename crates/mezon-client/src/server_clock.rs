//! Server-anchored wall clock.
//!
//! Token lifetimes are decided by comparing the JWT `exp` against local time, so a device whose
//! clock is off refreshes on every check or never refreshes at all. Every authenticated HTTP
//! response carries a `Date` header, which costs nothing to read and pins the difference.

use std::sync::atomic::{AtomicI64, Ordering};

static OFFSET_SECS: AtomicI64 = AtomicI64::new(0);
/// Ignore anything past this: a wildly wrong `Date` is more likely a broken intermediary than a
/// device that is a day out.
const MAX_PLAUSIBLE_SKEW_SECS: i64 = 24 * 60 * 60;

fn local_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Seconds since the epoch, corrected by the last `Date` header we saw.
pub fn now_secs() -> u64 {
    let local = local_now_secs() as i64;
    let corrected = local + OFFSET_SECS.load(Ordering::Relaxed);
    u64::try_from(corrected).unwrap_or(0)
}

pub fn observed_skew_secs() -> i64 {
    OFFSET_SECS.load(Ordering::Relaxed)
}

/// Record the skew implied by an HTTP `Date` header (RFC 7231 IMF-fixdate).
pub fn observe_http_date(date_header: &str) {
    let Some(server_secs) = parse_imf_fixdate(date_header) else {
        return;
    };
    let offset = server_secs as i64 - local_now_secs() as i64;
    if offset.abs() > MAX_PLAUSIBLE_SKEW_SECS {
        return;
    }
    let previous = OFFSET_SECS.swap(offset, Ordering::Relaxed);
    if (previous - offset).abs() >= 30 {
        tracing::info!("Clock skew against the server is {offset}s");
    }
}

fn parse_imf_fixdate(value: &str) -> Option<u64> {
    // "Wed, 06 Aug 2026 13:05:29 GMT"
    let rest = value.split_once(", ")?.1;
    let mut parts = rest.split_whitespace();
    let day: u32 = parts.next()?.parse().ok()?;
    let month = match parts.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i64 = parts.next()?.parse().ok()?;
    let mut hms = parts.next()?.split(':');
    let hour: i64 = hms.next()?.parse().ok()?;
    let minute: i64 = hms.next()?.parse().ok()?;
    let second: i64 = hms.next()?.parse().ok()?;
    let days = days_from_civil(year, month, day as i64);
    u64::try_from(days * 86_400 + hour * 3_600 + minute * 60 + second).ok()
}

/// Howard Hinnant's civil-date algorithm — days since 1970-01-01.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_imf_fixdate() {
        assert_eq!(
            parse_imf_fixdate("Wed, 06 Aug 2026 13:05:29 GMT"),
            Some(1_786_021_529)
        );
        assert_eq!(parse_imf_fixdate("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
    }

    #[test]
    fn rejects_anything_it_cannot_read() {
        assert_eq!(parse_imf_fixdate("not a date"), None);
        assert_eq!(parse_imf_fixdate("Wed, 06 Xxx 2026 13:05:29 GMT"), None);
    }

    #[test]
    fn an_implausible_date_is_ignored() {
        let before = observed_skew_secs();
        observe_http_date("Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(observed_skew_secs(), before);
    }
}
