//! T-191: each command with a `--listen` flag is in the listener table of
//! `net doctor`.

use clap::CommandFactory as _;

fn listeners(cmd: &clap::Command, path: &str, out: &mut Vec<String>) {
    if cmd.get_arguments().any(|a| a.get_long() == Some("listen")) {
        out.push(path.trim().to_string());
    }
    for sub in cmd.get_subcommands() {
        let name = sub.get_name();
        if name != "help" {
            listeners(sub, &format!("{path} {name}"), out);
        }
    }
}

#[test]
fn each_command_with_a_listen_flag_is_in_the_listener_table() {
    let mut root = super::Cli::command();
    root.build();
    let mut found = Vec::new();
    listeners(&root, "", &mut found);
    assert!(found.len() >= 5, "{found:?}");
    let missing: Vec<_> = found
        .iter()
        .filter(|path| !podnet::vnet::doctor::NEEDS.contains(path.as_str()))
        .collect();
    assert!(missing.is_empty(), "not in NEEDS: {missing:?}");
}
