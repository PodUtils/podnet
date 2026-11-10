use super::*;

/// The clock of this host against the `Date` of the first API that answers
/// TLS (issue 128).
pub(super) async fn clock(paths: &[Path], timeout: Duration) -> Clock {
    let Some(path) = paths
        .iter()
        .filter(|p| matches!(p.target.tls, Tls::Web))
        .find(|p| p.tls.as_ref().is_some_and(|t| t.verdict == Verdict::Ok))
    else {
        return Clock {
            verdict: Verdict::Unknown,
            skew: None,
            detail: "no API answered TLS".into(),
        };
    };
    let url = format!("https://{}/", path.target.authority());
    let answer =
        crate::fetch::send(crate::fetch::Request::new("HEAD", url.clone()).timeout(timeout)).await;
    let date = match &answer {
        Ok(r) => r.header("date").map(str::to_string),
        Err(_) => None,
    };
    match date.as_deref().and_then(parse_imf_fixdate) {
        Some(remote) => clock_of(now_unix(), remote, &path.target.host),
        None => Clock {
            verdict: Verdict::Unknown,
            skew: None,
            detail: format!("no Date header from {url}"),
        },
    }
}

pub(super) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The verdict of a skew of `host_time - remote` seconds.
pub(super) fn clock_of(host_time: i64, remote: i64, from: &str) -> Clock {
    let skew = host_time - remote;
    let (verdict, detail) = if skew.abs() > CLOCK_LIMIT {
        (
            Verdict::Refused,
            format!("this host is {skew} s off the Date of {from}: TLS certificates and tokens can fail; set the clock (timedatectl, ntpdate)"),
        )
    } else {
        (Verdict::Ok, format!("{skew} s off the Date of {from}"))
    };
    Clock {
        verdict,
        skew: Some(skew),
        detail,
    }
}

/// Unix seconds of an IMF-fixdate (RFC 9110): `Sun, 06 Nov 1994 08:49:37
/// GMT`.
pub fn parse_imf_fixdate(text: &str) -> Option<i64> {
    let mut parts = text.split_whitespace();
    let _weekday = parts.next()?;
    let day: i64 = parts.next()?.parse().ok()?;
    let month_name = parts.next()?;
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| *m == month_name)? as i64
        + 1;
    let year: i64 = parts.next()?.parse().ok()?;
    let mut hms = parts.next()?.split(':').map(|x| x.parse::<i64>().ok());
    let (h, m, s) = (hms.next()??, hms.next()??, hms.next()??);
    if parts.next()? != "GMT" {
        return None;
    }
    // Days from the civil date (H. Hinnant).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + m * 60 + s)
}
