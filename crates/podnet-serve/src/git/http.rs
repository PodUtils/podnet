//! The smart HTTP router of git hosting (T-092): `git http-backend` under
//! the CGI runner of [`super::cgi`].
//!
//! * The router answers only the three requests of the smart protocol:
//!   `GET REPO/info/refs?service=git-upload-pack` (or
//!   `git-receive-pack`), `POST REPO/git-upload-pack` and
//!   `POST REPO/git-receive-pack`. Each other path gets 404 (also
//!   `/.git/config`, and the files of the dumb protocol).
//! * The access is the access of the file server
//!   ([`crate::files::Access`]): a fetch needs the read role, a push needs
//!   the write role. A push with no credentials gets 401 with a Basic
//!   challenge, so git asks for them. `REMOTE_USER` is set only after a
//!   good authentication.
//! * The path of the repository goes through [`super::repo_rel`] and
//!   [`super::repo_dir`]: `..`, a dot segment and a path out of the root
//!   are refused.
//! * `Git-Protocol` (protocol v2) goes to `HTTP_GIT_PROTOCOL` when its
//!   value is valid. A body with `Content-Encoding: gzip` and a chunked
//!   body go to `git http-backend` as they come.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::response::{IntoResponse, Response};
use axum::Router;

use super::cgi::{self, Cgi};
use super::Service;
use crate::files::{Access, Role};

/// The settings of the router.
#[derive(Clone, Debug)]
pub struct HttpOptions {
    /// The directory that holds the repositories.
    pub root: PathBuf,
    /// The users and the anonymous read.
    pub access: Access,
    /// The `git` program.
    pub git: OsString,
    /// A push to a path with no repository makes a bare repository there.
    pub create_on_push: bool,
}

impl HttpOptions {
    /// The options for `root`: no users, `git` from `PATH`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        HttpOptions {
            root: root.into(),
            access: Access::default(),
            git: OsString::from("git"),
            create_on_push: false,
        }
    }
}

/// The part of the smart protocol of a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    /// `GET REPO/info/refs?service=SERVICE`: the ref advertisement.
    Refs(Service),
    /// `POST REPO/git-SERVICE`: one exchange of the service.
    Rpc(Service),
}

impl Call {
    /// The service of the call.
    pub fn service(self) -> Service {
        match self {
            Call::Refs(service) | Call::Rpc(service) => service,
        }
    }

    /// The end of `PATH_INFO` after the repository.
    fn tail(self) -> &'static str {
        match self {
            Call::Refs(_) => "info/refs",
            Call::Rpc(Service::ReceivePack) => "git-receive-pack",
            Call::Rpc(_) => "git-upload-pack",
        }
    }
}

/// The service of a value of `service=`.
fn service_named(name: &str) -> Option<Service> {
    match name {
        "git-upload-pack" => Some(Service::UploadPack),
        "git-receive-pack" => Some(Service::ReceivePack),
        _ => None,
    }
}

/// The repository path (decoded) and the call of a request, or `None` for
/// a request that is not part of the smart protocol.
pub fn parse(method: &http::Method, uri: &http::Uri) -> Option<(String, Call)> {
    let path = crate::files::percent_decode_path(uri.path())?;
    if let Some(repo) = path.strip_suffix("/info/refs") {
        if method != http::Method::GET {
            return None;
        }
        let service = uri
            .query()?
            .split('&')
            .find_map(|pair| pair.strip_prefix("service="))
            .and_then(service_named)?;
        return Some((repo.to_string(), Call::Refs(service)));
    }
    if method != http::Method::POST {
        return None;
    }
    let (repo, name) = path.rsplit_once('/')?;
    let service = service_named(name)?;
    Some((repo.to_string(), Call::Rpc(service)))
}

/// The router of git hosting for `options`. A root that is not a
/// directory is an error.
pub fn router(options: HttpOptions) -> Result<Router> {
    if !options.root.is_dir() {
        return Err(podnet_core::exit::fault(
            podnet_core::exit::Fault::Usage,
            anyhow::anyhow!(
                "the repository root {} is not a directory",
                options.root.display()
            ),
        ));
    }
    let root = options.root.canonicalize()?;
    let state = Arc::new(HttpOptions { root, ..options });
    Ok(Router::new()
        .fallback(answer)
        .with_state(state)
        .layer(axum::extract::DefaultBodyLimit::disable()))
}

fn refuse(status: http::StatusCode, message: &str) -> Response {
    (status, format!("{message}\n")).into_response()
}

/// Answer one request.
async fn answer(
    axum::extract::State(options): axum::extract::State<Arc<HttpOptions>>,
    mut request: axum::extract::Request,
) -> Response {
    let Some((repo, call)) = parse(request.method(), request.uri()) else {
        return refuse(http::StatusCode::NOT_FOUND, "Not Found");
    };
    if let Err(err) = super::repo_rel(&repo) {
        tracing::info!(reason = err, "git http: a path is refused");
        return refuse(http::StatusCode::NOT_FOUND, "Not Found");
    }
    let service = call.service();
    let need = if service.writes() {
        Role::Write
    } else {
        Role::Read
    };
    let user = match options.access.grant(&request, need) {
        Ok((_, user)) => user,
        Err(refused) => return *refused,
    };
    let dir = match super::repo_dir(&options.root, &repo) {
        Ok(dir) => dir,
        Err(_) if service.writes() && options.create_on_push => {
            match super::create_repo(&options.root, &repo, &options.git).await {
                Ok(dir) => dir,
                Err(err) => return refuse(http::StatusCode::FORBIDDEN, &err),
            }
        }
        Err(err) => {
            tracing::info!(reason = err, "git http: no repository");
            return refuse(http::StatusCode::NOT_FOUND, "Not Found");
        }
    };
    let Ok(rel) = dir.strip_prefix(&options.root) else {
        return refuse(http::StatusCode::NOT_FOUND, "Not Found");
    };
    let path_info = format!("/{}/{}", rel.display(), call.tail());
    // A value of Git-Protocol that is not valid does not reach git.
    let protocol = request
        .headers()
        .get("git-protocol")
        .and_then(|v| v.to_str().ok())
        .is_some_and(super::protocol_ok);
    if !protocol {
        request.headers_mut().remove("git-protocol");
    }
    let env = vec![
        (
            OsString::from("GIT_PROJECT_ROOT"),
            options.root.clone().into(),
        ),
        (OsString::from("GIT_HTTP_EXPORT_ALL"), OsString::from("1")),
    ];
    let cgi = Cgi {
        program: options.git.clone(),
        args: vec![OsString::from("http-backend")],
        path_info,
        remote_user: user,
        env,
    };
    cgi::run(cgi, request).await
}
