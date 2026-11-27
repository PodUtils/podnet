//! `podnet status`: one line of JSON about this host, for a program to read
//! before anything else. It opens no connection, asks no DNS, starts no
//! process and writes nothing. It never shows a credential: the proxy is
//! shown as its variable, host, port and kind, and the WARP identity as its
//! path and device id.

use serde_json::{json, Value};

// The variables of the proxy, in the order that podnet reads them.
use crate::egress::PROXY_VARIABLES;

/// The proxy that podnet uses, by the choice of the dial: `--no-proxy`,
/// `--proxy` (or `PODNET_PROXY`), else the first proxy variable that is set.
fn proxy() -> Value {
    use crate::egress::{Carrier, Proxy, ProxyChoice};
    let no_proxy = ["NO_PROXY", "no_proxy"]
        .iter()
        .any(|k| std::env::var(k).is_ok_and(|v| !v.trim().is_empty()));
    let describe = |p: &Proxy, source: &str| {
        json!({
            "source": source,
            "address": match p.carrier() {
                Carrier::Tcp => format!("{}:{}", p.address().0, p.address().1),
                _ => p.label(),
            },
            "kind": p.kind_text(),
            "credentials": p.has_credentials(),
            "no_proxy": no_proxy,
        })
    };
    match crate::egress::choice() {
        ProxyChoice::Direct => return json!({ "source": "--no-proxy", "kind": "direct" }),
        ProxyChoice::Via(p) => return describe(&p, "--proxy"),
        ProxyChoice::FromEnvironment => {}
    }
    for key in PROXY_VARIABLES {
        let Ok(value) = std::env::var(key) else {
            continue;
        };
        if value.trim().is_empty() {
            continue;
        }
        return match crate::egress::parse(&value) {
            Ok(p) => {
                let mut doc = describe(&p, key);
                // `variable` repeats `source`.
                doc["variable"] = json!(key);
                doc
            }
            Err(err) => json!({ "variable": key, "source": key, "error": err.to_string() }),
        };
    }
    Value::Null
}

fn edge_override() -> Value {
    match std::env::var("PODNET_EDGE") {
        Ok(list) if !list.trim().is_empty() => {
            let items: Vec<&str> = list
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            let bad: Vec<&&str> = items
                .iter()
                .filter(|i| i.parse::<std::net::SocketAddr>().is_err())
                .collect();
            // Issue 252: tunnel and doctor exit 78 on it; status is a
            // report, so it warns and exits 0.
            for item in &bad {
                eprintln!("podnet: warning: PODNET_EDGE entry {item:?} is not IP:PORT; tunnel and doctor refuse it");
            }
            json!({ "addresses": items, "valid": bad.is_empty() })
        }
        _ => Value::Null,
    }
}

fn warp_identity() -> Value {
    // A read: the saved file where it exists, with no directory made
    // (podssh; before, status made the state directory).
    let Some(candidates) = crate::state::candidates(&|k| std::env::var_os(k)) else {
        return json!({ "path": null, "reason": "PODNET_STATE_DIR=none" });
    };
    let path = match crate::state::find_existing("warp.json") {
        Ok(Some(path)) => path,
        // No saved file: the path of the first candidate, where a
        // registration would write it.
        Ok(None) => {
            let path = candidates.first().map(|(_, dir)| dir.join("warp.json"));
            return json!({ "path": path, "exists": false });
        }
        Err(err) => return json!({ "path": null, "error": format!("{err:#}") }),
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return json!({ "path": path, "exists": path.exists() });
    };
    let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    json!({
        "path": path,
        "exists": true,
        "device": parsed["device"]["id"],
        "tunnel_type": parsed["device"]["tunnel_type"],
    })
}

fn tty(fd: i32) -> bool {
    unsafe { libc::isatty(fd) == 1 }
}

/// The version line of podnet, as `podnet --version` gives it.
pub const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("PODNET_BUILD_COMMIT"),
    ")"
);

/// `doc` with `schema` (1, unless it has one) and `podnet` (the version)
/// when it is an object (E3): each JSON document of podnet has the two
/// keys. A `ready` event also gets the keys of the endpoint (`url`,
/// `listen`, `listens`; GitHub #641, T-451).
pub fn stamped(mut doc: Value) -> Value {
    if doc["event"] == "ready" {
        doc = crate::instances::envelope(doc);
    }
    if let Some(map) = doc.as_object_mut() {
        map.entry("schema").or_insert(json!(1));
        map.insert("podnet".into(), json!(VERSION));
    }
    doc
}

/// The JSON text `text` (a document of `pod-manual`), stamped as
/// [`stamped`] does, in the pretty form with a newline. A text that is
/// not JSON stays as it is.
pub fn stamped_text(text: &str) -> String {
    match serde_json::from_str::<Value>(text) {
        Ok(doc) => match serde_json::to_string_pretty(&stamped(doc)) {
            Ok(pretty) => pretty + "\n",
            Err(_) => text.to_string(),
        },
        Err(_) => text.to_string(),
    }
}

