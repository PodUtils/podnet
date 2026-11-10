//! The tests of the core modules (crates/podnet-core, T-047) that read a
//! module of a higher layer or a file of the repository. They stay in the
//! root crate: the core cannot use a higher layer, and a crate cannot read
//! a file outside its directory and still package.

mod services {
    /// The fallback nameservers of `warp tun` and the nameserver of the raw
    /// pod (live run 42222905277: an empty list made warp run --backend
    /// netns-raw panic after the take of pod-services).
    // An endpoint of the role udp gives its address as its host, with no
    // list of addresses: socket_addrs takes the host, ip_addrs does not.
    #[test]
    fn the_nameservers_of_the_service_are_1_1_1_1_and_1_0_0_1() {
        let want: Vec<std::net::IpAddr> =
            vec!["1.1.1.1".parse().unwrap(), "1.0.0.1".parse().unwrap()];
        assert_eq!(crate::warp::route::nameservers_of_service(), want);
    }
}

mod yaml {
    /// T-080: an error of a config file names the line of its rule.
    #[test]
    fn a_config_error_names_its_line() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.yml");
        let text = "tunnel: t\ningress:\n  - hostname: a.example\n    service: http://127.0.0.1:1\n  # the second rule\n  - hostnam: b.example\n    service: http://127.0.0.1:2\n  - service: http_status:404\n";
        std::fs::write(&file, text).unwrap();
        let err = format!("{:#}", crate::config::load_file(&file).unwrap_err());
        assert!(err.contains("ingress rule 2 (line 6)"), "{err}");
        assert_eq!(
            crate::yaml::numbered("ingress rule", 4, &[1, 2]),
            "ingress rule 5"
        );
    }
}

mod trust {
    /// The roots of pod-tls are the version of webpki-roots in Cargo.lock.
    #[test]
    fn the_roots_version_is_the_one_of_cargo_lock() {
        let lock = include_str!("../Cargo.lock");
        let versions: Vec<&str> = lock
            .split("[[package]]")
            .filter(|p| p.contains("name = \"webpki-roots\""))
            .filter_map(|p| p.lines().find_map(|l| l.strip_prefix("version = \"")))
            .map(|v| v.trim_end_matches('"'))
            .collect();
        assert!(
            versions.contains(&pod_tls::ROOTS_VERSION),
            "webpki-roots in Cargo.lock: {versions:?}"
        );
        let newest = versions
            .iter()
            .filter(|v| v.starts_with("1."))
            .max()
            .unwrap();
        assert_eq!(*newest, pod_tls::ROOTS_VERSION);
    }

    /// Only these files may set a certificate verifier of their own: the
    /// origin option noTLSVerify (podnet-core util/tls.rs, origin/dial.rs), the MASQUE key pin
    /// (warp/session.rs, which always compares the key), and the doctor
    /// (doctor/tls_check.rs: it records the chain, gives the decision to webpki, and
    /// closes each session after the handshake with no data).
    #[test]
    fn no_other_code_sets_its_own_verifier() {
        const ALLOWED: &[&str] = &[
            "src/doctor/tls_check.rs",
            "src/origin/dial.rs",
            "crates/podnet-core/src/util/tls.rs",
            "src/warp/session.rs",
        ];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut stack = vec![root.join("src")];
        for entry in std::fs::read_dir(root.join("crates")).unwrap().flatten() {
            stack.push(entry.path().join("src"));
        }
        let mut found = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                let marker = ["dangerous", "()"].concat();
                if text.contains(&marker)
                    || text.contains(&["impl ServerCert", "Verifier for"].concat())
                {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    found.push(rel);
                }
            }
        }
        found.sort();
        for file in &found {
            assert!(
                ALLOWED.contains(&file.as_str()),
                "{file} sets its own certificate verifier"
            );
        }
        assert_eq!(found.len(), ALLOWED.len(), "{found:?}");
    }
}
