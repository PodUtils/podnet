//! T-120 on the wire: packets cross a WAN between network namespaces. A
//! client namespace sends through the router namespace (this one, in
//! `unshare -rn`), and a far namespace holds the target 203.0.113.9:80
//! behind two WANs (veth pairs with the MTU 1280).
//!
//! ```text
//! client 10.9.0.2 -- t120c 10.9.0.1  [router]  t120a 192.0.2.2   -- t120fa 192.0.2.1   [far, lo 203.0.113.9]
//!                                              t120b 198.51.100.2 -- t120fb 198.51.100.1
//! ```

use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::time::Duration;

use super::netkit::{self, addr, capture, del_addr, mtu, syns, up, veth, Ns};
use super::tests::{can_make_links, rerun_in_namespace, section, wan, INNER};
use super::{apply, ifindex, read_default, remove, Plan};
use crate::network::{Health, Thresholds};

const TARGET: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 9);
const A: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 2);
const B: Ipv4Addr = Ipv4Addr::new(198, 51, 100, 2);
const GA: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 1);
const GB: Ipv4Addr = Ipv4Addr::new(198, 51, 100, 1);
const CLIENT: Ipv4Addr = Ipv4Addr::new(10, 9, 0, 2);
const WAN_MTU: u32 = 1280;

/// The far namespace with the two WANs, and a listener on the target that
/// writes the address of each peer and closes.
fn far_side() -> Ns {
    let far = Ns::new();
    veth("t120a", "t120fa", &far);
    veth("t120b", "t120fb", &far);
    for (dev, ip) in [("t120a", A), ("t120b", B)] {
        mtu(dev, WAN_MTU);
        addr(dev, ip, 24);
        up(dev);
    }
    let listener = far.run(|| {
        for (dev, ip) in [("t120fa", GA), ("t120fb", GB)] {
            mtu(dev, WAN_MTU);
            addr(dev, ip, 24);
            up(dev);
        }
        addr("lo", TARGET, 32);
        TcpListener::bind((TARGET, 80)).unwrap()
    });
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            let peer = conn.peer_addr().map(|p| p.ip().to_string());
            let _ = conn.write_all(peer.unwrap_or_default().as_bytes());
        }
    });
    far
}

fn plan(masquerade: bool, mss_clamp: bool) -> Plan {
    let mut a = wan("a", "t120a");
    a.gateway = Some(GA.to_string());
    let mut b = wan("b", "t120b");
    b.gateway = Some(GB.to_string());
    let mut k = section(vec![a, b]);
    k.masquerade = Some(masquerade);
    k.mss_clamp = Some(mss_clamp);
    Plan::new(&k).unwrap()
}

/// The far ifindex of each WAN and the address of its router end.
fn wan_sources(far: &Ns) -> Vec<(u32, Ipv4Addr)> {
    far.run(|| {
        vec![
            (ifindex("t120fa").unwrap(), A),
            (ifindex("t120fb").unwrap(), B),
        ]
    })
}

fn masquerade_body() {
    netkit::forward_on();
    let far = far_side();
    let client = Ns::new();
    veth("t120c", "t120cc", &client);
    addr("t120c", Ipv4Addr::new(10, 9, 0, 1), 24);
    up("t120c");
    client.run(|| {
        addr("t120cc", CLIENT, 24);
        up("t120cc");
        netkit::default_via(Ipv4Addr::new(10, 9, 0, 1), "t120cc");
    });
    let sources = wan_sources(&far);
    let cap = far.run(capture);
    let to = SocketAddr::from((TARGET, 80));
    let connect = || {
        client.run(|| {
            let mut s = TcpStream::connect_timeout(&to, Duration::from_secs(2))?;
            s.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut seen = String::new();
            s.read_to_string(&mut seen)?;
            Ok::<String, std::io::Error>(seen)
        })
    };

    // The guards off: the SYN leaves with the address of the client and
    // the MSS of its link (1500 - 40), and no answer comes back.
    let off = plan(false, false);
    apply(&off, None).unwrap();
    let _ = connect();
    let seen = syns(&cap, 80);
    assert!(!seen.is_empty(), "a SYN crossed a WAN");
    for syn in &seen {
        assert_eq!(
            (syn.src, syn.mss),
            (CLIENT, Some(1460)),
            "no guard: {syn:?}"
        );
    }

    // The guards on: the source is the address of the WAN that carried the
    // SYN, the MSS is the route MTU of the WAN (1280 - 40), and the flow
    // works.
    let on = plan(true, true);
    apply(&on, Some(&off)).unwrap();
    let peer = connect().expect("the flow works through the masquerade");
    assert!(peer == A.to_string() || peer == B.to_string(), "{peer}");
    let seen = syns(&cap, 80);
    assert!(!seen.is_empty(), "a SYN crossed a WAN");
    for syn in &seen {
        let wan = sources.iter().find(|(i, _)| *i == syn.ifindex).unwrap().1;
        assert_eq!(syn.src, wan, "masquerade to the address of the WAN");
        assert_eq!(syn.mss, Some(WAN_MTU as u16 - 40), "the MSS clamp");
        assert_eq!(syn.dst, TARGET);
    }
    remove(&on).unwrap();
    eprintln!("ran: the masquerade and the MSS clamp on the wire");
}

