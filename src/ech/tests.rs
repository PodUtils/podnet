use super::*;

/// Build a synthetic valid RFC 9849 ECHConfigList for tests.
fn sample_ech_config_list(public_name: &str, max_name_length: u8) -> Vec<u8> {
    let suite = pod_tls::crypto::hpke::SUITES[0];
    let (pk, _) = suite.generate_key_pair().expect("key pair");
    let mut body = vec![
        0x07, // config_id
        0x00, 0x20, // kem_id = DHKEM(X25519, HKDF-SHA256)
    ];
    body.extend_from_slice(&(pk.0.len() as u16).to_be_bytes());
    body.extend_from_slice(&pk.0);
    // cipher_suites: len=4, kdf=HKDF-SHA256 (0x0001), aead=AES-128-GCM (0x0001)
    body.extend_from_slice(&[0x00, 0x04, 0x00, 0x01, 0x00, 0x01]);
    body.push(max_name_length);
    body.push(public_name.len() as u8);
    body.extend_from_slice(public_name.as_bytes());
    body.extend_from_slice(&[0x00, 0x00]); // extensions_len = 0

    let mut cfg = vec![0xfe, 0x0d];
    cfg.extend_from_slice(&(body.len() as u16).to_be_bytes());
    cfg.extend_from_slice(&body);

    let mut list = (cfg.len() as u16).to_be_bytes().to_vec();
    list.extend_from_slice(&cfg);
    list
}

#[test]
fn ech_setting_parses_modes_base64_and_rejects_invalid() {
    for off in ["", "off", "OFF", "no", "false", "0"] {
        let s = EchSetting::parse(off).unwrap();
        assert_eq!(s, EchSetting::Off, "{off}");
        assert_eq!(s.mode(), Mode::Off);
    }
    for on in ["on", "ON", "yes", "auto", "AUTO", "true", "1"] {
        let s = EchSetting::parse(on).unwrap();
        assert_eq!(s, EchSetting::On, "{on}");
        assert_eq!(s.mode(), Mode::On);
    }
    let strict = EchSetting::parse("strict").unwrap();
    assert_eq!(strict, EchSetting::Strict);
    assert_eq!(strict.mode(), Mode::Strict);

    let raw = sample_ech_config_list("cloudflare-ech.com", 18);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
    let parsed = EchSetting::parse(&b64).unwrap();
    assert_eq!(parsed, EchSetting::ConfigList(raw));
    assert_eq!(parsed.mode(), Mode::On);

    for bad in ["maybe", "not-base64!!!", "AAAA"] {
        assert!(EchSetting::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn parse_ech_config_list_and_check_name_length() {
    let raw = sample_ech_config_list("cloudflare-ech.com", 18);
    let infos = parse_ech_config_list(&raw).unwrap();
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].public_name, "cloudflare-ech.com");
    assert_eq!(infos[0].max_name_length, 18);
    assert_eq!(infos[0].version, ECH_VERSION_DRAFT_13);

    // 36 > 18: error must name both 36 and 18.
    let long_name = "consumer-masque.cloudflareclient.com";
    assert_eq!(long_name.len(), 36);
    let err = check_name_length(long_name, &infos[0]).unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("36") && text.contains("18") && text.contains("cloudflare-ech.com"),
        "{text}"
    );

    // Short name (<= 18) passes.
    assert!(check_name_length("api.example.com", &infos[0]).is_ok());
    assert!(check_name_length("cloudflare-ech.com", &infos[0]).is_ok());

    // max_name_length == 0 (unspecified per RFC 9849) accepts long names.
    let open = sample_ech_config_list("cloudflare-ech.com", 0);
    let open_infos = parse_ech_config_list(&open).unwrap();
    assert_eq!(open_infos[0].max_name_length, 0);
    assert!(check_name_length(long_name, &open_infos[0]).is_ok());

    // Truncated or malformed lists fail.
    assert!(parse_ech_config_list(&[]).is_err());
    assert!(parse_ech_config_list(&[0x00, 0x05, 0xfe]).is_err());
}

#[test]
fn ech_hops_parses_combinations_and_rejects_unknown() {
    assert_eq!(EchHops::parse("").unwrap(), EchHops::all());
    assert_eq!(EchHops::parse("all").unwrap(), EchHops::all());
    assert_eq!(
        EchHops::parse("api,masque").unwrap(),
        EchHops {
            api: true,
            edge: false,
            masque: true
        }
    );
    assert_eq!(
        EchHops::parse("edge").unwrap(),
        EchHops {
            api: false,
            edge: true,
            masque: false
        }
    );
    assert_eq!(EchHops::parse("none").unwrap(), EchHops::none());
    assert!(EchHops::parse("api,bogus").is_err());
}

#[test]
fn pure_rust_hpke_builds_ech_and_grease_configs() {
    crate::init_crypto();
    let raw = sample_ech_config_list("cloudflare-ech.com", 64);
    assert!(validate_ech_config_list(&raw).is_ok());
    assert!(grease_config().is_ok());
    let grease_cfg = grease_client_config(&[], &[b"http/1.1"]).unwrap();
    assert_eq!(grease_cfg.alpn_protocols, vec![b"http/1.1".to_vec()]);
    let sealed_cfg = sealed_client_config(&raw, &[], &[b"h2"]).unwrap();
    assert_eq!(sealed_cfg.alpn_protocols, vec![b"h2".to_vec()]);
}

#[tokio::test]
async fn doctor_probe_reports_published_fit_and_missing() {
    struct MockRecords {
        list_short: Vec<u8>,
    }
    impl Records for MockRecords {
        fn ech_list<'a>(
            &'a self,
            name: &'a str,
            _deadline: Instant,
        ) -> pod_core::BoxFuture<'a, std::result::Result<Option<Published>, String>> {
            let list = self.list_short.clone();
            let name = name.to_string();
            Box::pin(async move {
                if name.contains("masque") {
                    Ok(Some(Published {
                        list,
                        ttl: Some(300),
                    }))
                } else if name.contains("trycloudflare") {
                    Ok(None)
                } else {
                    Err("dns timeout".into())
                }
            })
        }
    }
    let mock = MockRecords {
        list_short: sample_ech_config_list("cloudflare-ech.com", 18),
    };
    let row = probe_hops_with(&mock, EchHops::all(), Duration::from_secs(2)).await;
    assert_eq!(row.hops.len(), 4);
    let masque = row.hops.iter().find(|h| h.hop == "masque").unwrap();
    assert!(masque.published);
    assert_eq!(masque.public_name.as_deref(), Some("cloudflare-ech.com"));
    assert_eq!(masque.max_name_length, Some(18));
    assert_eq!(masque.name_fits, Some(false));
    assert!(masque.detail.contains("36") && masque.detail.contains("18"));

    let api = row.hops.iter().find(|h| h.hop == "api").unwrap();
    assert!(!api.published);

    let text = row.text();
    assert!(text.contains("ech    mode="), "{text}");
    assert!(text.contains("masque"), "{text}");
    let json = row.to_json();
    assert_eq!(json["hops"].as_array().unwrap().len(), 4);
}
