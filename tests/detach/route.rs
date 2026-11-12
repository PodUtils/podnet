use super::*;

/// T-100, the Prove: `podnet route --detach` gives a ready document with
/// its doors and the members of its groups; `ps` lists it; a door answers;
/// `stop` ends it.
#[test]
fn route_detaches_and_lists_its_members() {
    let env = Env::new();
    let dir = env.sock.parent().unwrap().to_path_buf();
    let door = dir.join("door.sock");
    let config = dir.join("route.yml");
    std::fs::write(
        &config,
        format!(
            "networks:\n  home: direct\n  spare: direct\ngroups:\n  egress: home*2, spare*1\ndoors:\n  - listen: unix:{}\n    via: egress\n",
            door.display()
        ),
    )
    .unwrap();
    let config = config.to_str().unwrap();
    let check = env.run(&["route", "--config", config, "--check"]);
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let start = env.run(&["route", "--config", config, "--detach", "--name", "r1"]);
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    let doc = ready_doc(&start);
    assert_eq!(doc["name"], "r1");
    let members: Vec<&str> = doc["groups"][0]["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(members, ["home", "spare"]);
    assert_eq!(
        doc["doors"][0]["listen"],
        format!("unix:{}", door.display())
    );
    assert!(connects(&door), "the door answers");
    let listed = env.ps()["instances"].as_array().unwrap().clone();
    assert!(listed.iter().any(|i| i["name"] == "r1"));
    let stop = env.run(&["stop", "r1", "--timeout", "5"]);
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
    // A bad file is a usage error that names the key.
    std::fs::write(
        dir.join("bad.yml"),
        "doors:\n  - listen: unix:/x\n    via: nowhere\n",
    )
    .unwrap();
    let bad = env.run(&[
        "route",
        "--config",
        dir.join("bad.yml").to_str().unwrap(),
        "--check",
    ]);
    assert_eq!(bad.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("doors[0].via"));
}

/// T-180: `route --control` pins a member, changes the probe interval,
/// rotates a network (one rotate for each minute) and reconnects it, on a
/// running router.
#[test]
fn route_control_flags_drive_a_running_router() {
    let env = Env::new();
    let dir = env.sock.parent().unwrap().to_path_buf();
    let (door, ctl) = (dir.join("door.sock"), dir.join("ctl.sock"));
    let config = dir.join("route.yml");
    std::fs::write(
        &config,
        format!(
            "networks:\n  home: direct\n  spare: direct\ngroups:\n  egress: home*2, spare*1\nhealth:\n  target: 192.0.2.1:443\n  interval: 30\nstate_dir: {}\ncontrol: unix:{}\ndoors:\n  - listen: unix:{}\n    via: egress\n",
            dir.join("state").display(),
            ctl.display(),
            door.display()
        ),
    )
    .unwrap();
    let start = env.run(&[
        "route",
        "--config",
        config.to_str().unwrap(),
        "--detach",
        "--name",
        "r2",
    ]);
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    let control = format!("unix:{}", ctl.display());
    let run = |flags: &[&str]| {
        let mut args = vec!["route", "--control", control.as_str()];
        args.extend_from_slice(flags);
        env.run(&args)
    };
    let pinned = run(&["--select", "egress=spare", "--json"]);
    assert!(
        pinned.status.success(),
        "{}",
        String::from_utf8_lossy(&pinned.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&pinned.stdout).unwrap();
    assert_eq!(doc["groups"][0]["pinned"], "spare");
    let text = run(&["--select", "egress=auto"]);
    assert!(String::from_utf8_lossy(&text.stdout).contains("pinned -"));
    let interval = run(&["--interval", "7", "--json"]);
    let doc: serde_json::Value = serde_json::from_slice(&interval.stdout).unwrap();
    assert_eq!(doc["interval"], 7);
    assert!(run(&["--reconnect", "home"]).status.success());
    assert!(run(&["--rotate", "home"]).status.success());
    assert_eq!(run(&["--rotate", "home"]).status.code(), Some(75));
    assert!(dir.join("state/home").is_dir());
    assert_eq!(run(&["--select", "egress=nope"]).status.code(), Some(64));
    assert_eq!(run(&["--interval", "0"]).status.code(), Some(64));
    let stop = env.run(&["stop", "r2", "--timeout", "5"]);
    assert!(stop.status.success());
    assert_eq!(run(&["--interval", "7"]).status.code(), Some(69));
}

/// T-097: `route --status unix:/PATH --metrics` prints the rows of a running
/// router, one for each member, in the text format of Prometheus.
#[test]
fn route_status_metrics_prints_the_rows() {
    let env = Env::new();
    let dir = env.sock.parent().unwrap().to_path_buf();
    let (door, ctl) = (dir.join("door.sock"), dir.join("ctl.sock"));
    let config = dir.join("route.yml");
    std::fs::write(
        &config,
        format!(
            "networks:\n  home: direct\n  spare: direct\ngroups:\n  egress: home*2, spare*1\ncontrol: unix:{}\ndoors:\n  - listen: unix:{}\n    via: egress\n",
            ctl.display(),
            door.display()
        ),
    )
    .unwrap();
    let start = env.run(&[
        "route",
        "--config",
        config.to_str().unwrap(),
        "--detach",
        "--name",
        "r3",
    ]);
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    let control = format!("unix:{}", ctl.display());
    let rows = env.run(&["route", "--status", control.as_str(), "--metrics"]);
    assert!(
        rows.status.success(),
        "{}",
        String::from_utf8_lossy(&rows.stderr)
    );
    let text = String::from_utf8_lossy(&rows.stdout);
    assert!(
        text.contains("podnet_route_member_up{group=\"egress\",member=\"home\"} 1"),
        "{text}"
    );
    assert!(
        text.contains("podnet_route_member_weight{group=\"egress\",member=\"spare\"} 1"),
        "{text}"
    );
    // --metrics and --json are two forms of one document: give one.
    let both = env.run(&["route", "--status", control.as_str(), "--metrics", "--json"]);
    assert_eq!(both.status.code(), Some(64));
    let stop = env.run(&["stop", "r3", "--timeout", "5"]);
    assert!(stop.status.success());
    let gone = env.run(&["route", "--status", control.as_str(), "--metrics"]);
    assert_eq!(gone.status.code(), Some(69));
}
