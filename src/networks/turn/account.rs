//! The account and the limits of a TURN network (T-321): the variables of
//! the process, or the variables with the suffix of the name of a network of
//! a router file. A fault names the variables and never holds a value.

use std::fmt;
use std::time::Duration;

use pod_core::Failure;
use pod_turn::Limits;

use super::{
    LIFETIME_VARIABLE, PASSWORD_FILE_VARIABLE, PASSWORD_VARIABLE, RATE_LIMIT_VARIABLE,
    REALM_VARIABLE, USER_VARIABLE,
};
use crate::networks::usage;

/// The user, the password and the realm of the allocations.
#[derive(Clone)]
pub(crate) struct Account {
    pub(crate) user: String,
    pub(crate) password: String,
    pub(crate) realm: Option<String>,
}

impl Account {
    #[cfg(test)]
    pub(crate) fn new(user: &str, password: &str) -> Account {
        Account {
            user: user.to_string(),
            password: password.to_string(),
            realm: None,
        }
    }
}

/// The user alone: the password never leaves the value as text.
impl fmt::Debug for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Account")
            .field("user", &self.user)
            .field("password", &"(hidden)")
            .field("realm", &self.realm)
            .finish()
    }
}

/// The suffix of the variables of the router network `name` (T-321): its
/// name in capitals, with `-` as `_` (`office-4g` gives `_OFFICE_4G`). It is
/// given when `PODNET_TURN_USER_<SUFFIX>` is set; the network then reads
/// each variable with the suffix. Else it reads the variables with no
/// suffix, the account of the process.
pub(crate) fn suffix_of(name: Option<&str>, get: &dyn Fn(&str) -> Option<String>) -> String {
    let Some(name) = name else {
        return String::new();
    };
    let suffix = format!("_{}", name.to_ascii_uppercase().replace('-', "_"));
    match get(&format!("{USER_VARIABLE}{suffix}")) {
        Some(user) if !user.is_empty() => suffix,
        _ => String::new(),
    }
}

/// Read the account of the process with `get` (the variables of the
/// environment). A fault names the variables and never holds a value.
#[cfg(test)]
pub(crate) fn account(get: &dyn Fn(&str) -> Option<String>) -> Result<Account, Failure> {
    account_of(get, "")
}

/// Read the account of the variables with `suffix` ([`suffix_of`]).
pub(crate) fn account_of(
    get: &dyn Fn(&str) -> Option<String>,
    suffix: &str,
) -> Result<Account, Failure> {
    let user_var = format!("{USER_VARIABLE}{suffix}");
    let file_var = format!("{PASSWORD_FILE_VARIABLE}{suffix}");
    let password_var = format!("{PASSWORD_VARIABLE}{suffix}");
    let set = |name: &str| get(name).filter(|v| !v.is_empty());
    let user = set(&user_var).ok_or_else(|| {
        usage(format!(
            "TURN needs a user: set {user_var} (and {file_var} or {password_var})"
        ))
    })?;
    let password = match (set(&file_var), set(&password_var)) {
        (Some(_), Some(_)) => {
            return Err(usage(format!(
                "set one of {file_var} and {password_var}, not both"
            )))
        }
        (Some(file), None) => {
            let text = std::fs::read_to_string(&file)
                .map_err(|e| usage(format!("{file_var}: cannot read {file}: {e}")))?;
            let line = text.strip_suffix('\n').unwrap_or(&text);
            line.strip_suffix('\r').unwrap_or(line).to_string()
        }
        (None, Some(password)) => password,
        (None, None) => {
            return Err(usage(format!(
                "TURN needs a password: set {file_var} (a file with one line) or {password_var}"
            )))
        }
    };
    if password.is_empty() {
        return Err(usage(format!("{file_var}: the file holds no password")));
    }
    Ok(Account {
        user,
        password,
        realm: set(&format!("{REALM_VARIABLE}{suffix}")),
    })
}

/// Read the lifetime and the rate limit of the settings, and check them
/// before any connection. A value that is not a number is a usage error
/// that names the variable, never a value.
/// The variables have the suffix `suffix` ([`suffix_of`]).
pub(crate) fn limits_of(
    get: &dyn Fn(&str) -> Option<String>,
    suffix: &str,
) -> Result<(Limits, Option<u64>), Failure> {
    let lifetime_var = format!("{LIFETIME_VARIABLE}{suffix}");
    let rate_var = format!("{RATE_LIMIT_VARIABLE}{suffix}");
    let mut limits = Limits::default();
    if let Some(text) = get(&lifetime_var).filter(|v| !v.is_empty()) {
        let secs: u64 = text.parse().map_err(|_| {
            usage(format!(
                "{lifetime_var} is the lifetime in seconds, a number: {text:?} is not one"
            ))
        })?;
        if !(1..=3600).contains(&secs) {
            return Err(usage(format!("{lifetime_var} is 1 to 3600 seconds")));
        }
        limits.lifetime = Duration::from_secs(secs);
        // A refresh comes before the end, at most at half of the lifetime.
        limits.refresh_before = Duration::from_secs(secs / 2).max(Duration::from_secs(1));
    }
    let rate_limit = match get(&rate_var).filter(|v| !v.is_empty()) {
        Some(text) => Some(
            text.parse::<u64>()
                .map_err(|_| usage(format!("{rate_var} is a number of bytes for each second")))?,
        ),
        None => None,
    };
    Ok((limits, rate_limit))
}
