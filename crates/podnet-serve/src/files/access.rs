//! The access of the file server (T-089): users with the roles read and
//! write (HTTP Basic), the PIN gate of `share`, and an anonymous read. The
//! check is a layer of the router of the files, so it works the same on a
//! local listener, through the tunnel and through the relay.
//!
//! The file of `--users` has one user on each line, `USER:ROLE:SECRET`.
//! ROLE is `read` or `write`. SECRET is a hash of [`hash_password`]
//! (`pbkdf2-sha256$ITERATIONS$SALT$KEY`, hex), or `@PATH`: the password is
//! the first line of the file PATH (mode 0600; a relative PATH starts at the
//! directory of the users file). An empty line and a line that starts with
//! `#` are skipped.

use std::collections::HashSet;
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use axum::body::Body;
use axum::extract::FromRequest;
use axum::response::{IntoResponse, Response};

use crate::gate::{Gate, UNLOCK_PATH};

/// The role of a user. A write user can also read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Read,
    Write,
}

impl Role {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "read" => Ok(Role::Read),
            "write" => Ok(Role::Write),
            other => bail!("the role {other:?} is not read or write"),
        }
    }

    /// The role that a request with `method` needs: read for GET and HEAD,
    /// write for each other method.
    pub fn needed(method: &http::Method) -> Self {
        if super::is_read(method) {
            Role::Read
        } else {
            Role::Write
        }
    }
}

/// The iterations of a new hash of [`hash_password`].
pub const HASH_ITERATIONS: u32 = 210_000;
/// The prefix of a hash of [`hash_password`].
const HASH_SCHEME: &str = "pbkdf2-sha256";
/// The most iterations of a hash in a users file.
const MAX_ITERATIONS: u32 = 10_000_000;
/// The most entries of the cache of the checked passwords.
const CACHE_MAX: usize = 256;

#[derive(Clone)]
enum Secret {
    /// PBKDF2-HMAC-SHA256.
    Hash {
        iterations: NonZeroU32,
        salt: Vec<u8>,
        key: Vec<u8>,
    },
    /// A password of a password file.
    Password(pod_proxy::Credentials),
}

#[derive(Clone)]
struct User {
    name: String,
    role: Role,
    secret: Secret,
}

/// The users of a file server, with their roles.
#[derive(Clone, Default)]
pub struct Users {
    list: Vec<User>,
    /// The SHA-256 of `user NUL password` of each hash that matched: a
    /// second request of a user does not compute PBKDF2 again.
    checked: Arc<Mutex<HashSet<[u8; 32]>>>,
}

impl std::fmt::Debug for Users {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<String> = self
            .list
            .iter()
            .map(|u| format!("{}:{:?}", u.name, u.role))
            .collect();
        write!(f, "Users({}, <hidden>)", names.join(", "))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// A hash of `password` for a users file, with a random salt and
/// [`HASH_ITERATIONS`] iterations.
pub fn hash_password(password: &str) -> String {
    hash_password_with(password, HASH_ITERATIONS)
}

/// [`hash_password`] with `iterations` iterations (at least 1).
pub fn hash_password_with(password: &str, iterations: u32) -> String {
    let iterations = NonZeroU32::new(iterations).unwrap_or(NonZeroU32::MIN);
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).expect("OS entropy unavailable");
    let mut key = [0u8; 32];
    ring::pbkdf2::derive(
        ring::pbkdf2::PBKDF2_HMAC_SHA256,
        iterations,
        &salt,
        password.as_bytes(),
        &mut key,
    );
    format!("{HASH_SCHEME}${iterations}${}${}", hex(&salt), hex(&key))
}

fn parse_hash(text: &str) -> Result<Secret> {
    let parts: Vec<&str> = text.split('$').collect();
    let [scheme, iterations, salt, key] = parts[..] else {
        bail!("a hash has the form {HASH_SCHEME}$ITERATIONS$SALT$KEY");
    };
    if scheme != HASH_SCHEME {
        bail!("the hash scheme {scheme:?} is not {HASH_SCHEME}");
    }
    let iterations: u32 = iterations
        .parse()
        .ok()
        .filter(|n| (1..=MAX_ITERATIONS).contains(n))
        .with_context(|| format!("the iterations must be 1 to {MAX_ITERATIONS}"))?;
    let salt = unhex(salt)
        .filter(|s| !s.is_empty())
        .context("a bad salt")?;
    let key = unhex(key).filter(|k| k.len() == 32).context("a bad key")?;
    Ok(Secret::Hash {
        iterations: NonZeroU32::new(iterations).expect("checked above"),
        salt,
        key,
    })
}

