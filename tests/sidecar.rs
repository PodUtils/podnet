//! The packet sidecar at the podnet surface (GitHub #495): the five
//! primitives, the genetic search, the three-valued probe, split and
//! tamper, fake TCP, the bond and the limits page. Every test runs against
//! the local simulator, and none opens a network socket.

#![cfg(feature = "sidecar")]

use std::process::Command;

use podnet::sidecar::{
    self as side, Behaviour, Primitive, ProbePath, SplitError, Step, Strategy, Trigger, Verdict,
};

fn strategy(primitive: Primitive) -> Strategy {
    Strategy::new(
        side::date(),
        side::NETWORK,
        "hand",
        vec![Step::new(Trigger::FirstOutboundData, primitive)],
    )
}

/// Each primitive changes the conversation that the baseline carries.
#[test]
fn the_five_primitives_divergence_from_the_baseline() {
    let cases = [
        (Primitive::Drop, "drop"),
        (Primitive::Tamper, "tamper"),
        (Primitive::Duplicate, "duplicate"),
        (Primitive::Fragment, "fragment"),
        (Primitive::Coalesce, "coalesce"),
    ];
    for (primitive, name) in cases {
        let run = side::run_strategy(&strategy(primitive));
        assert!(
            !run.divergence.is_empty(),
            "{name}: the simulator saw no divergence"
        );
    }
}

/// Drop shortens the stream: the server gets fewer bytes than the baseline.
#[test]
fn drop_shortens_the_delivered_stream() {
    let run = side::run_strategy(&strategy(Primitive::Drop));
    let baseline = side::run_strategy(&Strategy::new(
        side::date(),
        side::NETWORK,
        "hand",
        Vec::new(),
    ));
    assert!(
        run.divergence
            .bytes
            .iter()
            .any(|diff| diff.got.len() < diff.expected.len()),
        "drop: no stream got shorter: {}",
        run.divergence.summary()
    );
    assert!(
        baseline.divergence.is_empty(),
        "the baseline with no step diverged: {}",
        baseline.divergence.summary()
    );
}

/// Tamper changes a byte and the receiver keeps a different stream.
#[test]
fn tamper_changes_a_byte_of_the_stream() {
    let run = side::run_strategy(&strategy(Primitive::Tamper));
    assert!(
        run.divergence
            .bytes
            .iter()
            .any(|diff| diff.expected != diff.got),
        "tamper: the two streams agree: {}",
        run.divergence.summary()
    );
}

/// Duplicate adds one outbound segment and the stream stays the same.
#[test]
fn duplicate_adds_a_segment_without_changing_the_stream() {
    let run = side::run_strategy(&strategy(Primitive::Duplicate));
    assert!(
        run.divergence
            .segments
            .iter()
            .any(|diff| diff.got > diff.expected),
        "duplicate: no direction got more segments: {}",
        run.divergence.summary()
    );
    assert!(
        run.divergence.bytes.is_empty(),
        "duplicate changed the stream: {}",
        run.divergence.summary()
    );
}

/// Fragment adds one segment and the stream stays the same.
#[test]
fn fragment_adds_a_segment_without_changing_the_stream() {
    let run = side::run_strategy(&strategy(Primitive::Fragment));
    assert!(
        run.divergence
            .segments
            .iter()
            .any(|diff| diff.got > diff.expected),
        "fragment: no direction got more segments: {}",
        run.divergence.summary()
    );
    assert!(
        run.divergence.bytes.is_empty(),
        "fragment changed the stream: {}",
        run.divergence.summary()
    );
}

/// Coalesce removes one segment and the stream stays the same.
#[test]
fn coalesce_removes_a_segment_without_changing_the_stream() {
    let run = side::run_strategy(&strategy(Primitive::Coalesce));
    assert!(
        run.divergence
            .segments
            .iter()
            .any(|diff| diff.got < diff.expected),
        "coalesce: no direction got fewer segments: {}",
        run.divergence.summary()
    );
    assert!(
        run.divergence.bytes.is_empty(),
        "coalesce changed the stream: {}",
        run.divergence.summary()
    );
}

/// The genetic search converges on a hand-written strategy: the same
/// divergence, in fewer generations than the limit.
#[test]
fn the_search_converges_on_a_hand_written_strategy() {
    let target = strategy(Primitive::Drop);
    let generations = 40;
    let outcome = side::search_towards(&target, 7, generations).expect("the search ran");
    let target_run = side::run_strategy(&target);
    assert!(
        !target_run.divergence.is_empty(),
        "the target strategy is silent"
    );
    assert_eq!(
        outcome.run.divergence,
        target_run.divergence,
        "the search found another divergence: {}",
        outcome.run.divergence.summary()
    );
    assert!(
        outcome.generation < generations,
        "the search reached the target at the last generation"
    );
}

