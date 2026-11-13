//! T-241: the registration of a WARP device holds a lock, saves the device
//! before the enrolment step, and resumes. A fake API on loopback stands in
//! for the service.

mod test_host;

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use podnet::warp::{Identity, WarpClient};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// The tests share the variable `PODNET_WARP_API`: one at a time.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Default)]
struct Fake {
    /// `METHOD /path` of each request, in order.
    log: Mutex<Vec<String>>,
    posts: AtomicUsize,
    /// The statuses of the next PATCH requests (200 when empty).
    patch: Mutex<Vec<u16>>,
    /// A PATCH waits for ever while this is set.
    hang: AtomicBool,
}

impl Fake {
    fn count(&self, what: &str) -> usize {
        let log = self.log.lock().unwrap();
        log.iter().filter(|l| l.starts_with(what)).count()
    }
}

fn device(id: &str, tunnel: &str) -> String {
    serde_json::json!({
        "id": id, "token": "tok", "tunnel_type": tunnel,
        "account": {"id": "acc"},
        "config": {"peers": [{"public_key": "k", "endpoint": {"v4": "162.159.198.2:0"}}]}
    })
    .to_string()
}

async fn serve(fake: Arc<Fake>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let fake = fake.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).to_string();
                let mut words = head.split_whitespace();
                let method = words.next().unwrap_or("").to_string();
                let path = words.next().unwrap_or("").to_string();
                fake.log.lock().unwrap().push(format!("{method} {path}"));
                let id = path.rsplit('/').next().unwrap_or("").to_string();
                let (status, body) = match method.as_str() {
                    "POST" => {
                        let n = fake.posts.fetch_add(1, Ordering::SeqCst) + 1;
                        (200, device(&format!("dev-{n}"), "wireguard"))
                    }
                    "PATCH" => {
                        while fake.hang.load(Ordering::SeqCst) {
                            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        }
                        let mut queue = fake.patch.lock().unwrap();
                        let status = if queue.is_empty() {
                            200
                        } else {
                            queue.remove(0)
                        };
                        let body = if status == 200 {
                            device(&id, "masque")
                        } else {
                            "no".into()
                        };
                        (status, body)
                    }
                    _ => (204, String::new()),
                };
                let text = format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(text.as_bytes()).await;
            });
        }
    });
    base
}

fn stored(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// A record with the state `state` and the device `id`.
fn plant(path: &Path, id: &str, state: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    let (cert_pem, key_pem) = Identity::generate().unwrap().to_pem();
    let record = serde_json::json!({
        "device": serde_json::from_str::<serde_json::Value>(&device(id, "wireguard")).unwrap(),
        "cert_pem": cert_pem, "key_pem": key_pem.as_str(), "state": state,
    });
    std::fs::write(path, record.to_string()).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[tokio::test]
async fn two_register_or_load_calls_make_one_post() {
    test_host::need_loopback!();
    let _one = ONE_AT_A_TIME.lock().await;
    let fake = Arc::new(Fake::default());
    std::env::set_var("PODNET_WARP_API", serve(fake.clone()).await);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("warp.json");
    let (a, b) = tokio::join!(
        WarpClient::register_or_load(&path),
        WarpClient::register_or_load(&path)
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(fake.count("POST"), 1, "{:?}", fake.log.lock().unwrap());
    assert_eq!(fake.count("PATCH"), 1);
    assert_eq!(a.device().id, b.device().id);
    assert_eq!(stored(&path)["device"]["tunnel_type"], "masque");
    assert!(stored(&path).get("state").is_none());
    assert!(dir.path().join("warp.json.lock").exists());
}

/// The child of the kill test: it registers against the fake API of the
/// parent, whose PATCH never answers.
#[tokio::test]
async fn child_for_the_kill_test() {
    let (Ok(path), Ok(base)) = (
        std::env::var("T241_CHILD_PATH"),
        std::env::var("T241_CHILD_API"),
    ) else {
        return;
    };
    std::env::set_var("PODNET_WARP_API", base);
    let _ = WarpClient::register_or_load(Path::new(&path)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_kill_after_the_post_resumes_with_a_patch() {
    test_host::need_loopback!();
    let _one = ONE_AT_A_TIME.lock().await;
    let fake = Arc::new(Fake::default());
    fake.hang.store(true, Ordering::SeqCst);
    let base = serve(fake.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("warp.json");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_for_the_kill_test", "--nocapture"])
        .env("T241_CHILD_PATH", &path)
        .env("T241_CHILD_API", &base)
        .spawn()
        .unwrap();
    for _ in 0..500 {
        if fake.count("PATCH") == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(fake.count("PATCH"), 1, "the child sent no PATCH");
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(stored(&path)["state"], "created");
    assert_eq!(stored(&path)["device"]["id"], "dev-1");
    // The next run.
    fake.hang.store(false, Ordering::SeqCst);
    std::env::set_var("PODNET_WARP_API", &base);
    let client = WarpClient::register_or_load(&path).await.unwrap();
    assert_eq!(fake.count("POST"), 1, "no second POST");
    assert_eq!(fake.count("PATCH"), 2);
    assert!(fake.log.lock().unwrap()[2].ends_with("/reg/dev-1"));
    assert_eq!(client.device().id, "dev-1");
    assert_eq!(stored(&path)["device"]["tunnel_type"], "masque");
    assert!(stored(&path).get("state").is_none());
}

#[tokio::test]
async fn a_patch_404_registers_anew() {
    test_host::need_loopback!();
    let _one = ONE_AT_A_TIME.lock().await;
    let fake = Arc::new(Fake::default());
    fake.patch.lock().unwrap().push(404);
    std::env::set_var("PODNET_WARP_API", serve(fake.clone()).await);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("warp.json");
    plant(&path, "gone", "created");
    let client = WarpClient::register_or_load(&path).await.unwrap();
    assert_eq!(fake.count("POST"), 1);
    assert_eq!(fake.count("PATCH"), 2);
    assert_eq!(client.device().id, "dev-1");
    assert_eq!(stored(&path)["device"]["id"], "dev-1");
    assert_eq!(stored(&path)["device"]["tunnel_type"], "masque");
}

#[tokio::test]
async fn a_failed_patch_deletes_the_new_device() {
    test_host::need_loopback!();
    let _one = ONE_AT_A_TIME.lock().await;
    let fake = Arc::new(Fake::default());
    fake.patch.lock().unwrap().push(400);
    std::env::set_var("PODNET_WARP_API", serve(fake.clone()).await);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("warp.json");
    assert!(WarpClient::register_or_load(&path).await.is_err());
    assert_eq!(fake.count("DELETE"), 1, "{:?}", fake.log.lock().unwrap());
    assert!(!path.exists());
}

#[tokio::test]
async fn a_partial_file_with_an_unknown_state_is_refused() {
    test_host::need_loopback!();
    let _one = ONE_AT_A_TIME.lock().await;
    let fake = Arc::new(Fake::default());
    std::env::set_var("PODNET_WARP_API", serve(fake.clone()).await);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("warp.json");
    plant(&path, "x", "half-done");
    let err = WarpClient::register_or_load(&path).await.err().unwrap();
    assert!(format!("{err:#}").contains("half-done"), "{err:#}");
    assert!(WarpClient::load(&path).is_err());
    assert!(fake.log.lock().unwrap().is_empty());
}