/// The seven dependency classes counted by `podnet status` (T-368).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DependencyClass {
    RelayHost,
    RelayDomain,
    CertificateAuthority,
    CarrierProtocol,
    CarrierDestination,
    DnsResolver,
    ThirdPartyService,
}

/// Deduplicated dependency counts across the seven tracked classes.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct DependencyCounts {
    pub relay_host: usize,
    pub relay_domain: usize,
    pub certificate_authority: usize,
    pub carrier_protocol: usize,
    pub carrier_destination: usize,
    pub dns_resolver: usize,
    pub third_party_service: usize,
    pub total: usize,
}

/// Collects and deduplicates dependencies across the seven classes.
#[derive(Clone, Debug, Default)]
pub struct DependencyCollector {
    relay_host: std::collections::BTreeSet<String>,
    relay_domain: std::collections::BTreeSet<String>,
    certificate_authority: std::collections::BTreeSet<String>,
    carrier_protocol: std::collections::BTreeSet<String>,
    carrier_destination: std::collections::BTreeSet<String>,
    dns_resolver: std::collections::BTreeSet<String>,
    third_party_service: std::collections::BTreeSet<String>,
}

impl DependencyCollector {
    /// Create an empty collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one dependency value in `class`, normalizing case and trimming whitespace.
    pub fn add(&mut self, class: DependencyClass, value: impl AsRef<str>) {
        let normalized = value
            .as_ref()
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if normalized.is_empty() {
            return;
        }
        let target = match class {
            DependencyClass::RelayHost => &mut self.relay_host,
            DependencyClass::RelayDomain => &mut self.relay_domain,
            DependencyClass::CertificateAuthority => &mut self.certificate_authority,
            DependencyClass::CarrierProtocol => &mut self.carrier_protocol,
            DependencyClass::CarrierDestination => &mut self.carrier_destination,
            DependencyClass::DnsResolver => &mut self.dns_resolver,
            DependencyClass::ThirdPartyService => &mut self.third_party_service,
        };
        target.insert(normalized);
    }

    /// Record a road endpoint URL (`scheme://host[:port]/...`), populating protocol,
    /// destination, relay host, and deduplicated relay domain.
    pub fn add_road_url(&mut self, url: &str) {
        let trimmed = url.trim();
        let (scheme, rest) = match trimmed.split_once("://") {
            Some((s, r)) => (Some(s), r),
            None => (None, trimmed),
        };
        if let Some(proto) = scheme {
            self.add(DependencyClass::CarrierProtocol, proto);
        }
        let authority = rest.split(['/', '?']).next().unwrap_or("").trim();
        let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        if authority.is_empty() {
            return;
        }
        self.add(DependencyClass::CarrierDestination, authority);
        let host = extract_host(authority);
        if !host.is_empty() {
            self.add(DependencyClass::RelayHost, &host);
            if let Some(domain) = extract_domain(&host) {
                self.add(DependencyClass::RelayDomain, domain);
            }
        }
    }

    /// Compute deduplicated counts across all seven classes.
    pub fn counts(&self) -> DependencyCounts {
        let relay_host = self.relay_host.len();
        let relay_domain = self.relay_domain.len();
        let certificate_authority = self.certificate_authority.len();
        let carrier_protocol = self.carrier_protocol.len();
        let carrier_destination = self.carrier_destination.len();
        let dns_resolver = self.dns_resolver.len();
        let third_party_service = self.third_party_service.len();
        let total = relay_host
            + relay_domain
            + certificate_authority
            + carrier_protocol
            + carrier_destination
            + dns_resolver
            + third_party_service;
        DependencyCounts {
            relay_host,
            relay_domain,
            certificate_authority,
            carrier_protocol,
            carrier_destination,
            dns_resolver,
            third_party_service,
            total,
        }
    }

    /// Build a [`DependencySummary`] combining collected dependencies with third-party road checks.
    pub fn summarize(
        &self,
        roads: &[crate::roads::third_party::ThirdPartyRoad],
        today: crate::roads::third_party::VerifiedDate,
    ) -> DependencySummary {
        let mut collector = self.clone();
        for road in roads {
            if road.enabled {
                collector.add(DependencyClass::ThirdPartyService, &road.name);
                if let Some(ep) = &road.endpoint {
                    collector.add_road_url(ep);
                }
            }
        }
        let oldest_verified = roads
            .iter()
            .map(|r| r.last_verified)
            .min()
            .map(|d| d.to_string());
        let stale_roads: Vec<String> = roads
            .iter()
            .filter(|r| r.evaluate_state(today).is_stale())
            .map(|r| format!("{} (verified {})", r.name, r.last_verified))
            .collect();
        let stale = !stale_roads.is_empty();
        DependencySummary {
            counts: collector.counts(),
            oldest_verified,
            stale,
            stale_roads,
        }
    }
}