/// A correct path agrees, the disagreeing path disagrees, and silence is
/// the third value (PROBE-2).
#[test]
fn the_probe_has_three_values_and_silence_is_not_a_pass() {
    assert_eq!(
        side::probe(ProbePath::Simulator, None).verdict,
        Verdict::Agrees
    );
    assert_eq!(
        side::probe(ProbePath::Disagreeing, None).verdict,
        Verdict::Disagrees
    );
    let silent = side::probe(ProbePath::Silent, None);
    assert_eq!(silent.verdict, Verdict::Unknown);
    assert!(
        silent.reason.contains("could not determine"),
        "the silent reason does not say it could not determine: {}",
        silent.reason
    );
    let words: Vec<&str> = [Verdict::Agrees, Verdict::Disagrees, Verdict::Unknown]
        .into_iter()
        .map(side::verdict_word)
        .collect();
    assert_eq!(words, ["agrees", "disagrees", "could not determine"]);
}

/// A raw probe with no target, and a host without a raw socket, is
/// `could not determine` with a named reason (PROBE-2, LIM-1).
#[test]
fn a_raw_probe_that_cannot_run_is_unknown() {
    let finding = side::probe(ProbePath::Raw, None);
    assert_eq!(finding.verdict, Verdict::Unknown);
    assert!(!finding.reason.is_empty());
}

/// A strategy file with no date exits 78 and names the date.
#[test]
fn a_strategy_with_no_date_exits_78() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let file = dir.path().join("strategy.toml");
    std::fs::write(
        &file,
        "network = \"local-simulator\"\nfound_by = \"hand\"\nsteps = []\n",
    )
    .expect("the strategy file");
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["sidecar", "run", "--strategy"])
        .arg(&file)
        .env("PODNET_STATE_DIR", dir.path().join("state"))
        .env("PODNET_RUN_DIR", dir.path().join("run"))
        .output()
        .expect("the binary ran");
    assert_eq!(
        out.status.code(),
        Some(78),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("date"),
        "the error does not name the date: {stderr}"
    );
    assert!(
        stderr.contains("strategy"),
        "the error does not name the strategy: {stderr}"
    );
}

/// A dated strategy runs and prints its date, its network and each step.
/// The undated file of the test above is refused; this one is applied.
#[test]
fn a_dated_strategy_runs_and_prints_its_steps() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let file = dir.path().join("strategy.toml");
    std::fs::write(
        &file,
        "date = \"2026-11-20\"\n\
         network = \"local-simulator\"\n\
         found_by = \"hand\"\n\
         \n\
         [[steps]]\n\
         trigger = \"first-outbound-data\"\n\
         primitive = \"drop\"\n\
         \n\
         [[steps]]\n\
         trigger = \"syn-ack\"\n\
         primitive = \"fragment\"\n\
         offset = 8\n",
    )
    .expect("the strategy file");
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["sidecar", "run", "--strategy"])
        .arg(&file)
        .env("PODNET_STATE_DIR", dir.path().join("state"))
        .env("PODNET_RUN_DIR", dir.path().join("run"))
        .output()
        .expect("the binary ran");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("date       2026-11-20 network local-simulator found_by hand"),
        "{text}"
    );
    assert!(text.contains("step 00  FirstOutboundData drop"), "{text}");
    assert!(text.contains("step 01  SynAck fragment"), "{text}");
    assert!(text.contains("divergence"), "{text}");
    assert!(text.contains("segments   out"), "{text}");
}

/// Each split behaviour runs on the default path and is defeated by the
/// strict path, and each names a limit on the page.
#[test]
fn each_split_behaviour_has_a_stated_defeat() {
    let outcomes = side::split_all();
    assert_eq!(outcomes.len(), Behaviour::ALL.len());
    for outcome in &outcomes {
        assert!(
            outcome.run.is_ok(),
            "{}: the default path failed: {:?}",
            outcome.behaviour,
            outcome.run
        );
        assert!(
            outcome.strict.is_err(),
            "{}: the strict path did not defeat it",
            outcome.behaviour
        );
        assert!(
            outcome.limit.is_some(),
            "{}: no limit on the page",
            outcome.behaviour
        );
        let defeat = outcome.defeat();
        assert!(
            !defeat.is_empty(),
            "{}: no defeat stated",
            outcome.behaviour
        );
    }
    assert!(
        matches!(outcomes[0].strict, Err(SplitError::Reassembled { .. })),
        "the first behaviour is not defeated by reassembly: {:?}",
        outcomes[0].strict
    );
}