impl Users {
    /// The users of the file at `path`. The file must be a regular file of
    /// this user that others cannot read.
    pub fn load(path: &Path) -> Result<Self> {
        let text = pod_core::files::read_own(path, pod_core::files::Others::NoAccess)
            .map_err(anyhow::Error::msg)?
            .with_context(|| format!("{}: no such file", path.display()))?;
        let base = path.parent().unwrap_or(Path::new("."));
        Self::parse(&text, base).with_context(|| path.display().to_string())
    }

    /// The users of `text`; a relative `@PATH` starts at `base`.
    pub fn parse(text: &str, base: &Path) -> Result<Self> {
        let mut users = Users::default();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            let at = || format!("line {}", index + 1);
            let mut fields = line.splitn(3, ':');
            let (Some(name), Some(role), Some(secret)) =
                (fields.next(), fields.next(), fields.next())
            else {
                bail!("{}: give USER:ROLE:HASH or USER:ROLE:@PASSWORD-FILE", at());
            };
            let role = Role::parse(role).with_context(at)?;
            let secret = match secret.strip_prefix('@') {
                Some(file) => Secret::Password(
                    pod_proxy::Credentials::load(name, &base.join(file))
                        .map_err(anyhow::Error::msg)
                        .with_context(at)?,
                ),
                None => {
                    if name.is_empty() || name.len() > 255 {
                        bail!("{}: a user has 1 to 255 bytes", at());
                    }
                    parse_hash(secret).with_context(at)?
                }
            };
            if users.list.iter().any(|u| u.name == name) {
                bail!("{}: the user {name:?} is in the file two times", at());
            }
            users.list.push(User {
                name: name.to_string(),
                role,
                secret,
            });
        }
        if users.list.is_empty() {
            bail!("no user in the file");
        }
        Ok(users)
    }

    /// Add the user of `credentials` with `role` (`--auth-user`).
    pub fn add(&mut self, credentials: pod_proxy::Credentials, role: Role) -> Result<()> {
        let name = credentials.user().to_string();
        if self.list.iter().any(|u| u.name == name) {
            bail!("the user {name:?} is given two times");
        }
        self.list.push(User {
            name,
            role,
            secret: Secret::Password(credentials),
        });
        Ok(())
    }

    /// True when a user has the write role.
    pub fn has_writer(&self) -> bool {
        self.list.iter().any(|u| u.role == Role::Write)
    }

    /// The role of the user of an `Authorization: Basic` value, or `None`
    /// for a wrong user or password.
    pub fn check_basic(&self, value: Option<&str>) -> Option<Role> {
        self.check_basic_user(value).map(|(_, role)| role)
    }

    /// The user and the role of an `Authorization: Basic` value, or
    /// `None` for a wrong user or password.
    pub fn check_basic_user(&self, value: Option<&str>) -> Option<(String, Role)> {
        use base64::Engine as _;
        let (scheme, token) = value?.trim().split_once(' ')?;
        if !scheme.eq_ignore_ascii_case("basic") {
            return None;
        }
        let raw = base64::engine::general_purpose::STANDARD
            .decode(token.trim())
            .ok()?;
        let raw = String::from_utf8(raw).ok()?;
        let (name, password) = raw.split_once(':')?;
        Some((name.to_string(), self.check(name, password)?))
    }

    /// The role of `name` with `password`, or `None`.
    pub fn check(&self, name: &str, password: &str) -> Option<Role> {
        let user = self.list.iter().find(|u| u.name == name)?;
        let ok = match &user.secret {
            Secret::Password(credentials) => credentials.check(name, password),
            Secret::Hash {
                iterations,
                salt,
                key,
            } => {
                let digest = {
                    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
                    ctx.update(name.as_bytes());
                    ctx.update(&[0]);
                    ctx.update(password.as_bytes());
                    let mut out = [0u8; 32];
                    out.copy_from_slice(ctx.finish().as_ref());
                    out
                };
                let cached = self.checked.lock().is_ok_and(|set| set.contains(&digest));
                cached || {
                    let ok = ring::pbkdf2::verify(
                        ring::pbkdf2::PBKDF2_HMAC_SHA256,
                        *iterations,
                        salt,
                        password.as_bytes(),
                        key,
                    )
                    .is_ok();
                    if ok {
                        if let Ok(mut set) = self.checked.lock() {
                            if set.len() >= CACHE_MAX {
                                set.clear();
                            }
                            set.insert(digest);
                        }
                    }
                    ok
                }
            }
        };
        ok.then_some(user.role)
    }
}

