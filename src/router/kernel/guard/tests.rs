use super::super::tests::{bridge, can_make_links, rerun_in_namespace, section, wan, INNER};
use super::super::*;
use super::*;

fn config(text: &str) -> Plan {
    let parsed = crate::router::parse(text).unwrap();
    Plan::new(parsed.kernel.as_ref().unwrap()).unwrap()
}

#[test]
fn kernel_guard_keys_default_on_and_keep_old_files_valid() {
    let old = "kernel:\n  wans:\n    - name: a\n      dev: tun0\n";
    assert_eq!(config(old).guard, Guard::default());
    assert!(Guard::default().kill_switch && Guard::default().masquerade);
    let off = "kernel:\n  masquerade: false\n  mss_clamp: false\n  kill_switch: false\n  rp_filter: false\n  wans:\n    - name: a\n      dev: tun0\n";
    let guard = config(off).guard;
    assert!(!guard.masquerade && !guard.mss_clamp && !guard.kill_switch && !guard.rp_filter);
    let bad = "kernel:\n  masquerade: maybe\n  wans:\n    - name: a\n      dev: tun0\n";
    assert!(crate::router::parse(bad).is_err());
}

#[test]
fn kernel_guard_requests_encode_the_unreachable_route_and_the_batch() {
    let msg = new_unreachable_request(200);
    assert_eq!(u16::from_ne_bytes([msg[4], msg[5]]), RTM_NEWROUTE);
    assert_eq!(msg[16 + 7], RTN_UNREACHABLE, "the route type");
    let found = attrs(&msg[16 + 12..]);
    assert!(found.contains(&(RTA_TABLE, &200u32.to_ne_bytes()[..])));
    assert!(found.contains(&(RTA_PRIORITY, &UNREACHABLE_METRIC.to_ne_bytes()[..])));

    let plan = Plan::new(&section(vec![wan("a", "tun0"), wan("b", "tun1")])).unwrap();
    let batch = nft_batch(&plan);
    let kinds: Vec<u16> = messages(&batch).iter().map(|(k, _)| *k & 0xff).collect();
    // begin, table, delete, table, chain nat, 2 rules, chain mss, 2 rules, end.
    let want = [0x10, 0, 2, 0, 3, 6, 6, 3, 6, 6, 0x11];
    assert_eq!(kinds, want);
    // Without the guards of nf_tables the batch only deletes the table.
    let mut off = section(vec![wan("a", "tun0")]);
    off.masquerade = Some(false);
    off.mss_clamp = Some(false);
    let batch = nft_batch(&Plan::new(&off).unwrap());
    assert_eq!(messages(&batch).len(), 4);
}

/// Set the link `index` up or down.
fn set_link(sock: &Socket, index: u32, up: bool) {
    let mut b = vec![0u8; 16];
    b[4..8].copy_from_slice(&index.to_ne_bytes());
    b[8..12].copy_from_slice(&u32::from(up).to_ne_bytes());
    b[12..16].copy_from_slice(&1u32.to_ne_bytes());
    assert_eq!(sock.ack(&message(16, NLM_F_ACK, b)).unwrap(), 0);
}

/// The result of a UDP `connect` to a public address: the route lookup.
fn lookup() -> std::io::Result<()> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("8.8.8.8:53")
}

fn kill_switch_body() {
    let sock = Socket::open().unwrap();
    let ia = bridge(&sock, "t120a", [192, 0, 2, 2]);
    let ib = bridge(&sock, "t120b", [198, 51, 100, 2]);
    let im = bridge(&sock, "t120m", [203, 0, 113, 2]);
    // The host default route in the main table: the leak path.
    let main_gw = Some(Ipv4Addr::new(203, 0, 113, 1));
    let code = sock.ack(&new_route_request(MAIN, &[(im, main_gw, 1)]));
    assert_eq!(code.unwrap(), 0, "main default route");
    let (ga, gb) = (Ipv4Addr::new(192, 0, 2, 1), Ipv4Addr::new(198, 51, 100, 1));
    let (mut a, mut b) = (wan("a", "t120a"), wan("b", "t120b"));
    a.gateway = Some(ga.to_string());
    b.gateway = Some(gb.to_string());
    let mut config = section(vec![a, b]);
    config.masquerade = Some(false);
    config.mss_clamp = Some(false);
    config.rp_filter = Some(false);
    let on = Plan::new(&config).unwrap();
    apply(&on, None).unwrap();
    assert!(
        read_unreachable(200).unwrap(),
        "the kill switch is in place"
    );
    assert_eq!(
        read_default(200).unwrap(),
        Some(vec![(ia, Some(ga), 1), (ib, Some(gb), 1)]),
        "the unreachable route is not the default route of the table"
    );
    lookup().expect("a route through a WAN");

    // Each WAN down: no route in the aggregate table, and no leak. The kernel
    // gives EHOSTUNREACH for an `unreachable` route (ENETUNREACH is for no route).
    set_link(&sock, ia, false);
    set_link(&sock, ib, false);
    let err = lookup().expect_err("each WAN is down");
    assert_eq!(err.raw_os_error(), Some(libc::EHOSTUNREACH), "{err}");

    // Without the kill switch the lookup goes on to the main table.
    assert_eq!(sock.ack(&del_unreachable_request(200)).unwrap(), 0);
    assert!(!read_unreachable(200).unwrap(), "the switch is removed");
    lookup().expect("the leak: the main table takes the packet");

    // The switch comes back, and the WANs with it.
    apply_kill_switch(&sock, &on).unwrap();
    assert!(lookup().is_err());
    set_link(&sock, ia, true);
    set_link(&sock, ib, true);
    apply(&on, Some(&on)).unwrap();
    lookup().expect("a route through a WAN again");

    remove(&on).unwrap();
    assert!(!read_unreachable(200).unwrap(), "remove deletes the switch");
}