/// A UDP echo crosses a middlebox that refuses a plain datagram.
#[test]
fn a_udp_echo_crosses_a_middlebox_that_refuses_udp() {
    let echo = side::fake_tcp_echo(b"podnet sidecar").expect("the echo crossed");
    assert_eq!(echo, b"podnet sidecar");
}

/// A raw path that this host refuses is a named skip, not a failure.
#[test]
fn a_refused_raw_socket_is_a_named_skip() {
    match side::raw_skip() {
        Some(reason) => assert!(
            reason.contains("raw socket") || reason.contains("CAP_NET_RAW"),
            "the skip does not name the raw socket: {reason}"
        ),
        None => {
            // A host that allows a raw socket must say so, not stay silent.
            let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
                .args(["sidecar", "faketcp", "--text", "podnet sidecar"])
                .output()
                .expect("the binary ran");
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(
                text.contains("raw      this host allows a raw socket"),
                "a raw-capable host did not report the raw path: {text}"
            );
        }
    }
}

/// A failed echo exits nonzero and names the reason (GitHub #495). The text
/// is 100000 bytes, more than the 16 fragments that the receiver's window
/// holds, so the framing fails.
#[test]
fn a_failed_fake_tcp_echo_exits_nonzero_with_the_reason() {
    let payload = "x".repeat(100_000);
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["sidecar", "faketcp", "--text", &payload])
        .output()
        .expect("the binary ran");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    eprintln!("status {:?}\n{text}", out.status.code());
    assert_eq!(
        out.status.code(),
        Some(69),
        "the failure is not a fault: {text}"
    );
    assert!(
        text.contains("more than 16 fragments wait"),
        "the failure does not name the fragment bound: {text}"
    );
    assert!(text.contains("fake tcp"), "{text}");
}

/// `FirstInboundRst` fires only in the scenario where the server sends a
/// RST. The search runs `Echo`, where no inbound RST exists, so a strategy
/// with this trigger is silent there. This test runs the strategy in the
/// `Refused` scenario and records the outcome: the dropped RST leaves the
/// client in SYN-SENT instead of CLOSED.
#[test]
fn the_first_inbound_rst_trigger_reaches_a_refused_connection() {
    assert!(
        Trigger::ALL.contains(&Trigger::FirstInboundRst),
        "the search's candidate triggers lost FirstInboundRst"
    );
    assert_eq!(
        Trigger::parse("firstinboundrst").expect("the TOML spelling"),
        Trigger::FirstInboundRst
    );
    let strategy = Strategy::new(
        side::date(),
        side::NETWORK,
        "hand",
        vec![Step::new(Trigger::FirstInboundRst, Primitive::Drop)],
    );
    let echo = side::run_strategy(&strategy);
    assert!(
        echo.divergence.is_empty(),
        "the Echo scenario has no inbound RST: {}",
        echo.divergence.summary()
    );
    let run = side::run_strategy_refused(&strategy);
    let client = run
        .divergence
        .states
        .iter()
        .find(|diff| diff.expected.as_str() == "CLOSED" && diff.got.as_str() == "SYN-SENT");
    assert!(
        client.is_some(),
        "the RST trigger changed no client state: {}",
        run.divergence.summary()
    );
    eprintln!("FirstInboundRst in Refused: {}", run.divergence.summary());
}

/// Two links at 10:1 carry the flow within 20 percent of the sum, and it
/// does not move while its link lives.
#[test]
fn the_bond_carries_one_flow_within_twenty_percent_and_does_not_move_it() {
    let report = side::bond_report().expect("the bond");
    assert_eq!(report.sum, 11);
    assert_eq!(report.flow, Some(10));
    assert!(
        report.within_twenty_percent(),
        "10 of 11 is not within 20 percent"
    );
    assert_eq!(report.moves, 0, "the flow moved on a live link");
}

/// The limits page names every limit, with its condition and its check. A
/// split behaviour with no row fails this test (LIM-1).
#[test]
fn the_limits_page_names_each_limit() {
    let page = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/docs/limits-carriers.md"
    ))
    .expect("docs/limits-carriers.md");
    for limit in podnet::sidecar::LIMITS {
        for field in [limit.behaviour, limit.condition, limit.detection] {
            assert!(
                page.contains(field),
                "the limits page does not name {field:?} for {}",
                limit.behaviour
            );
        }
    }
    for behaviour in Behaviour::ALL {
        let name = behaviour.name();
        assert!(
            podnet::sidecar::limits::limit_of(name).is_some(),
            "the split behaviour {name} has no limit"
        );
        assert!(page.contains(name), "the limits page does not name {name}");
    }
}