/// Dependency counts and third-party freshness status for `podnet status` (T-368).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DependencySummary {
    pub counts: DependencyCounts,
    pub oldest_verified: Option<String>,
    pub stale: bool,
    pub stale_roads: Vec<String>,
}

impl DependencySummary {
    /// True when no registered third-party road is stale.
    pub fn ok(&self) -> bool {
        !self.stale
    }

    /// Return an error when any third-party road check date is stale.
    pub fn check(&self) -> anyhow::Result<()> {
        if self.stale {
            anyhow::bail!(
                "stale third-party road verification (oldest {}): {}",
                self.oldest_verified.as_deref().unwrap_or("none"),
                self.stale_roads.join(", ")
            );
        }
        Ok(())
    }

    /// Human-readable text summary of dependency counts and staleness.
    pub fn text(&self) -> String {
        let oldest = self.oldest_verified.as_deref().unwrap_or("none");
        let state = if self.stale {
            format!("stale ({})", self.stale_roads.join(", "))
        } else {
            "ok".to_string()
        };
        format!(
            "dependencies: total={total} relay_host={rh} relay_domain={rd} certificate_authority={ca} carrier_protocol={cp} carrier_destination={cd} dns_resolver={dr} third_party_service={tp}\noldest_verified: {oldest}\nwatch_state: {state}",
            total = self.counts.total,
            rh = self.counts.relay_host,
            rd = self.counts.relay_domain,
            ca = self.counts.certificate_authority,
            cp = self.counts.carrier_protocol,
            cd = self.counts.carrier_destination,
            dr = self.counts.dns_resolver,
            tp = self.counts.third_party_service,
        )
    }
}

fn extract_host(authority: &str) -> String {
    if let Some(rest) = authority.strip_prefix('[') {
        return rest
            .split_once(']')
            .map_or(authority, |(h, _)| h)
            .to_ascii_lowercase();
    }
    authority
        .rsplit_once(':')
        .map_or(authority, |(h, _)| h)
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn extract_domain(host: &str) -> Option<String> {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    let labels: Vec<&str> = host.split('.').filter(|s| !s.is_empty()).collect();
    if labels.len() >= 2 {
        Some(format!(
            "{}.{}",
            labels[labels.len() - 2],
            labels[labels.len() - 1]
        ))
    } else if labels.len() == 1 {
        Some(labels[0].to_string())
    } else {
        None
    }
}

/// Default dependency summary computed offline from local configuration and built-in metadata.
pub fn default_dependencies() -> DependencySummary {
    let mut collector = DependencyCollector::new();
    if let Ok(roots) = crate::trust::context().roots() {
        for source in &roots.sources {
            collector.add(DependencyClass::CertificateAuthority, source);
        }
    }
    if let Ok(list) = std::env::var("PODNET_EDGE") {
        for entry in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            collector.add_road_url(entry);
        }
    }
    let registry = crate::roads::third_party::default_registry();
    let today = crate::roads::third_party::VerifiedDate::today_utc();
    collector.summarize(registry.roads(), today)
}

