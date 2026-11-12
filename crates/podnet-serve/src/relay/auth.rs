//! The login to an SSH relay (T-318). The client tries the methods in the
//! order of the forward-relay watcher of `PodUtils/podcrates`: a key made
//! for the run; then no key, which also gives the list of methods that the
//! server offers; then keyboard-interactive with an empty answer, when the
//! list names it. A relay of the free tier takes one of these. The client
//! sends no password and keeps no key.

use russh::client::{AuthResult, Handle, Handler, KeyboardInteractiveAuthResponse};

use super::*;

/// The rounds of keyboard-interactive that the client answers.
const ROUNDS: usize = 4;

/// How a login ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Login {
    /// The relay took this method: `publickey`, `none` or
    /// `keyboard-interactive`.
    In(&'static str),
    /// The relay took no method. The text names the methods that it
    /// offered and the last failure.
    Refused(String),
}

/// The methods of a refusal, as the server names them.
fn names(methods: &russh::MethodSet) -> Vec<&'static str> {
    methods.iter().map(<&str>::from).collect()
}

/// Log in as `user`. An error is a failure of the connection, not a
/// refusal of the relay.
pub async fn authenticate<H: Handler>(
    handle: &mut Handle<H>,
    user: &str,
    key: Arc<PrivateKey>,
) -> Result<Login, russh::Error> {
    let key = PrivateKeyWithHashAlg::new(key, None);
    if handle.authenticate_publickey(user, key).await?.success() {
        return Ok(Login::In("publickey"));
    }
    // No key: the server answers with the list of methods that it offers.
    let mut last = "none was refused";
    let offered = match handle.authenticate_none(user).await? {
        AuthResult::Success => return Ok(Login::In("none")),
        AuthResult::Failure {
            remaining_methods, ..
        } => names(&remaining_methods),
    };
    if offered.contains(&"keyboard-interactive") {
        if keyboard_interactive(handle, user).await? {
            return Ok(Login::In("keyboard-interactive"));
        }
        last = "keyboard-interactive was refused";
    }
    let offered = if offered.is_empty() {
        "nothing".to_string()
    } else {
        offered.join(", ")
    };
    Ok(Login::Refused(format!(
        "the relay offers: {offered}; podnet tries publickey, none and an empty keyboard-interactive; last failure: {last}"
    )))
}

/// An empty answer to each prompt, for a few rounds: a relay that lets
/// anyone in asks nothing that a user must know.
async fn keyboard_interactive<H: Handler>(
    handle: &mut Handle<H>,
    user: &str,
) -> Result<bool, russh::Error> {
    let mut reply = handle
        .authenticate_keyboard_interactive_start(user, None::<String>)
        .await?;
    for _ in 0..ROUNDS {
        match reply {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let answers = vec![String::new(); prompts.len()];
                reply = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await?;
            }
        }
    }
    Ok(matches!(reply, KeyboardInteractiveAuthResponse::Success))
}