/// T-120: the masquerade and the MSS clamp, measured on the wire.
#[test]
fn route_kernel_masquerade_and_mss_clamp_on_the_wire() {
    if std::env::var_os(INNER).is_some() {
        if !can_make_links() {
            eprintln!("skip: the namespace has no CAP_NET_ADMIN or no bridge link");
            return;
        }
        masquerade_body();
        return;
    }
    rerun_in_namespace(
        module_path!(),
        "route_kernel_masquerade_and_mss_clamp_on_the_wire",
    );
}

fn probe_body() {
    let far = far_side();
    let plan = plan(true, true);
    apply(&plan, None).unwrap();
    let (ia, ib) = (ifindex("t120a").unwrap(), ifindex("t120b").unwrap());
    let both = Some(vec![(ia, Some(GA), 1), (ib, Some(GB), 1)]);
    let thresholds = Thresholds {
        down_after: 3,
        up_after: 2,
        cooldown: Duration::ZERO,
        interval: Duration::from_secs(1),
        timeout: Duration::from_secs(1),
        rtt_window: 0,
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let target = SocketAddrV4::new(TARGET, 80);
    let mut watch = super::watch::Watch::new(plan.clone(), target, thresholds).unwrap();
    let now = tokio::time::Instant::now;
    let health =
        |w: &super::watch::Watch| w.health().into_iter().map(|(_, h)| h).collect::<Vec<_>>();
    rt.block_on(async {
        assert!(!watch.round(now()).await.unwrap(), "both WANs pass");
        assert_eq!(health(&watch), [Health::Up, Health::Up]);

        // The session of b dies, its link stays up: the far end loses its
        // address, so no answer comes back through b.
        far.run(|| del_addr("t120fb", GB, 24));
        assert!(!watch.round(now()).await.unwrap(), "a miss: cooldown");
        assert!(!watch.round(now()).await.unwrap(), "two misses");
        assert!(watch.round(now()).await.unwrap(), "three misses: b is down");
        assert_eq!(health(&watch), [Health::Up, Health::Down]);
        assert_eq!(watch.routed(), ["a"]);
        assert_eq!(read_default(200).unwrap(), Some(vec![(ia, Some(GA), 1)]));
        for i in 0..8 {
            let s = TcpStream::connect_timeout(&SocketAddr::from(target), Duration::from_secs(2));
            assert!(s.is_ok(), "flow {i} goes through a: {s:?}");
        }

        // b comes back: one pass gives probation (it takes flows again).
        far.run(|| addr("t120fb", GB, 24));
        assert!(watch.round(now()).await.unwrap(), "b is back in the route");
        assert_eq!(health(&watch), [Health::Up, Health::Probation]);
        assert_eq!(read_default(200).unwrap(), both);
        assert!(!watch.round(now()).await.unwrap());
        assert_eq!(health(&watch), [Health::Up, Health::Up]);

        // Each WAN fails: an outage of the host takes no WAN down.
        far.run(|| {
            del_addr("t120fa", GA, 24);
            del_addr("t120fb", GB, 24);
        });
        for _ in 0..4 {
            assert!(!watch.round(now()).await.unwrap(), "no change at an outage");
        }
        assert_eq!(health(&watch), [Health::Cooldown, Health::Cooldown]);
        assert_eq!(read_default(200).unwrap(), both);
    });
    remove(&plan).unwrap();
    eprintln!("ran: the probe of the WANs on the wire");
}

/// T-120, part 4: the probe of each WAN through its own table takes a WAN
/// with a dead session (its link up) out of the aggregate route.
#[test]
fn route_kernel_probe_takes_a_dead_wan_out_of_the_route() {
    if std::env::var_os(INNER).is_some() {
        if !can_make_links() {
            eprintln!("skip: the namespace has no CAP_NET_ADMIN or no bridge link");
            return;
        }
        probe_body();
        return;
    }
    rerun_in_namespace(
        module_path!(),
        "route_kernel_probe_takes_a_dead_wan_out_of_the_route",
    );
}
