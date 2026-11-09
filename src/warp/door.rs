//! The order of the front door addresses of the MASQUE session.

use std::net::SocketAddr;

/// The order of the front door addresses: the one that worked last first,
/// then the others; IPv6 only with an IPv6 route (or a proxy or a relay,
/// which reach it for podnet). With no door that worked and no IPv4 route
/// (`v4_route` false), the IPv6 doors go first: an IPv4 door can only fail
/// with `Network unreachable` (T-169).
pub(super) fn endpoint_order(
    all: Vec<SocketAddr>,
    won: Option<SocketAddr>,
    bad: Option<SocketAddr>,
    ipv6: bool,
    v4_route: bool,
) -> Vec<SocketAddr> {
    let mut out: Vec<SocketAddr> = all.into_iter().filter(|a| a.is_ipv4() || ipv6).collect();
    let won = won.filter(|w| Some(*w) != bad);
    if let Some(won) = won {
        if let Some(i) = out.iter().position(|a| *a == won) {
            out.swap(0, i);
        }
    } else if !v4_route {
        out.sort_by_key(|a| a.is_ipv4());
    }
    // Issue 133: a door that carried no packet goes last.
    if let Some(bad) = bad {
        if let Some(i) = out.iter().position(|a| *a == bad) {
            let door = out.remove(i);
            out.push(door);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Issue 124: the address that worked goes first; IPv6 only with a
    // route.
    #[test]
    fn the_order_of_the_front_door_addresses() {
        let v4: SocketAddr = "162.159.198.2:443".parse().unwrap();
        let v6: SocketAddr = "[2606:4700:103::2]:443".parse().unwrap();
        assert_eq!(
            endpoint_order(vec![v4, v6], None, None, true, true),
            [v4, v6]
        );
        assert_eq!(
            endpoint_order(vec![v4, v6], Some(v6), None, true, true),
            [v6, v4]
        );
        assert_eq!(endpoint_order(vec![v4, v6], None, None, false, true), [v4]);
        assert_eq!(
            endpoint_order(vec![v4, v6], Some(v6), None, false, true),
            [v4]
        );
        // Issue 133: a door that carried no packet goes last, also when it
        // won the handshake.
        assert_eq!(
            endpoint_order(vec![v4, v6], Some(v4), Some(v4), true, true),
            [v6, v4]
        );
        assert_eq!(
            endpoint_order(vec![v4, v6], None, Some(v4), true, true),
            [v6, v4]
        );
    }

    // T-169: with no IPv4 route, the IPv6 door goes first; a door that won
    // keeps its place.
    #[test]
    fn endpoint_order_puts_v6_first_without_a_v4_route() {
        let v4: SocketAddr = "162.159.198.2:443".parse().unwrap();
        let v6: SocketAddr = "[2606:4700:103::2]:443".parse().unwrap();
        assert_eq!(
            endpoint_order(vec![v4, v6], None, None, true, false),
            [v6, v4]
        );
        assert_eq!(
            endpoint_order(vec![v4, v6], Some(v4), None, true, false),
            [v4, v6]
        );
        assert_eq!(
            endpoint_order(vec![v4, v6], None, None, true, true),
            [v4, v6]
        );
    }
}
