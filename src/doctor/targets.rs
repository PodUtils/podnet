//! The targets of the doctor.

use super::{masque_address, masque_address_v6, Target, Tls};

/// The targets: the APIs, the edge by name and by address, and MASQUE.
pub fn targets(relay: Option<&str>) -> Vec<Target> {
    let edge = crate::udp::probe_target();
    let quick_api = crate::services::endpoint(crate::services::QUICK, "api");
    let warp_api = crate::services::endpoint(crate::services::WARP, "api");
    vec![
        Target {
            id: "api",
            host: quick_api.host().into(),
            port: quick_api.port(),
            tls: Tls::Web,
        },
        Target {
            id: "warp-api",
            host: warp_api.host().into(),
            port: warp_api.port(),
            tls: Tls::Web,
        },
        Target {
            id: "edge-name",
            host: crate::tunnel::edge_region_name().into(),
            port: 7844,
            tls: Tls::Edge,
        },
        Target {
            id: "edge-address",
            host: edge.ip().to_string(),
            port: edge.port(),
            tls: Tls::Edge,
        },
        Target {
            id: "masque",
            host: masque_address().into(),
            port: crate::warp::register::masque_port(),
            tls: Tls::Masque,
        },
    ]
    .into_iter()
    .chain(
        (crate::resolve::has_default_route(true)
            || crate::egress::proxy_for(masque_address_v6(), crate::warp::register::masque_port())
                .is_some())
        .then(|| Target {
            id: "masque-v6",
            host: masque_address_v6().into(),
            port: crate::warp::register::masque_port(),
            tls: Tls::Masque,
        }),
    )
    // Issue 128: the WebSocket relay is a road of its own (--relay, PODNET_RELAY).
    .chain(relay_target(relay))
    .collect()
}

/// The row of the relay: `relay`, else `PODNET_RELAY`, else the default. The
/// row is the check of the relay network of the registry (T-059): an
/// `https://` relay gets a TLS check, a loopback `http://` relay a plain one.
pub(super) fn relay_target(relay: Option<&str>) -> Option<Target> {
    let spec = match relay {
        Some(url) => format!("relay:{url}"),
        None => "relay".to_string(),
    };
    let network = crate::networks::parse(&spec).ok()?;
    let check = crate::network::Network::checks(&network)
        .into_iter()
        .find(|c| c.name == "relay")?;
    Some(Target {
        id: "relay",
        host: check.target.host(),
        port: check.target.port(),
        tls: if check.tls { Tls::Web } else { Tls::Plain },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_relay_row_follows_the_given_relay() {
        let row = relay_target(Some("https://relay.example")).unwrap();
        assert_eq!(
            (row.id, row.host.as_str(), row.port, row.tls),
            ("relay", "relay.example", 443, Tls::Web)
        );
        let list = relay_target(Some(
            "https://a.relay.example.com,https://b.relay.example.com",
        ))
        .unwrap();
        assert_eq!(list.host, "a.relay.example.com");
        let loopback = relay_target(Some("http://127.0.0.1:8787")).unwrap();
        assert_eq!(
            (loopback.host.as_str(), loopback.port, loopback.tls),
            ("127.0.0.1", 8787, Tls::Plain)
        );
        assert!(relay_target(Some("http://relay.example")).is_none());
    }

    #[test]
    fn the_targets_hold_the_given_relay() {
        let rows = targets(Some("http://127.0.0.1:8787"));
        assert!(rows.iter().any(|t| t.id == "relay" && t.port == 8787));
    }
}
