use super::*;

mod access;
mod qr;
mod share_link;
mod static_server;

fn req(method: &str, path: &str) -> RequestHead {
    RequestHead {
        method: method.to_string(),
        path: path.to_string(),
        headers: Vec::new(),
    }
}

fn req_with_auth(path: &str, header: &str) -> RequestHead {
    let mut r = req("GET", path);
    r.set_header("Authorization", header);
    r
}

fn tmpdir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("podnet-feature-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    d
}

/// Read the status code out of a rendered response, so a test asserts on
/// the bytes a client would receive rather than on the struct that made
/// them. The two can disagree if a field is dropped on the way to the wire.
fn status_line(bytes: &[u8]) -> u16 {
    String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}
