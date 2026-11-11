use super::*;

pub(super) fn wan(name: &str, dev: &str) -> WanConfig {
    WanConfig {
        name: name.into(),
        dev: dev.into(),
        gateway: None,
        weight: None,
        table: None,
        mark: None,
    }
}

pub(super) fn section(wans: Vec<WanConfig>) -> KernelConfig {
    KernelConfig {
        wans,
        ..KernelConfig::default()
    }
}

#[test]
fn kernel_plan_puts_the_suppress_rule_before_the_rules_of_the_clients() {
    let plan = Plan::new(&section(vec![wan("a", "tun0"), wan("b", "tun1")])).unwrap();
    let rules = plan.rules();
    assert_eq!(
        rules,
        vec![
            Rule {
                priority: 1000,
                select: Select::SuppressDefault,
                table: MAIN
            },
            Rule {
                priority: 1001,
                select: Select::Mark(1),
                table: 100
            },
            Rule {
                priority: 1002,
                select: Select::Mark(2),
                table: 101
            },
            Rule {
                priority: 1003,
                select: Select::Any,
                table: 200
            },
        ]
    );
    let routes = plan.routes();
    assert_eq!(routes.len(), 3);
    assert_eq!(routes[2].0, 200);
    assert_eq!(routes[2].1.len(), 2, "the aggregate route holds each WAN");
}

#[test]
fn kernel_config_errors_name_the_key() {
    let mut cases: Vec<(KernelConfig, &str)> = vec![
        (section(vec![]), "kernel.wans"),
        (section(vec![wan("a b", "x")]), "kernel.wans[0].name"),
        (section(vec![wan("a", "")]), "kernel.wans[0].dev"),
        (
            section(vec![wan("a", "x"), wan("a", "y")]),
            "kernel.wans[1]",
        ),
        (
            section(vec![wan("a", "x"), wan("b", "x")]),
            "kernel.wans[1]",
        ),
    ];
    let mut w = wan("a", "x");
    w.gateway = Some("nope".into());
    cases.push((section(vec![w]), "kernel.wans[0].gateway"));
    let mut w = wan("a", "x");
    w.weight = Some(0);
    cases.push((section(vec![w]), "kernel.wans[0].weight"));
    let mut w = wan("a", "x");
    w.table = Some(254);
    cases.push((section(vec![w]), "kernel.wans[0].table"));
    let mut w = wan("a", "x");
    w.mark = Some(0);
    cases.push((section(vec![w]), "kernel.wans[0].mark"));
    let mut k = section(vec![wan("a", "x")]);
    k.table = Some(255);
    cases.push((k, "kernel.table"));
    let mut k = section(vec![wan("a", "x")]);
    k.priority = Some(32765);
    cases.push((k, "kernel.priority"));
    for (config, key) in cases {
        let err = Plan::new(&config).unwrap_err().to_string();
        assert!(err.contains(key), "{key}: {err}");
    }
}

#[test]
fn kernel_section_keeps_each_old_file_valid() {
    let old = "networks:\n  home: direct\ndoors:\n  - listen: unix:/tmp/r.sock\n    via: home\n";
    let config = crate::router::parse(old).unwrap();
    assert!(config.kernel.is_none());
    // A kernel file needs no door.
    let text = "kernel:\n  wans:\n    - name: a\n      dev: tun0\n    - name: b\n      dev: tun1\n      weight: 3\n";
    let config = crate::router::parse(text).unwrap();
    let plan = Plan::new(config.kernel.as_ref().unwrap()).unwrap();
    assert_eq!(plan.wans[1].weight, 3);
    // A file with no door and no kernel stays an error.
    let err = crate::router::parse("networks:\n  home: direct\n").unwrap_err();
    assert!(err.to_string().contains("doors"), "{err}");
    let err = crate::router::parse("kernel:\n  wans: []\n").unwrap_err();
    assert!(err.to_string().contains("kernel.wans"), "{err}");
}

