use super::*;

#[test]
fn probes_run_and_render() {
    let probes = run();
    assert!(!probes.is_empty());
    let rendered = render(&probes);
    assert!(rendered.contains("bind(AF_UNIX, path)"));
    assert!(!ready(&[]), "an empty probe set is not ready");
    let allowed = |name: &'static str| Probe {
        name,
        result: ProbeResult::Ok("allowed".into()),
    };
    let sealed = [
        Probe {
            name: "bind(AF_INET, 127.0.0.1:0)",
            result: ProbeResult::Fail("EACCES".into()),
        },
        allowed("bind(AF_UNIX, path)"),
    ];
    assert_eq!(
        verdict(&sealed),
        "podnet-vnet: not ready: no C compiler for the shim"
    );
    // Issue 181: the detail of the probe, and the advice for a scratch
    // directory.
    let mut scratch = sealed.to_vec();
    scratch.push(Probe {
        name: "cc -shared -fPIC",
        result: ProbeResult::Fail("cannot create a scratch dir".into()),
    });
    let v = verdict(&scratch);
    assert!(v.contains("cannot create a scratch dir"), "{v}");
    assert!(v.contains("TMPDIR"), "{v}");
}

/// T-524 (GitHub #692): "not needed" only when a connect to the
/// loopback works; a bind and a listen alone are not enough.
#[test]
fn doctor_verdict_needs_a_loopback_connect() {
    let row = |name: &'static str, ok: bool| Probe {
        name,
        result: if ok {
            ProbeResult::Ok("allowed".into())
        } else {
            ProbeResult::Fail("ENETUNREACH".into())
        },
    };
    let table: [(&str, [bool; 4], &str); 4] = [
        ("connect works", [true, true, true, true], "podnet-vnet: not needed"),
        ("no loopback", [true, true, false, true], "podnet-vnet: TCP on the loopback does not work here: use unix sockets (warp proxy --listen unix:"),
        ("no connect at all", [true, true, false, false], "podnet-vnet: not ready: no connect"),
        ("bind denied", [false, false, false, true], "podnet-vnet: not ready: no C compiler"),
    ];
    for (name, [bind, listen, inet, unix], want) in table {
        let probes = [
            row("bind(AF_INET, 127.0.0.1:0)", bind),
            row("listen(AF_INET, 127.0.0.1:0)", listen),
            row(CONNECT_INET, inet),
            row("bind(AF_UNIX, path)", unix),
            row(CONNECT_UNIX, unix),
        ];
        let got = verdict(&probes);
        assert!(got.starts_with(want), "{name}: {got}");
    }
    // The probe set has both rows.
    let names: Vec<_> = run().iter().map(|p| p.name).collect();
    assert!(
        names.contains(&CONNECT_INET) && names.contains(&CONNECT_UNIX),
        "{names:?}"
    );
}

// T-168: a passwd file with no entry for the uid is one fail line that
// names the shim.
#[test]
fn passwd_probe_names_the_shim() {
    let dir = std::env::temp_dir().join(format!("podnet-passwd-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("passwd");
    std::fs::write(
        &file,
        "root:x:0:0:root:/root:/bin/sh\nme:x:1000:1000::/home/me:/bin/sh\n",
    )
    .unwrap();
    assert_eq!(
        passwd_probe(&file, 1000).result,
        ProbeResult::Ok("uid 1000 is me".into())
    );
    match passwd_probe(&file, 4242).result {
        ProbeResult::Fail(why) => {
            assert!(why.contains("uid 4242"), "{why}");
            assert!(why.contains("podnet net passwd-shim"), "{why}");
        }
        other => panic!("{other:?}"),
    }
    assert!(!passwd_probe(&dir.join("none"), 0).result.is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ready_requires_the_core_conditions() {
    let probes = vec![
        Probe {
            name: "bind(AF_INET, 127.0.0.1:0)",
            result: ProbeResult::Fail("EACCES".into()),
        },
        Probe {
            name: "bind(AF_UNIX, path)",
            result: ProbeResult::Ok("allowed".into()),
        },
        Probe {
            name: "cc -shared -fPIC",
            result: ProbeResult::Ok("cc".into()),
        },
    ];
    assert!(ready(&probes));
    let mut not_ready = probes.clone();
    not_ready[0].result = ProbeResult::Ok("allowed".into());
    assert!(!ready(&not_ready));
}