/// T-120, the Prove. With each WAN down, a client of the aggregate gets
/// an error of the unreachable route (EHOSTUNREACH) and no packet goes to the main table. The test
/// runs itself in `unshare -rn`.
#[test]
fn route_kernel_kill_switch_does_not_leak() {
    if std::env::var_os(INNER).is_some() {
        if !can_make_links() {
            eprintln!("skip: the namespace has no CAP_NET_ADMIN or no bridge link");
            return;
        }
        kill_switch_body();
        return;
    }
    rerun_in_namespace(module_path!(), "route_kernel_kill_switch_does_not_leak");
}

fn view(chain: &str, dev: &str, exprs: &[&str]) -> RuleView {
    RuleView {
        chain: chain.into(),
        exprs: exprs.iter().map(|e| e.to_string()).collect(),
        dev: Some(dev.into()),
    }
}

const MASQ: &[&str] = &["meta", "cmp", "masq"];
const MSS: &[&str] = &[
    "meta", "cmp", "meta", "cmp", "payload", "bitwise", "cmp", "rt", "exthdr",
];

fn set_rp(dev: &str, value: &str) {
    std::fs::write(rp_path(dev), value).unwrap();
}

fn nft_body() {
    let sock = Socket::open().unwrap();
    bridge(&sock, "t120a", [192, 0, 2, 2]);
    bridge(&sock, "t120b", [198, 51, 100, 2]);
    set_rp("t120a", "1");
    set_rp("t120b", "2");
    let one = Plan::new(&section(vec![wan("a", "t120a"), wan("b", "t120b")])).unwrap();
    if let Err(e) = apply(&one, None) {
        let text = format!("{e:#}");
        // The host has no nf_tables in the namespace: not a fault of the code.
        assert!(
            text.contains("nf_tables"),
            "only nf_tables may fail here: {text}"
        );
        eprintln!("skip: nf_tables is not available in the namespace ({text})");
        return;
    }
    assert_eq!(read_rp_filter("t120a").unwrap(), 0);
    assert_eq!(read_rp_filter("t120b").unwrap(), 0);
    let want = vec![
        view("nat", "t120a", MASQ),
        view("nat", "t120b", MASQ),
        view("mss", "t120a", MSS),
        view("mss", "t120b", MSS),
    ];
    assert_eq!(read_nft().unwrap(), want, "rules of the plan");
    // A second apply replaces the table: no copy of a rule.
    apply(&one, Some(&one)).unwrap();
    assert_eq!(read_nft().unwrap(), want, "no copy of a rule");

    // Only A, with no MSS clamp.
    let mut config = section(vec![wan("a", "t120a")]);
    config.mss_clamp = Some(false);
    let two = Plan::new(&config).unwrap();
    apply(&two, Some(&one)).unwrap();
    assert_eq!(read_nft().unwrap(), vec![view("nat", "t120a", MASQ)]);
    assert_eq!(read_rp_filter("t120a").unwrap(), 0);
    assert_eq!(read_rp_filter("t120b").unwrap(), 2, "B is written back");

    remove(&two).unwrap();
    assert_eq!(read_nft().unwrap(), vec![]);
    assert_eq!(read_rp_filter("t120a").unwrap(), 1, "A is written back");
}

/// T-120: the masquerade and the MSS clamp through nf_tables netlink, and
/// `rp_filter` 0, read back from the kernel in a new namespace.
#[test]
fn route_kernel_nft_masquerade_and_mss_clamp_in_a_namespace() {
    if std::env::var_os(INNER).is_some() {
        if !can_make_links() {
            eprintln!("skip: the namespace has no CAP_NET_ADMIN or no bridge link");
            return;
        }
        nft_body();
        return;
    }
    rerun_in_namespace(
        module_path!(),
        "route_kernel_nft_masquerade_and_mss_clamp_in_a_namespace",
    );
}
