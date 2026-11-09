//! The command that starts the child of `--detach`.

use std::path::Path;
use std::process::{Command, Stdio};

use super::{
    Options, CHILD_FLAG, COMMAND_VARIABLE, HASH_VARIABLE, NAME_VARIABLE, READY_TIMEOUT_VARIABLE,
    SUPERVISED_VARIABLE, SUPERVISE_FLAG,
};

/// True for a `PODNET_` variable that an autostart entry keeps. The report
/// program and the supervisor mark of a detached process are of that process
/// only (T-205).
pub(crate) fn kept_variable(key: &str) -> bool {
    key.starts_with("PODNET_") && !matches!(key, "PODNET_ERROR_PROGRAM" | SUPERVISED_VARIABLE)
}

/// The command of the child (`name` and `hash` of the instance). The child
/// does not inherit `PODNET_DETACH_SUPERVISED`: a nested detached start with
/// `--no-restart` has no supervisor (T-205).
pub(super) fn child_command(
    program: &Path,
    args: &[String],
    vars: &[(String, String)],
    (name, hash): (&str, &str),
    command: &str,
    options: &Options,
) -> Command {
    let mut cmd = Command::new(program);
    {
        use std::os::unix::process::CommandExt;
        cmd.arg0("podnet");
    }
    cmd.arg(if options.restart {
        SUPERVISE_FLAG
    } else {
        CHILD_FLAG
    })
    .args(args)
    .envs(vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
    .env_remove(SUPERVISED_VARIABLE)
    .env(NAME_VARIABLE, name)
    .env(HASH_VARIABLE, hash)
    .env(
        READY_TIMEOUT_VARIABLE,
        options.ready_timeout.as_secs().to_string(),
    )
    .env(COMMAND_VARIABLE, command)
    .stdin(Stdio::inherit())
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit());
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_secrets_skips_supervised_variable() {
        assert!(kept_variable("PODNET_PROXY"));
        assert!(!kept_variable("PODNET_DETACH_SUPERVISED"));
        assert!(!kept_variable("PODNET_ERROR_PROGRAM"));
        assert!(!kept_variable("HOME"));
    }

    #[test]
    fn child_env_has_no_supervised_variable() {
        let options = Options {
            name: None,
            ready_timeout: std::time::Duration::from_secs(1),
            replace: false,
            restart: false,
        };
        let vars = vec![(SUPERVISED_VARIABLE.to_string(), "1".to_string())];
        let cmd = child_command(
            Path::new("podnet"),
            &["tunnel".to_string()],
            &vars,
            ("n", "h"),
            "tunnel",
            &options,
        );
        let set: Vec<_> = cmd
            .get_envs()
            .filter(|(k, _)| *k == SUPERVISED_VARIABLE)
            .collect();
        assert!(!set.is_empty());
        assert!(set.iter().all(|(_, v)| v.is_none()), "{set:?}");
    }
}