/// Who can reach the file server.
#[derive(Clone, Debug, Default)]
pub struct Access {
    /// The users of `--users` and `--auth-user`. With users, each request
    /// must give a user (HTTP Basic), except a read with `anon_read`.
    pub users: Option<Users>,
    /// The PIN gate of `--pin`. A visitor unlocks it one time; then the
    /// session cookie opens it.
    pub pin: Option<Gate>,
    /// A GET or HEAD needs no user and no PIN.
    pub anon_read: bool,
}

impl Access {
    /// True when the access checks nothing.
    pub fn is_open(&self) -> bool {
        self.users.is_none() && self.pin.is_none()
    }

    /// True when some visitor can get the write role: a write user, or a
    /// PIN with no users.
    pub fn can_write(&self) -> bool {
        match &self.users {
            Some(users) => users.has_writer(),
            None => self.pin.is_some(),
        }
    }

    /// The role of a request, or the answer that refuses it. With no users
    /// and no PIN, each visitor has the read role. With a PIN and no
    /// users, a visitor that unlocks the gate has the write role.
    pub(crate) fn role(&self, request: &axum::extract::Request) -> Result<Role, Box<Response>> {
        self.grant(request, Role::needed(request.method()))
            .map(|(role, _)| role)
    }

    /// The role and the user of a request that needs the role `need`, or
    /// the answer that refuses it (git hosting, T-092: a push is a POST
    /// that needs the write role, and a fetch is a POST that needs the
    /// read role). The user is `None` with no users.
    pub(crate) fn grant(
        &self,
        request: &axum::extract::Request,
        need: Role,
    ) -> Result<(Role, Option<String>), Box<Response>> {
        if self.anon_read && need == Role::Read {
            return Ok((Role::Read, None));
        }
        if let Some(gate) = &self.pin {
            if let Some(locked) =
                crate::share::gate_blocked(gate, request.method(), request.headers(), request.uri())
            {
                return Err(Box::new(locked));
            }
        }
        let (role, user) = match &self.users {
            Some(users) => {
                let value = request
                    .headers()
                    .get(http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok());
                let (user, role) = users
                    .check_basic_user(value)
                    .ok_or_else(|| Box::new(unauthorized()))?;
                (role, Some(user))
            }
            None if self.pin.is_some() => (Role::Write, None),
            None => (Role::Read, None),
        };
        if role < need {
            return Err(Box::new(
                (
                    http::StatusCode::FORBIDDEN,
                    "this user has the read role: a change needs the write role",
                )
                    .into_response(),
            ));
        }
        Ok((role, user))
    }
}

/// 401 with a challenge of HTTP Basic.
fn unauthorized() -> Response {
    (
        http::StatusCode::UNAUTHORIZED,
        [(
            http::header::WWW_AUTHENTICATE,
            "Basic realm=\"podnet\", charset=\"UTF-8\"",
        )],
        "Unauthorized",
    )
        .into_response()
}

/// The role of a request, for the handlers after the layer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Granted(pub(crate) Role);

/// The layer of the access: the unlock of the PIN gate, then the role of
/// the request. A request that passes gets [`Granted`] as an extension.
pub(crate) async fn layer(
    axum::extract::State(access): axum::extract::State<Arc<Access>>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if let Some(gate) = &access.pin {
        if request.uri().path() == UNLOCK_PATH {
            return unlock(gate, request).await;
        }
    }
    match access.role(&request) {
        Ok(role) => {
            request.extensions_mut().insert(Granted(role));
            next.run(request).await
        }
        Err(refused) => *refused,
    }
}

/// The POST of the PIN form.
async fn unlock(gate: &Gate, request: axum::extract::Request) -> Response {
    if request.method() != http::Method::POST {
        return (http::StatusCode::METHOD_NOT_ALLOWED, "use POST").into_response();
    }
    let (parts, body) = request.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, crate::gate::MAX_UNLOCK_BODY as usize).await else {
        return (http::StatusCode::PAYLOAD_TOO_LARGE, "the form is too large").into_response();
    };
    let headers = parts.headers.clone();
    let uri = parts.uri.clone();
    let request = axum::extract::Request::from_parts(parts, Body::from(bytes));
    match axum::Form::<crate::share::UnlockForm>::from_request(request, &()).await {
        Ok(axum::Form(form)) => {
            crate::share::unlock_response(gate, &headers, &uri, &form.password, &form.next)
        }
        Err(rejection) => rejection.into_response(),
    }
}

#[cfg(test)]
#[path = "access_tests.rs"]
mod tests;