#[test]
fn kernel_requests_encode_rules_and_a_multipath_route() {
    let rule = Rule {
        priority: 1001,
        select: Select::Mark(7),
        table: 300,
    };
    let msg = new_rule_request(&rule);
    let total = u32::from_ne_bytes(msg[0..4].try_into().unwrap()) as usize;
    assert_eq!(total, msg.len());
    assert_eq!(u16::from_ne_bytes([msg[4], msg[5]]), RTM_NEWRULE);
    let flags = u16::from_ne_bytes([msg[6], msg[7]]);
    assert_ne!(flags & NLM_F_EXCL, 0, "a copy of a rule is refused");
    let found = attrs(&msg[16 + 12..]);
    assert!(found.contains(&(FRA_PRIORITY, &1001u32.to_ne_bytes()[..])));
    assert!(found.contains(&(FRA_FWMARK, &7u32.to_ne_bytes()[..])));
    assert!(found.contains(&(FRA_TABLE, &300u32.to_ne_bytes()[..])));
    let gw = Some(Ipv4Addr::new(10, 0, 0, 1));
    let msg = new_route_request(200, &[(3, gw, 2), (4, None, 1)]);
    assert_eq!(u16::from_ne_bytes([msg[4], msg[5]]), RTM_NEWROUTE);
    let flags = u16::from_ne_bytes([msg[6], msg[7]]);
    assert_ne!(flags & NLM_F_REPLACE, 0, "the route is replaced in place");
    let (_, multipath) = attrs(&msg[16 + 12..])
        .into_iter()
        .find(|(k, _)| *k == RTA_MULTIPATH)
        .unwrap();
    // First next hop: 8 bytes and the gateway attribute (8 bytes).
    assert_eq!(u16::from_ne_bytes([multipath[0], multipath[1]]), 16);
    assert_eq!(multipath[3], 1, "weight 2 is hops 1");
    assert_eq!(u32::from_ne_bytes(multipath[4..8].try_into().unwrap()), 3);
    assert_eq!(&multipath[12..16], &[10, 0, 0, 1]);
    assert_eq!(u16::from_ne_bytes([multipath[16], multipath[17]]), 8);
    assert_eq!(multipath[19], 0);
}

// ── the namespace ───────────────────────────────────────────────────────────

pub(super) const INNER: &str = "PODNET_T119_INNER";
const RTM_NEWLINK: u16 = 16;
const RTM_NEWADDR: u16 = 20;

/// The request of a link of a kind that every kernel has (a bridge).
pub(super) fn link_request(name: &str) -> Vec<u8> {
    let mut b = vec![0u8; 16];
    attr(&mut b, 3, format!("{name}\0").as_bytes());
    let start = b.len();
    b.extend_from_slice(&[0; 4]);
    attr(&mut b, 1, b"bridge");
    let len = (b.len() - start) as u16;
    b[start..start + 2].copy_from_slice(&len.to_ne_bytes());
    b[start + 2..start + 4].copy_from_slice(&18u16.to_ne_bytes());
    message(RTM_NEWLINK, NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL, b)
}

/// A bridge link that is up, with the address `addr`/24.
pub(super) fn bridge(sock: &Socket, name: &str, addr: [u8; 4]) -> u32 {
    let code = sock.ack(&link_request(name));
    assert_eq!(code.unwrap(), 0, "bridge link {name}");
    let index = ifindex(name).unwrap();
    // Up: ifi_flags and ifi_change are IFF_UP.
    let mut b = vec![0u8; 16];
    b[4..8].copy_from_slice(&index.to_ne_bytes());
    b[8..12].copy_from_slice(&1u32.to_ne_bytes());
    b[12..16].copy_from_slice(&1u32.to_ne_bytes());
    assert_eq!(sock.ack(&message(RTM_NEWLINK, NLM_F_ACK, b)).unwrap(), 0);
    let mut b = vec![AF_INET, 24, 0, 0];
    b.extend_from_slice(&index.to_ne_bytes());
    attr(&mut b, 2, &addr);
    attr(&mut b, 1, &addr);
    let flags = NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL;
    assert_eq!(sock.ack(&message(RTM_NEWADDR, flags, b)).unwrap(), 0);
    index
}

fn kernel_rules(plan: &Plan) -> Vec<Rule> {
    read_rules(plan.priority, plan.priority + 100).unwrap()
}

