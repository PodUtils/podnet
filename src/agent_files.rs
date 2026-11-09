//! The file hosts of the agent doctor (T-182). An index names the files of a
//! package; a host that reaches the index can still miss the host of the
//! files. The host of the files comes from an answer of the index (docker,
//! cargo) or from `pod-services` (pip). The doctor holds no host of its own
//! for them.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context as _, Result};
use pod_core::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};
use pod_proxy::client::Request;
use serde_json::Value;
use tokio::time::Instant;

use crate::agent::Path;

/// Where the host of the files of a task comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Files {
    /// The config of a sparse index of cargo: its `dl` key names the host.
    Cargo,
    /// The redirect of one blob request of a registry of docker.
    Docker,
    /// The endpoint `.1` of the service `.0` of `pod-services`.
    Service(&'static str, &'static str),
}

/// The repository of the blob request of [`Files::Docker`]: a public image.
const DOCKER_REPOSITORY: &str = "library/hello-world";
/// The types of manifest that the registry can give.
const MANIFEST_TYPES: &str = "application/vnd.oci.image.index.v1+json, \
application/vnd.docker.distribution.manifest.list.v2+json, \
application/vnd.oci.image.manifest.v1+json, \
application/vnd.docker.distribution.manifest.v2+json";

/// The way out of one path of the doctor, as a dial of the HTTP client.
struct PathDial(Path);

impl Dial for PathDial {
    fn label(&self) -> String {
        format!("{:?}", self.0).to_lowercase()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let limit = deadline.saturating_duration_since(Instant::now());
            match crate::agent::dial(self.0, &target.host(), target.port(), limit).await {
                Ok(io) => Ok(Connected::new(Box::new(io))),
                Err(e) => Err(Failure::caused(
                    Stage::Connect,
                    Class::Unavailable,
                    format!("{e:#}"),
                    e,
                )),
            }
        })
    }
}

/// The host and the port of the files of a task, over `path`. `index` is the
/// host and the port of the index.
pub async fn host(
    files: Files,
    index: (&str, u16),
    path: Path,
    timeout: Duration,
) -> Result<(String, u16)> {
    if let Files::Service(id, name) = files {
        let endpoint = crate::services::endpoint(id, name);
        return Ok((endpoint.host().to_string(), endpoint.port()));
    }
    let client = pod_proxy::Client::new(crate::trust::context().clone(), Arc::new(PathDial(path)))
        .with_keep_alive(false)
        .with_user_agent(format!("podnet/{}", env!("CARGO_PKG_VERSION")));
    let ask = Asker { client, timeout };
    let work = async {
        match files {
            Files::Cargo => cargo(&ask, index).await,
            _ => docker(&ask, index).await,
        }
    };
    tokio::time::timeout(timeout.saturating_mul(5), work)
        .await
        .map_err(|_| anyhow::anyhow!("no answer in {} s", timeout.as_secs() * 5))?
}

struct Asker {
    client: pod_proxy::Client,
    timeout: Duration,
}

impl Asker {
    async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<pod_proxy::http::Response> {
        let mut request = Request::new("GET", url)
            .timeout(self.timeout)
            .max_body(1 << 20);
        for (name, value) in headers {
            request = request.header(name, *value);
        }
        self.client
            .send(request)
            .await
            .map_err(|failure| anyhow::anyhow!("{failure}"))
    }
}

fn authority(host: &str, port: u16) -> String {
    if port == 443 {
        host.to_string()
    } else {
        format!("{host}:{port}")
    }
}

/// The host and the port of the `https://` URL `url`.
pub fn host_of(url: &str) -> Option<(String, u16)> {
    let rest = url.strip_prefix("https://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = rest[..end].rsplit('@').next()?;
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && !port.ends_with(']') => {
            (host, port.parse().ok()?)
        }
        _ => (authority, 443),
    };
    let host = host.trim_matches(['[', ']']);
    (!host.is_empty()).then(|| (host.to_string(), port))
}

async fn cargo(ask: &Asker, index: (&str, u16)) -> Result<(String, u16)> {
    let url = format!("https://{}/config.json", authority(index.0, index.1));
    let answer = ask.get(&url, &[]).await?;
    if !answer.is_success() {
        bail!("{url}: HTTP {}", answer.status);
    }
    let config: Value = serde_json::from_slice(&answer.body).context("config.json")?;
    let dl = config["dl"].as_str().context("config.json has no dl key")?;
    host_of(dl).with_context(|| format!("the dl key {dl:?} is not an https URL"))
}

