//! The query keys of a carrier address (T-306).
//!
//! An address of `link listen`, `link dial` and `link connect` can end in
//! a query: `ADDRESS?key=value&key=value`. This module splits the address
//! from the query and checks the query before any socket opens.
//!
//! * The allowed keys are in [`KEYS`]. A key is given once. An unknown key
//!   exits 64 and the message lists the allowed keys.
//! * `password`, `key`, `token` and `identity` are refused, with or
//!   without a value. A secret does not belong in an address.
//! * A message never holds a value. It holds the name of an allowed key,
//!   or the name of a refused key, and nothing that the caller typed
//!   otherwise.
//! * The URL has 1024 bytes at most and 16 keys at most. A value is
//!   ASCII, with one level of percent-encoding: a decoded value that
//!   holds `%` is refused.
//!
//! A key that has no effect yet is parsed and checked, and then
//! [`Query::require_wired`] exits 64 with the entry that adds it.
//! [`crate::carriers::check_query`] checks the keys that depend on the
//! carrier.

use anyhow::{anyhow, Result};

use crate::exit::{self, Fault};

/// The most bytes of an address with a query.
pub const MAX_URL: usize = 1024;

/// The most keys of a query.
pub const MAX_KEYS: usize = 16;

/// The allowed keys, with the entry that gives each one its effect.
pub const KEYS: [(&str, &str); 11] = [
    ("carrier", "T-264"),
    ("tls", "T-269"),
    ("retry", "T-263"),
    ("lb", "T-288"),
    ("compress", "T-299"),
    ("pad", "T-300"),
    ("mtu", "T-276"),
    ("maxhops", "T-287"),
    ("poll", "T-271"),
    ("window", "T-262"),
    ("name", "T-308"),
];

/// The keys that a query never holds, with or without a value.
pub const REFUSED: [&str; 4] = ["password", "key", "token", "identity"];

/// The policies of `lb` (T-288).
pub const LB: [&str; 4] = ["weighted", "rr", "minrtt", "leastload"];

/// The values of `compress` (T-299).
pub const COMPRESS: [&str; 2] = ["none", "snappy"];

/// The range of `window`, in bytes: the range of `pod_link::Settings`.
pub const WINDOW: (u32, u32) = (16 * 1024, 8 * 1024 * 1024);

/// The range of `mtu`, in bytes.
pub const MTU: (usize, usize) = (64, 65535);

/// The range of `retry`, in tries.
pub const RETRY: (u8, u8) = (0, 16);

/// The range of `maxhops` (T-287).
pub const MAXHOPS: (u8, u8) = (1, 8);

/// The range of `poll`, in seconds (T-271).
pub const POLL: (u8, u8) = (1, 60);

/// A checked query. A field is `None` when the key is not given.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Query {
    pub carrier: Option<String>,
    pub tls: Option<bool>,
    pub retry: Option<u8>,
    pub lb: Option<&'static str>,
    pub compress: Option<&'static str>,
    pub pad: Option<bool>,
    pub mtu: Option<usize>,
    pub maxhops: Option<u8>,
    pub poll: Option<u8>,
    pub window: Option<u32>,
    pub name: Option<String>,
}

fn usage(text: String) -> anyhow::Error {
    exit::fault(Fault::Usage, anyhow!(text))
}

fn allowed_list() -> String {
    KEYS.iter().map(|k| k.0).collect::<Vec<_>>().join(", ")
}

/// Split `address` at its first `?` and check the query. An address with
/// no `?` gives an empty query. The error never holds a value.
pub fn split(address: &str) -> Result<(&str, Query)> {
    let Some((base, text)) = address.split_once('?') else {
        return Ok((address, Query::default()));
    };
    if address.len() > MAX_URL {
        return Err(usage(format!(
            "the address with a query has {MAX_URL} bytes at most"
        )));
    }
    Ok((base, parse(text)?))
}

/// Check the text of a query (the part after `?`).
pub fn parse(text: &str) -> Result<Query> {
    let mut query = Query::default();
    if text.is_empty() {
        return Ok(query);
    }
    let pairs: Vec<&str> = text.split('&').collect();
    if pairs.len() > MAX_KEYS {
        return Err(usage(format!("a query has {MAX_KEYS} keys at most")));
    }
    let mut seen: Vec<&str> = Vec::new();
    for pair in pairs {
        let (key, raw) = pair.split_once('=').unwrap_or((pair, ""));
        let lower = key.to_ascii_lowercase();
        if REFUSED.contains(&lower.as_str()) {
            return Err(usage(format!(
                "the query key {lower:?} is refused: a secret does not belong in an address"
            )));
        }
        let Some(&(name, _)) = KEYS.iter().find(|k| k.0 == key) else {
            return Err(usage(format!(
                "a query key is not known: the allowed keys are {}",
                allowed_list()
            )));
        };
        if seen.contains(&name) {
            return Err(usage(format!("the query key {name} is given twice")));
        }
        seen.push(name);
        if !pair.contains('=') || raw.is_empty() {
            return Err(usage(format!("the query key {name} has no value")));
        }
        let value = decode(raw).map_err(|why| usage(format!("the query key {name}: {why}")))?;
        set(&mut query, name, &value)
            .map_err(|why| usage(format!("the query key {name}: {why}")))?;
    }
    Ok(query)
}