/// The body of the test; it runs in a new user and network namespace.
fn in_namespace() {
    let sock = Socket::open().unwrap();
    let ia = bridge(&sock, "t119a", [192, 0, 2, 2]);
    let ib = bridge(&sock, "t119b", [198, 51, 100, 2]);
    let ic = bridge(&sock, "t119c", [203, 0, 113, 2]);
    let (ga, gb) = (Ipv4Addr::new(192, 0, 2, 1), Ipv4Addr::new(198, 51, 100, 1));
    let mut a = wan("a", "t119a");
    a.gateway = Some(ga.to_string());
    a.weight = Some(2);
    let mut b = wan("b", "t119b");
    b.gateway = Some(gb.to_string());
    let one = Plan::new(&section(vec![a.clone(), b])).unwrap();
    apply(&one, None).unwrap();
    assert_eq!(kernel_rules(&one), one.rules(), "rules of the plan");
    assert_eq!(read_default(100).unwrap(), Some(vec![(ia, Some(ga), 1)]));
    assert_eq!(read_default(101).unwrap(), Some(vec![(ib, Some(gb), 1)]));
    assert_eq!(
        read_default(200).unwrap(),
        Some(vec![(ia, Some(ga), 2), (ib, Some(gb), 1)])
    );
    // A second apply changes nothing and does not fail.
    apply(&one, None).unwrap();
    assert_eq!(kernel_rules(&one), one.rules(), "no copy of a rule");

    // B goes away, C comes with no gateway, and A weighs 3.
    a.weight = Some(3);
    let two = Plan::new(&section(vec![a, wan("c", "t119c")])).unwrap();
    apply(&two, Some(&one)).unwrap();
    assert_eq!(kernel_rules(&two), two.rules());
    assert_eq!(read_default(101).unwrap(), Some(vec![(ic, None, 1)]));
    assert_eq!(
        read_default(200).unwrap(),
        Some(vec![(ia, Some(ga), 3), (ic, None, 1)])
    );

    remove(&two).unwrap();
    assert_eq!(kernel_rules(&two), vec![]);
    for table in [100, 101, 200] {
        assert_eq!(read_default(table).unwrap(), None, "table {table}");
    }
}

/// True when the namespace can make a bridge link (CAP_NET_ADMIN).
pub(super) fn can_make_links() -> bool {
    let Ok(sock) = Socket::open() else {
        return false;
    };
    sock.ack(&link_request("t119probe")).ok() == Some(0)
}

/// T-119, the Prove. The test runs itself in `unshare -rn`, and skips when
/// the host does not allow a user and network namespace.
#[test]
fn route_kernel_tables_and_rules_in_a_namespace() {
    if std::env::var_os(INNER).is_some() {
        if !can_make_links() {
            eprintln!("skip: the namespace has no CAP_NET_ADMIN or no bridge link");
            return;
        }
        in_namespace();
        return;
    }
    rerun_in_namespace(
        module_path!(),
        "route_kernel_tables_and_rules_in_a_namespace",
    );
}

/// Run the test `name` of the module `module` again in `unshare -rn` with
/// [`INNER`] set. The test passes with a message when the host does not
/// allow the namespace.
pub(super) fn rerun_in_namespace(module: &str, name: &str) {
    let me = std::env::current_exe().unwrap();
    let path = module.split_once("::").unwrap().1;
    let run = std::process::Command::new("unshare")
        .arg("-rn")
        .arg(&me)
        .args(["--exact", &format!("{path}::{name}")])
        .args(["--nocapture", "--test-threads=1"])
        .env(INNER, "1")
        .output();
    let out = match run {
        Ok(out) => out,
        Err(e) => {
            eprintln!("skip: no unshare program ({e})");
            return;
        }
    };
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    // A failure of unshare itself (before the test runs): "unshare failed:
    // ..." or "unshare: write failed /proc/self/uid_map" (AppArmor on GitHub
    // runners). The test binary never prints a line that starts so.
    let refused = text
        .lines()
        .any(|l| l.starts_with("unshare failed") || l.starts_with("unshare: "));
    if !out.status.success() && refused {
        eprintln!("skip: the host does not allow a user and network namespace: {text}");
        return;
    }
    assert!(out.status.success(), "{text}");
    if let Some(line) = text.lines().find(|l| l.contains("skip:")) {
        eprintln!("{line}");
    }
}

/// T-121, the Prove: the plan in the JSON form of the mwan director.
#[test]
fn route_export_mwan_json() {
    let mut a = wan("a", "tun0");
    a.gateway = Some("10.0.0.1".into());
    a.weight = Some(2);
    let plan = Plan::new(&section(vec![a, wan("b", "tun1")])).unwrap();
    let doc = plan.mwan_json(Some("1.1.1.1:443"));
    assert_eq!(
        doc,
        serde_json::json!({
            "wans": [
                {"name": "a", "iface": "tun0", "table": 100, "weight": 2,
                 "gateway": "10.0.0.1", "check_target": "1.1.1.1:443"},
                {"name": "b", "iface": "tun1", "table": 101, "weight": 1,
                 "check_target": "1.1.1.1:443"},
            ],
            "clients": [],
        })
    );
    let bare = plan.mwan_json(None);
    assert!(bare["wans"][1].get("check_target").is_none());
    assert!(bare["wans"][1].get("gateway").is_none());
}
