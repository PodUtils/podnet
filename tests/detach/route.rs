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
