use crate::cli::cmd::agent::agent_commands;

// Issues 110 and 116: a unix-only warp up gets warp exec and
// ssh-config, and no eval line.
#[test]
fn the_agent_commands_follow_the_listeners() {
    let unix_only = agent_commands(true, false);
    assert!(
        !unix_only.iter().any(|c| c.contains("eval")),
        "{unix_only:?}"
    );
    assert!(unix_only.iter().any(|c| c.contains("podnet warp exec")));
    assert!(unix_only.iter().any(|c| c.contains("ssh-config")));
    let tcp = agent_commands(true, true);
    assert!(tcp[0].starts_with("eval"), "{tcp:?}");
    let direct = agent_commands(false, true);
    assert_eq!(direct, ["podnet agent status", "podnet agent down"]);
}
