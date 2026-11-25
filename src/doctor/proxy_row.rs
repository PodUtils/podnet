//! The row `proxy` of `podnet doctor`: the proxy of each target.

use serde_json::{json, Value};

use super::Target;

pub(super) fn proxy_report(targets: &[Target]) -> Value {
    let mut used = Vec::new();
    for t in targets {
        if let Some(p) = crate::egress::proxy_for(&t.host, t.port) {
            used.push((t.id, p));
        }
    }
    let direct: Vec<&str> = targets
        .iter()
        .filter(|t| crate::egress::proxy_for(&t.host, t.port).is_none())
        .map(|t| t.id)
        .collect();
    let first = used.first().map(|(_, p)| p.clone());
    let text = match &first {
        None => "none: each connection is direct".to_string(),
        Some(p) => {
            let mut text = format!(
                "{} (credentials: {})",
                p.label(),
                if p.has_credentials() {
                    "present"
                } else {
                    "none"
                }
            );
            if !direct.is_empty() {
                text.push_str(&format!("; direct by NO_PROXY: {}", direct.join(", ")));
            }
            text
        }
    };
    json!({
        "url": first.as_ref().map(|p| p.label()),
        "credentials": first.as_ref().map(|p| p.has_credentials()),
        "direct": direct,
        "text": text,
    })
}
