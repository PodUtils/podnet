//! The retry rules of [`super::provision`].

/// The wait of a `Retry-After` value: a number of seconds, or an HTTP date
/// (IMF-fixdate).
pub fn parse_retry_after(value: &str) -> Option<std::time::Duration> {
    let value = value.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(std::time::Duration::from_secs(secs));
    }
    // Sun, 06 Nov 1994 08:49:37 GMT
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() != 6 || parts[5] != "GMT" {
        return None;
    }
    let day: i64 = parts[1].parse().ok()?;
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = MONTHS.iter().position(|m| *m == parts[2])? as i64 + 1;
    let year: i64 = parts[3].parse().ok()?;
    let hms: Vec<i64> = parts[4].split(':').filter_map(|p| p.parse().ok()).collect();
    if hms.len() != 3 {
        return None;
    }
    // Days from 1970-01-01 (the algorithm of H. Hinnant).
    let (y, m) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let at = days * 86_400 + hms[0] * 3600 + hms[1] * 60 + hms[2];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    Some(std::time::Duration::from_secs((at - now).max(0) as u64))
}

/// True for a transport error at the hop `dns`.
pub(super) fn is_dns(err: &anyhow::Error) -> bool {
    crate::hop::of(err) == Some(crate::hop::Hop::Dns)
}

/// The state of the retries of a failure at name resolution.
#[derive(Default)]
pub(super) struct Dns {
    failures: u32,
    step: Option<&'static str>,
}

impl Dns {
    /// True when a retry can help: the first failure is tried once more;
    /// after it, only a change of the step of the resolver chain that
    /// answers for the host of `endpoint` gives a new try.
    pub(super) async fn again(&mut self, endpoint: &str) -> bool {
        let host = endpoint
            .split("://")
            .nth(1)
            .unwrap_or(endpoint)
            .split(['/', ':'])
            .next()
            .unwrap_or("");
        let step = crate::resolve::answering_step(host).await;
        self.decide(step)
    }

    /// The decision of [`Dns::again`] for the step `step` that answers now.
    fn decide(&mut self, step: &'static str) -> bool {
        self.failures += 1;
        let again = self.failures <= 1 || self.step != Some(step);
        self.step = Some(step);
        again
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-170: one retry, then a retry only for a new answering step.
    #[test]
    fn a_dns_failure_is_retried_once() {
        let mut dns = Dns::default();
        assert!(dns.decide("none"));
        assert!(!dns.decide("none"));
        assert!(dns.decide("doh"));
        assert!(!dns.decide("doh"));
    }
}
