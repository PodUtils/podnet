//! The replication log of PodUtils/podnet#499: two replicas write the same
//! key without seeing each other, and a merge resolves the conflict with
//! last-writer-wins and a stated tie rule.
//!
//! Both orders of the two logs give the same file and the same winner, and
//! the conflict names the hashes of the winner and of the loser. The two
//! replicas then converge on the same state.

use std::path::Path;
use std::process::Command;

use podnet::offline::replication::Replica;
use sha2::{Digest, Sha256};

fn podnet(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env("PODNET_STATE_DIR", dir.join("state"))
        .env("PODNET_RUN_DIR", dir.join("run"))
        .env_remove("RUST_LOG");
    cmd
}

fn key(dir: &Path) -> podnet::identity::Identity {
    std::fs::create_dir_all(dir).expect("a state directory");
    podnet::identity::create(dir).expect("a key")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn merge(dir: &Path, logs: &[&Path], out: &Path, key_dir: &Path) -> std::process::Output {
    let mut args = vec![
        "offline".to_string(),
        "replica".to_string(),
        "merge".to_string(),
    ];
    for log in logs {
        args.push(log.to_str().expect("a path").to_string());
    }
    args.extend_from_slice(&[
        "--out".to_string(),
        out.to_str().expect("a path").to_string(),
        "--key".to_string(),
        key_dir.to_str().expect("a path").to_string(),
        "--json".to_string(),
    ]);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    podnet(dir, &refs).output().expect("replica merge")
}

#[test]
fn two_replicas_diverge_and_converge_with_both_hashes() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let a = key(&dir.path().join("a"));
    let b = key(&dir.path().join("b"));
    let _merge_key = key(&dir.path().join("merge"));

    let mut ra = Replica::new(a.key());
    let mut rb = Replica::new(b.key());
    ra.set_session("x", b"from-a");
    rb.set_session("x", b"from-b");
    let a_log = dir.path().join("a.log");
    let b_log = dir.path().join("b.log");
    ra.save(&a_log).expect("the log of a");
    rb.save(&b_log).expect("the log of b");

    // The tie rule: the greater own counter, then the greater hex of the
    // actor. Both counters are 1, so the greater key wins.
    let (winner, loser, winner_value) = if a.public().hex() > b.public().hex() {
        (a.public(), b.public(), b"from-a")
    } else {
        (b.public(), a.public(), b"from-b")
    };
    let winner_hash = hex(&Sha256::digest(winner_value));
    let loser_value = if winner_value == b"from-a" {
        b"from-b"
    } else {
        b"from-a"
    };
    let loser_hash = hex(&Sha256::digest(loser_value));

    let one = dir.path().join("one.log");
    let two = dir.path().join("two.log");
    let first = merge(
        dir.path(),
        &[&a_log, &b_log],
        &one,
        &dir.path().join("merge"),
    );
    assert_eq!(first.status.code(), Some(0), "{first:?}");
    let second = merge(
        dir.path(),
        &[&b_log, &a_log],
        &two,
        &dir.path().join("merge"),
    );
    assert_eq!(second.status.code(), Some(0), "{second:?}");
    assert_eq!(
        std::fs::read(&one).expect("the first merge"),
        std::fs::read(&two).expect("the second merge"),
        "the two orders of the logs differ"
    );

    let doc: serde_json::Value =
        serde_json::from_slice(&first.stdout).expect("the JSON of the merge");
    assert_eq!(doc["records"].as_u64(), Some(2), "{doc}");
    assert_eq!(
        doc["state"]["session:x"],
        hex(winner_value),
        "the winner is not {winner:?}"
    );
    let conflicts = doc["conflicts"].as_array().expect("the conflicts");
    assert_eq!(conflicts.len(), 1, "{doc}");
    assert_eq!(conflicts[0]["key"], "session:x");
    assert_eq!(conflicts[0]["winner"], winner.hex());
    assert_eq!(conflicts[0]["winner_hash"], winner_hash);
    assert_eq!(conflicts[0]["losers"][0], loser.hex());
    assert_eq!(conflicts[0]["loser_hashes"][0], loser_hash);
    assert_ne!(winner_hash, loser_hash, "the two values are the same");

    // Both replicas take the log of the other and agree.
    let mut a_side = Replica::load(&a_log, a.key()).expect("the log of a");
    let mut b_side = Replica::load(&b_log, b.key()).expect("the log of b");
    a_side.merge(b_side.log());
    b_side.merge(a_side.log());
    assert_eq!(a_side.state(), b_side.state(), "the replicas diverged");
    assert_eq!(a_side.get("session:x").expect("the key"), winner_value);
    assert_eq!(b_side.get("session:x").expect("the key"), winner_value);
    assert_eq!(a_side.log().len(), 2);
    assert_eq!(b_side.log().len(), 2);
    let conflict = &a_side.conflicts()[0];
    assert_eq!(conflict.winner, winner);
    assert_eq!(conflict.winner_hash, winner_hash);
    assert_eq!(conflict.loser_hashes, vec![loser_hash]);
}