/// The status of this host.
pub fn document() -> Value {
    json!({
        "schema": 1,
        "podnet": concat!(env!("CARGO_PKG_VERSION"), " (", env!("PODNET_BUILD_COMMIT"), ")"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "proxy": proxy(),
        "edge_override": edge_override(),
        "warp_identity": warp_identity(),
        "tls_roots": match crate::trust::context().roots() {
            Ok(roots) => json!(roots.sources),
            Err(err) => json!({"error": format!("{err:#}")}),
        },
        "error_program": std::env::var(crate::report::PROGRAM_VARIABLE).ok().filter(|v| !v.is_empty()),
        "stdin_tty": tty(0),
        "stdout_tty": tty(1),
        "instances": instances(),
        "nofile": match crate::rlimit::now() {
            Some(n) => json!({"soft": n.soft, "hard": n.hard, "raised_from": n.raised_from}),
            None => Value::Null,
        },
        "dependencies": serde_json::to_value(default_dependencies()).unwrap_or(Value::Null),
    })
}

/// The detached instances (`--detach`): name, state, pid, the URL or the
/// listen address, and the data path of a WARP session (T-140).
fn instances() -> Value {
    let dir = crate::instances::lookup_dir();
    Value::Array(
        crate::instances::list(&dir)
            .into_iter()
            .map(|e| instance(&e.record))
            .collect(),
    )
}

/// One instance of [`instances`].
fn instance(record: &crate::instances::Record) -> Value {
    let ready = |key: &str| record.ready.as_ref().and_then(|d| d.get(key));
    let at = ready("url")
        .filter(|u| !u.is_null())
        .or_else(|| ready("listen"))
        .cloned();
    let mut doc = json!({
        "name": record.name,
        "state": record.state,
        "pid": record.pid,
        "at": at,
    });
    // T-140: `warp proxy` writes them; other commands have none.
    for key in ["data_path", "last_probe"] {
        if let Some(value) = ready(key) {
            doc[key] = value.clone();
        }
    }
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `println!` statements of `text` (one source file, before its
    /// test module) that print a JSON document with no stamp: a `json!(`
    /// or a `{event}` with no `stamped` in the statement.
    fn bare_json(text: &str) -> Vec<usize> {
        let lines: Vec<&str> = text.lines().collect();
        let end = (0..lines.len())
            .find(|&i| {
                lines[i].starts_with("#[cfg(test)]")
                    && lines.get(i + 1).is_some_and(|l| l.starts_with("mod "))
            })
            .unwrap_or(lines.len());
        (0..end)
            .filter(|&i| lines[i].replace("eprintln!(", "").contains("println!("))
            .filter(|&i| {
                let stop = (i..end).find(|&j| lines[j].contains(");")).unwrap_or(i);
                let statement = lines[i..=stop.min(i + 12)].join(" ");
                (statement.contains("json!(") || statement.contains("{event}"))
                    && !statement.contains("stamped")
            })
            .map(|i| i + 1)
            .collect()
    }

    /// GitHub #716, the class: each JSON document that a command prints has
    /// `schema` and `podnet` (`docs/cli.md`, JSON documents, rule 2). The
    /// plant must be found.
    #[test]
    fn each_printed_json_document_is_stamped() {
        let plant = "fn f() {\n    println!(\n        \"{}\",\n        json!({ \"event\": \"x\" })\n    );\n    println!(\"{}\", stamped(json!({})));\n}\n";
        assert_eq!(bare_json(plant), vec![2]);
        let mut files = Vec::new();
        let mut dirs = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if path.is_dir() {
                    dirs.push(path);
                } else if name.ends_with(".rs")
                    && name != "tests.rs"
                    && !name.ends_with("_tests.rs")
                {
                    files.push(path);
                }
            }
        }
        assert!(files.len() > 100, "the scan read {} files", files.len());
        let faults: Vec<String> = files
            .iter()
            .flat_map(|f| {
                let text = std::fs::read_to_string(f).unwrap();
                bare_json(&text)
                    .into_iter()
                    .map(move |n| format!("{}:{n}", f.display()))
            })
            .collect();
        assert!(
            faults.is_empty(),
            "a JSON document with no stamp (#716): {faults:#?}"
        );
        // GitHub #724: `schema` is a positive integer; a named form is `kind`.
        let named: Vec<String> = files
            .iter()
            .flat_map(|f| {
                let text = std::fs::read_to_string(f).unwrap();
                let end = text.find("#[cfg(test)]\nmod ").unwrap_or(text.len());
                text[..end]
                    .lines()
                    .enumerate()
                    .filter(|(_, l)| l.contains("\"schema\": \""))
                    .map(|(n, _)| format!("{}:{}", f.display(), n + 1))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(
            named.is_empty(),
            "a schema that is not an integer (#724): {named:#?}"
        );
    }

    /// T-140: `podnet status` names the data path of a WARP instance.
    #[test]
    fn status_names_the_data_path_of_an_instance() {
        let mut record = crate::instances::Record {
            name: "warp".into(),
            state: "ready".into(),
            ready: Some(json!({"listen": "unix:/r/warp.sock"})),
            ..Default::default()
        };
        let doc = instance(&record);
        assert_eq!(doc["at"], "unix:/r/warp.sock");
        assert!(doc.get("data_path").is_none(), "{doc}");
        crate::warp::live::instance::amend(
            &mut record,
            crate::warp::live::Report {
                data_path: "tcp-dead",
                last_probe: Some(1000),
            },
        );
        let doc = instance(&record);
        assert_eq!(doc["data_path"], "tcp-dead", "{doc}");
        assert_eq!(doc["last_probe"], 1000, "{doc}");
    }

    #[test]
    fn the_status_has_its_fields_and_no_credentials() {
        let doc = document();
        assert_eq!(doc["schema"], 1);
        for key in [
            "podnet",
            "os",
            "arch",
            "proxy",
            "warp_identity",
            "stdin_tty",
        ] {
            assert!(doc.get(key).is_some(), "{key}");
        }
        let text = doc.to_string();
        for key in PROXY_VARIABLES {
            if let Ok(v) = std::env::var(key) {
                if v.contains('@') {
                    assert!(
                        !text.contains(&v),
                        "the proxy URL with credentials is shown"
                    );
                }
            }
        }
    }
}
