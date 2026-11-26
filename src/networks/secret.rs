//! The secrets of a road of `pod-egress` (T-126): the credentials of an
//! `https://` proxy and the secret of a wstunnel server. A secret is never a
//! part of a spec (a spec goes into `argv`, logs and JSON). It comes from a
//! file or from a variable of the process, and it is read when the network
//! comes up. A fault names the variable and never holds a value.

use super::usage;
use crate::network::Failure;

/// A reader of the variables of the process (`std::env::var` in a run, a
/// table in a test).
pub(super) type Get<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The suffix of the variables of the router network `name`: its name in
/// capitals, with `-` as `_` (`office-4g` gives `_OFFICE_4G`). It is given
/// when one of `vars` with the suffix is set; the network then reads each
/// variable with the suffix. Else it reads the variables with no suffix, the
/// settings of the process (as `turn://` does).
pub(super) fn suffix_of(name: Option<&str>, get: Get<'_>, vars: &[&str]) -> String {
    let Some(name) = name else {
        return String::new();
    };
    let suffix = format!("_{}", name.to_ascii_uppercase().replace('-', "_"));
    let set = vars
        .iter()
        .any(|var| get(&format!("{var}{suffix}")).is_some_and(|v| !v.is_empty()));
    if set {
        suffix
    } else {
        String::new()
    }
}

/// The form of a secret that was given, for the status of a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Form {
    /// The file in the variable `..._FILE`.
    File,
    /// The value of the variable.
    Variable,
}

impl Form {
    pub(super) fn word(self) -> &'static str {
        match self {
            Form::File => "file",
            Form::Variable => "variable",
        }
    }
}

/// A secret, its form and the variable that gave it.
pub(super) struct Given {
    pub(super) text: String,
    pub(super) form: Form,
    pub(super) variable: String,
}

/// The value is not shown.
impl std::fmt::Debug for Given {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Given")
            .field("form", &self.form)
            .field("variable", &self.variable)
            .finish_non_exhaustive()
    }
}

/// The text of the file in `file_var`, else the value of `value_var`;
/// `None` when neither is set. Both set, a file that cannot be read and an
/// empty secret are usage faults that name the variable.
pub(super) fn read(
    get: Get<'_>,
    file_var: &str,
    value_var: &str,
) -> Result<Option<Given>, Failure> {
    let set = |name: &str| get(name).filter(|v| !v.is_empty());
    let (text, form, variable) = match (set(file_var), set(value_var)) {
        (Some(_), Some(_)) => {
            return Err(usage(format!(
                "set one of {file_var} and {value_var}, not both"
            )))
        }
        (Some(path), None) => {
            // The error of the system names the path, not the content.
            let text = std::fs::read_to_string(&path)
                .map_err(|e| usage(format!("{file_var}: cannot read {path}: {e}")))?;
            (text, Form::File, file_var)
        }
        (None, Some(value)) => (value, Form::Variable, value_var),
        (None, None) => return Ok(None),
    };
    if text.trim().is_empty() {
        return Err(usage(format!("{variable}: it holds no secret")));
    }
    Ok(Some(Given {
        text,
        form,
        variable: variable.to_string(),
    }))
}

/// `spec` with no user, no password, no path and no query, for the line of a
/// fault: a path or a query can hold a secret (the path prefix of a
/// wstunnel server).
pub(super) fn bare(spec: &str) -> String {
    let (scheme, rest) = match spec.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), spec),
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host = match authority.rsplit_once('@') {
        Some((_, host)) => format!("***@{host}"),
        None => authority.to_string(),
    };
    let tail = if end < rest.len() { "/..." } else { "" };
    format!("{scheme}{host}{tail}")
}