/// The value of `key` in a `Www-Authenticate: Bearer key="value",...` header.
fn challenge_value<'a>(header: &'a str, key: &str) -> Option<&'a str> {
    let at = header.find(&format!("{key}=\""))? + key.len() + 2;
    let rest = &header[at..];
    Some(&rest[..rest.find('"')?])
}

async fn docker(ask: &Asker, index: (&str, u16)) -> Result<(String, u16)> {
    let base = format!("https://{}", authority(index.0, index.1));
    let ping = ask.get(&format!("{base}/v2/"), &[]).await?;
    let header = ping.header("www-authenticate").unwrap_or_default();
    let mut bearer = Vec::new();
    if ping.status == 401 {
        let realm =
            challenge_value(header, "realm").context("the registry names no token realm")?;
        let service = challenge_value(header, "service").unwrap_or_default();
        let scope = format!("repository:{DOCKER_REPOSITORY}:pull");
        let token = ask
            .get(&format!("{realm}?service={service}&scope={scope}"), &[])
            .await?;
        let token: Value = serde_json::from_slice(&token.body).context("the token answer")?;
        let token = token["token"]
            .as_str()
            .context("the token answer has no token")?;
        bearer.push(("Authorization", format!("Bearer {token}")));
    }
    let with = |accept: &'static str| -> Vec<(&str, &str)> {
        let mut headers = vec![("Accept", accept)];
        headers.extend(bearer.iter().map(|(n, v)| (*n, v.as_str())));
        headers
    };
    let manifest = |reference: String, accept: &'static str| {
        let url = format!("{base}/v2/{DOCKER_REPOSITORY}/manifests/{reference}");
        let headers = with(accept);
        async move {
            let answer = ask.get(&url, &headers).await?;
            if !answer.is_success() {
                bail!("{url}: HTTP {}", answer.status);
            }
            serde_json::from_slice::<Value>(&answer.body).context("a manifest")
        }
    };
    let mut found = manifest("latest".into(), MANIFEST_TYPES).await?;
    if let Some(list) = found["manifests"].as_array() {
        let pick = list
            .iter()
            .find(|m| m["platform"]["architecture"] == "amd64" && m["platform"]["os"] == "linux")
            .or_else(|| list.first())
            .context("an empty manifest list")?;
        let digest = pick["digest"]
            .as_str()
            .context("a manifest with no digest")?;
        found = manifest(digest.to_string(), MANIFEST_TYPES).await?;
    }
    let digest = found["layers"][0]["digest"]
        .as_str()
        .or_else(|| found["config"]["digest"].as_str())
        .context("the manifest names no blob")?;
    let url = format!("{base}/v2/{DOCKER_REPOSITORY}/blobs/{digest}");
    let blob = ask.get(&url, &with("*/*")).await?;
    match blob.status {
        301 | 302 | 303 | 307 | 308 => {
            let location = blob
                .header("location")
                .context("a redirect with no Location")?;
            host_of(location).with_context(|| format!("the redirect {location:?} is not https"))
        }
        // The registry serves the blob itself: the index host is the files host.
        200..=299 => Ok((index.0.to_string(), index.1)),
        status => bail!("{url}: HTTP {status}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_of_an_url() {
        let of = |url| host_of(url);
        assert_eq!(
            of("https://static.crates.io/crates"),
            Some(("static.crates.io".into(), 443))
        );
        assert_eq!(of("https://a.test:8443/x?y"), Some(("a.test".into(), 8443)));
        assert_eq!(of("https://u@a.test"), Some(("a.test".into(), 443)));
        assert_eq!(of("https://[::1]:9/x"), Some(("::1".into(), 9)));
        assert_eq!(of("http://a.test/"), None);
    }

    #[test]
    fn a_challenge_names_its_realm_and_service() {
        let header = r#"Bearer realm="https://auth.test/token",service="registry.test""#;
        assert_eq!(
            challenge_value(header, "realm"),
            Some("https://auth.test/token")
        );
        assert_eq!(challenge_value(header, "service"), Some("registry.test"));
        assert_eq!(challenge_value(header, "scope"), None);
    }
}
