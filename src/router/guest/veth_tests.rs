//! T-248: a Linux guest kernel on a real wire. The guest is the kernel of a
//! network namespace: on a veth pair (`veth:NAME`), and on a TUN device
//! (`tun:NAME`). The router runs over fake ways (`kit::Fake`), so no flow
//! leaves the host. Each test runs in `unshare -rn` and skips with a line
//! when the host refuses the namespace, a link or the TUN.

use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use super::kit::Fake;
use super::wire;
use super::Opened;
use crate::router::kernel::netkit::{self, addr, up, veth, Ns};
use crate::router::kernel::tests::{can_make_links, rerun_in_namespace, INNER};
use crate::router::{parse, Router};

const GUEST: Ipv4Addr = Ipv4Addr::new(10, 66, 0, 4);
const GATEWAY: Ipv4Addr = Ipv4Addr::new(10, 66, 0, 1);
const FAR: [u8; 4] = [93, 184, 216, 34];

fn file(wire: &str) -> String {
    format!(
        "networks:\n  a: direct\n  b: direct\nguests:\n  - wire: {wire}\n    net: 10.66.0.0/24\n    via: a\n"
    )
}

/// A router of the guest wire `wire` over the fake ways `a` and `b`, on a
/// runtime of its own threads.
fn router(rt: &tokio::runtime::Runtime, wire: &str, a: &Arc<Fake>) -> Router {
    let config = parse(&file(wire)).unwrap();
    let mut ways = std::collections::HashMap::new();
    ways.insert("a".to_string(), a.way());
    ways.insert("b".to_string(), Fake::new("b").way());
    let _enter = rt.enter();
    Router::build(&config, ways).unwrap()
}

/// The flows of a Linux guest: a TCP flow that carries 1 MiB each way
/// through the echo of way `a`, and UDP datagrams of 4 and 3000 bytes (the
/// guest kernel cuts the large one into fragments).
fn guest_flows() {
    let to = SocketAddr::from((FAR, 80));
    let mut s = TcpStream::connect_timeout(&to, Duration::from_secs(5)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut label = [0u8; 2];
    s.read_exact(&mut label).unwrap();
    assert_eq!(&label, b"a|", "the flow went through way a");
    let data: Vec<u8> = (0..1 << 20).map(|i: u32| (i % 251) as u8).collect();
    let mut writer = s.try_clone().unwrap();
    let sent = data.clone();
    let back = std::thread::scope(|scope| {
        scope.spawn(move || writer.write_all(&sent).unwrap());
        let mut back = vec![0u8; data.len()];
        s.read_exact(&mut back).map(|_| back)
    });
    assert!(back.unwrap() == data, "1 MiB came back unchanged");
    s.shutdown(std::net::Shutdown::Write).unwrap();
    let mut rest = Vec::new();
    s.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty(), "the flow ends both ways");

    let udp = UdpSocket::bind((GUEST, 0)).unwrap();
    udp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    for size in [4usize, 3000] {
        let datagram = vec![7u8; size];
        udp.send_to(&datagram, SocketAddr::from((FAR, 53))).unwrap();
        let mut buf = vec![0u8; 4096];
        let (n, from) = udp.recv_from(&mut buf).expect("the echo of the datagram");
        assert_eq!((n, from), (size, SocketAddr::from((FAR, 53))));
    }
}

fn veth_body() {
    let guest = Ns::new();
    veth("t248r", "t248g", &guest);
    guest.run(|| {
        addr("t248g", GUEST, 24);
        up("t248g");
        netkit::default_via(GATEWAY, "t248g");
    });
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let a = Fake::new("a");
    let router = router(&rt, "veth:t248r", &a);
    let served = router.guests[0].clone();
    let opened = served.open(false).unwrap();
    assert!(matches!(opened, Opened::Frames(_)));
    let task = rt.spawn(served.serve(opened));
    guest.run(guest_flows);
    assert_eq!(a.dialed(), ["93.184.216.34:80"]);
    let stats = router.guests[0].stats.json();
    assert_eq!(stats["udp_up"], 2, "{stats}");
    assert_eq!(stats["udp_down"], 2, "{stats}");
    task.abort();
    eprintln!("ran: a Linux guest on a veth wire");
}

/// T-248: a veth wire on an interface of the router, with a Linux guest
/// kernel at the other end (its offloads on).
#[test]
fn guest_wire_veth_carries_tcp_and_udp_of_a_linux_guest() {
    if std::env::var_os(INNER).is_some() {
        if !can_make_links() {
            eprintln!("skip: the namespace has no CAP_NET_ADMIN or no bridge link");
            return;
        }
        veth_body();
        return;
    }
    rerun_in_namespace(
        module_path!(),
        "guest_wire_veth_carries_tcp_and_udp_of_a_linux_guest",
    );
}

fn tun_body() {
    let fd = match wire::open_tun("t248t") {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("skip: the namespace has no TUN device: {e}");
            return;
        }
    };
    // The kernel of this namespace is the guest: its address is on the TUN,
    // and its default route goes to the router through it.
    addr("t248t", GUEST, 24);
    up("t248t");
    netkit::default_via(GATEWAY, "t248t");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let a = Fake::new("a");
    let router = router(&rt, "tun:t248t", &a);
    let task = rt.spawn(router.guests[0].clone().serve(Opened::Packets(fd)));
    guest_flows();
    assert_eq!(a.dialed(), ["93.184.216.34:80"]);
    task.abort();
    eprintln!("ran: a Linux guest on a TUN wire");
}

/// T-248: the data path of a real TUN, with the kernel of the namespace as
/// the guest.
#[test]
fn guest_wire_tun_carries_tcp_and_udp_of_a_linux_guest() {
    if std::env::var_os(INNER).is_some() {
        if !can_make_links() {
            eprintln!("skip: the namespace has no CAP_NET_ADMIN or no bridge link");
            return;
        }
        tun_body();
        return;
    }
    rerun_in_namespace(
        module_path!(),
        "guest_wire_tun_carries_tcp_and_udp_of_a_linux_guest",
    );
}