/// One level of percent-decoding. The result is printable ASCII with no `%`.
fn decode(raw: &str) -> Result<String, String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let b = bytes[at];
        if b == b'%' {
            let hex = bytes.get(at + 1..at + 3).ok_or("a bad percent escape")?;
            let hex = std::str::from_utf8(hex).map_err(|_| "a bad percent escape")?;
            let v = u8::from_str_radix(hex, 16).map_err(|_| "a bad percent escape")?;
            if v == b'%' {
                return Err("nested percent-encoding".into());
            }
            out.push(v);
            at += 3;
        } else {
            out.push(b);
            at += 1;
        }
    }
    if out.contains(&b'%') {
        return Err("nested percent-encoding".into());
    }
    if !out.iter().all(|b| (0x21..=0x7e).contains(b)) {
        return Err("the value is not printable ASCII".into());
    }
    Ok(String::from_utf8(out).expect("ASCII"))
}

fn number<T: std::str::FromStr + PartialOrd + Copy + std::fmt::Display>(
    value: &str,
    (low, high): (T, T),
) -> Result<T, String> {
    let n: T = value.parse().map_err(|_| "the value is not a number")?;
    if n < low || n > high {
        return Err(format!("the value is not from {low} to {high}"));
    }
    Ok(n)
}

fn one_of(value: &str, words: &[&'static str]) -> Result<&'static str, String> {
    words
        .iter()
        .find(|w| **w == value)
        .copied()
        .ok_or_else(|| format!("the value is not one of {}", words.join(", ")))
}

fn switch(value: &str) -> Result<bool, String> {
    Ok(one_of(value, &["on", "off"])? == "on")
}

/// A name of a carrier or of a link: 1 to `max` characters of `a-z 0-9 -`.
fn word(value: &str, max: usize) -> Result<String, String> {
    let ok = !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(value.to_string())
    } else {
        Err(format!("the value has 1 to {max} characters of a-z 0-9 -"))
    }
}

fn set(q: &mut Query, key: &str, v: &str) -> Result<(), String> {
    match key {
        "carrier" => q.carrier = Some(word(v, 16)?),
        "tls" => q.tls = Some(switch(v)?),
        "retry" => q.retry = Some(number(v, RETRY)?),
        "lb" => q.lb = Some(one_of(v, &LB)?),
        "compress" => q.compress = Some(one_of(v, &COMPRESS)?),
        "pad" => q.pad = Some(switch(v)?),
        "mtu" => q.mtu = Some(number(v, MTU)?),
        "maxhops" => q.maxhops = Some(number(v, MAXHOPS)?),
        "poll" => q.poll = Some(number(v, POLL)?),
        "window" => q.window = Some(number(v, WINDOW)?),
        "name" => q.name = Some(word(v, 32)?),
        _ => unreachable!("the key is allowed"),
    }
    Ok(())
}

impl Query {
    /// The keys that are given, in the order of [`KEYS`].
    pub fn given(&self) -> Vec<&'static str> {
        let on = [
            self.carrier.is_some(),
            self.tls.is_some(),
            self.retry.is_some(),
            self.lb.is_some(),
            self.compress.is_some(),
            self.pad.is_some(),
            self.mtu.is_some(),
            self.maxhops.is_some(),
            self.poll.is_some(),
            self.window.is_some(),
            self.name.is_some(),
        ];
        KEYS.iter()
            .zip(on)
            .filter_map(|(k, on)| on.then_some(k.0))
            .collect()
    }

    /// Exit 64 for a key that has no effect yet, with the entry that adds
    /// it. `carrier` has an effect. The others wait for their entry.
    pub fn require_wired(&self) -> Result<()> {
        for key in self.given() {
            // `pad` and `compress` seal the records of a stream (T-299, T-300):
            // the command checks that --seal is given.
            if matches!(key, "carrier" | "pad" | "compress") {
                continue;
            }
            // T-308 binds the name before the command starts: the flag gives it.
            if key == "name" {
                return Err(usage(
                    "the query key name has no effect: give the name with --name".into(),
                ));
            }
            let entry = KEYS.iter().find(|k| k.0 == key).map_or("", |k| k.1);
            return Err(usage(format!(
                "the query key {key} has no effect yet: {entry} adds it"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(text: &str) -> String {
        format!("{:#}", parse(text).unwrap_err())
    }

    #[test]
    fn each_key_parses() {
        let q = parse("carrier=tcp&tls=on&retry=3&lb=rr&compress=snappy&pad=off&mtu=1200&maxhops=4&poll=25&window=65536&name=a-b1").unwrap();
        assert_eq!(q.carrier.as_deref(), Some("tcp"));
        assert_eq!(q.tls, Some(true));
        assert_eq!(q.retry, Some(3));
        assert_eq!(q.lb, Some("rr"));
        assert_eq!(q.compress, Some("snappy"));
        assert_eq!(q.pad, Some(false));
        assert_eq!(q.mtu, Some(1200));
        assert_eq!(q.maxhops, Some(4));
        assert_eq!(q.poll, Some(25));
        assert_eq!(q.window, Some(65536));
        assert_eq!(q.name.as_deref(), Some("a-b1"));
        assert_eq!(q.given().len(), 11);
    }

    #[test]
    fn no_query_is_empty() {
        assert_eq!(split("/tmp/x").unwrap(), ("/tmp/x", Query::default()));
        assert_eq!(split("/tmp/x?").unwrap().1, Query::default());
        assert_eq!(split("/tmp/x?retry=2").unwrap().0, "/tmp/x");
    }

    #[test]
    fn a_repeat_is_refused() {
        assert!(err("retry=1&retry=2").contains("given twice"));
    }

    #[test]
    fn an_unknown_key_lists_the_allowed_keys_and_not_its_name() {
        let m = err("hunter2=1");
        assert!(
            m.contains("carrier, tls, retry, lb, compress, pad, mtu, maxhops, poll, window, name")
        );
        assert!(!m.contains("hunter2"));
    }

    #[test]
    fn a_secret_key_is_refused_with_no_value_too() {
        for k in ["password", "key", "token", "identity", "PassWord"] {
            for q in [format!("{k}=s3cretvalue"), k.to_string(), format!("{k}=")] {
                let m = err(&q);
                assert!(m.contains("refused"), "{m}");
                assert!(!m.contains("s3cretvalue"), "{m}");
            }
        }
    }

    #[test]
    fn no_message_holds_a_value() {
        for q in [
            "retry=zzsecretzz",
            "retry=99",
            "mtu=zzsecretzz",
            "lb=zzsecretzz",
            "name=ZZsecretZZ",
            "carrier=zzsecretzz!",
            "tls=zzsecretzz",
            "window=1zzsecretzz",
            "poll=%zzsecretzz",
            "name=%25zzsecretzz",
            "name=caf%C3%A9zzsecretzz",
            "name=\u{e9}zzsecretzz",
        ] {
            let m = err(q);
            assert!(
                !m.contains("zzsecret") && !m.contains("ZZsecret"),
                "{q}: {m}"
            );
        }
    }

    #[test]
    fn bad_values_are_refused() {
        assert!(err("retry=17").contains("not from 0 to 16"));
        assert!(err("retry=x").contains("not a number"));
        assert!(err("maxhops=0").contains("not from 1 to 8"));
        assert!(err("window=1").contains("not from"));
        assert!(err("lb=x").contains("weighted, rr, minrtt, leastload"));
        assert!(err("name=").contains("no value"));
        assert!(err("name").contains("no value"));
        assert!(err("name=A").contains("1 to 32"));
        assert!(err(&format!("name={}", "a".repeat(33))).contains("1 to 32"));
        assert!(parse(&format!("name={}", "a".repeat(32))).is_ok());
        assert!(err("retry=1&&tls=on").contains("not known"));
    }

    #[test]
    fn the_encoding_has_one_level() {
        assert_eq!(parse("name=a%2Db").unwrap().name.as_deref(), Some("a-b"));
        assert!(err("name=a%252Db").contains("nested"));
        assert!(err("name=a%25").contains("nested"));
        assert!(err("name=a%2").contains("percent"));
        assert!(err("name=a%zz").contains("percent"));
        assert!(err("name=a%C3%A9").contains("ASCII"));
        assert!(err("name=a%00").contains("ASCII"));
    }

    #[test]
    fn the_limits_hold() {
        let many: Vec<String> = (0..17).map(|_| "tls=on".to_string()).collect();
        assert!(err(&many.join("&")).contains("16 keys at most"));
        let long = format!("/p?name={}", "a".repeat(1024));
        assert!(split(&long).unwrap_err().to_string().contains("1024 bytes"));
        let ok = format!("/p?name={}", "a".repeat(30));
        assert!(split(&ok).is_ok());
    }

    #[test]
    fn a_key_with_no_effect_names_its_entry() {
        let q = parse("mtu=1200").unwrap();
        let m = format!("{:#}", q.require_wired().unwrap_err());
        assert!(m.contains("T-276"), "{m}");
        assert!(parse("compress=snappy&pad=on")
            .unwrap()
            .require_wired()
            .is_ok());
        let q = parse("name=x").unwrap();
        assert!(format!("{:#}", q.require_wired().unwrap_err()).contains("--name"));
        assert!(parse("carrier=unix").unwrap().require_wired().is_ok());
        assert!(Query::default().require_wired().is_ok());
    }
}
