//! The verbs `reconnect` and `rotate` of the link control socket (T-307,
//! GitHub #476, row E10-2), over unix socket pairs: an instance that dialed
//! holds a [`Redial`] that makes the same kind of link again. The process
//! test of the commands is in `link_ctl_plane.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use podnet::hello;
use podnet::identity::{self, AllowList, Identity};
use podnet::link::{self, Settings, Stream};
use podnet::link_ctl::{Client, Control, Hub};
use podnet::link_service;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

fn install(dir: &Path, name: &str) -> Identity {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    identity::create(&sub).unwrap()
}

fn row(doc: &Value, id: u64) -> &Value {
    doc["links"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == id)
        .unwrap_or_else(|| panic!("no link {id}: {doc}"))
}

/// An instance that dialed: a hub with one dialer link over a unix socket
/// pair, and a [`Redial`] that makes the same kind of link again.
struct DialRig {
    hub: Arc<Hub>,
    path: PathBuf,
    token: String,
    peer: String,
    streams: mpsc::UnboundedReceiver<(usize, Stream)>,
    _control: Control,
    _dir: tempfile::TempDir,
}

struct Again {
    me: Identity,
    far: Identity,
    count: std::sync::atomic::AtomicUsize,
    tx: mpsc::UnboundedSender<(usize, Stream)>,
}

impl Again {
    /// One link over a unix pair: hold the dialer end in `hub`, and let the
    /// far end answer each stream and send it on `tx`.
    async fn make(&self, hub: &Arc<Hub>) -> anyhow::Result<u64> {
        let index = self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (dialer, listener) = link::unix_pair(Settings::default())?;
        let list = |k: &Identity| AllowList::parse(&k.fingerprint()).unwrap();
        let (d, l) = tokio::join!(
            hello::dial(&dialer, &self.me, list(&self.far)),
            hello::accept(&listener, &self.far, list(&self.me))
        );
        let (peer_key, _) = (d?, l?);
        let (dialer, listener) = (Arc::new(dialer), Arc::new(listener));
        let slot = hub.hold(
            dialer.clone(),
            peer_key,
            "unix",
            &format!("pair{index}"),
            "dialer",
        );
        let id = slot.id();
        // The slot leaves the hub when the link ends (the task of `link connect`).
        tokio::spawn(async move {
            let _ = dialer.closed().await;
            drop(slot);
        });
        let tx = self.tx.clone();
        tokio::spawn(async move {
            while let Ok(mut stream) = listener.accept().await {
                let tx = tx.clone();
                tokio::spawn(async move {
                    if link_service::accept(&mut stream).await.is_ok()
                        && link_service::reply(&mut stream, link_service::OPEN)
                            .await
                            .is_ok()
                    {
                        let _ = tx.send((index, stream));
                    }
                });
            }
        });
        Ok(id)
    }
}

impl podnet::link_ctl::Redial for Again {
    fn redial<'a>(
        &'a self,
        hub: &'a Arc<Hub>,
        _expect: &'a pod_identity::PublicKey,
    ) -> podnet::link_ctl::Dialing<'a> {
        Box::pin(self.make(hub))
    }
}

impl DialRig {
    async fn new() -> DialRig {
        let dir = tempfile::tempdir().unwrap();
        let (me, far) = (install(dir.path(), "me"), install(dir.path(), "far"));
        let peer = far.fingerprint();
        let hub = Hub::new(&me.fingerprint());
        let path = dir.path().join("ctl.sock");
        let control = Control::start(&path, hub.clone()).unwrap();
        let (tx, streams) = mpsc::unbounded_channel();
        let again = Arc::new(Again {
            me,
            far,
            count: 0.into(),
            tx,
        });
        again.make(&hub).await.unwrap();
        hub.attach_redial(again);
        DialRig {
            hub,
            path,
            token: control.token().to_string(),
            peer,
            streams,
            _control: control,
            _dir: dir,
        }
    }

    async fn call(&self, verb: &str, args: Value) -> anyhow::Result<Value> {
        Client::connect(&self.path)
            .await?
            .call(&self.token, verb, args)
            .await
    }

    async fn open(&mut self) -> (UnixStream, usize, Stream) {
        let conn = Client::connect(&self.path)
            .await
            .unwrap()
            .open(&self.token, &self.peer, "raw")
            .await
            .unwrap();
        let (index, far) = tokio::time::timeout(Duration::from_secs(5), self.streams.recv())
            .await
            .expect("the far end got the stream")
            .unwrap();
        (conn, index, far)
    }

    /// The ids of the links that status lists, until `ok` holds for them.
    async fn links_until(&self, ok: impl Fn(&[u64]) -> bool) -> Vec<u64> {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let doc = self.hub.status();
            let ids: Vec<u64> = doc["links"]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| l["id"].as_u64().unwrap())
                .collect();
            if ok(&ids) || Instant::now() > end {
                return ids;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

#[tokio::test]
async fn reconnect_gives_a_new_link_id_and_a_new_stream_works() {
    let mut rig = DialRig::new().await;
    let (mut old_conn, index, mut old_far) = rig.open().await;
    assert_eq!(index, 0);
    let old_id = u64::from(old_far.id());
    let doc = rig.call("reconnect", json!({"link": 1})).await.unwrap();
    assert_eq!(doc["verb"], "reconnect", "{doc}");
    assert_eq!(doc["link"], 2, "the new link id: {doc}");
    assert_eq!(doc["replaced"], 1, "{doc}");
    assert!(
        doc["reset_streams"]
            .to_string()
            .contains(&old_id.to_string()),
        "the open stream is named: {doc}"
    );
    // The stream of the old session ends with a reset.
    let mut buf = [0u8; 4];
    let end = tokio::time::timeout(Duration::from_secs(5), old_far.read(&mut buf))
        .await
        .expect("the reset reaches the far end");
    assert!(end.is_err(), "a reset is an error: {end:?}");
    let mut rest = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), old_conn.read_to_end(&mut rest)).await;
    assert_eq!(
        rig.links_until(|ids| ids == [2]).await,
        [2],
        "the old link left"
    );
    // A new stream works on the new link.
    let (mut conn, index, mut far) = rig.open().await;
    assert_eq!(index, 1, "the stream is on the new link");
    conn.write_all(b"again").await.unwrap();
    let mut got = [0u8; 5];
    far.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"again");
}

#[tokio::test]
async fn rotate_keeps_an_open_stream_running_until_it_ends_and_puts_a_new_stream_on_the_new_link() {
    let mut rig = DialRig::new().await;
    let (mut conn_a, index, mut far_a) = rig.open().await;
    assert_eq!(index, 0);
    let doc = rig.call("rotate", json!({"link": 1})).await.unwrap();
    assert_eq!(doc["verb"], "rotate", "{doc}");
    assert_eq!(doc["link"], 2, "{doc}");
    assert_eq!(doc["replaced"], 1, "{doc}");
    assert_eq!(doc["selected"], 2, "new streams go to the new link: {doc}");
    assert_eq!(row(&doc, 1)["state"], "up", "the old link stays: {doc}");
    // The stream on the old link still runs, both ways.
    conn_a.write_all(b"before").await.unwrap();
    let mut got = [0u8; 6];
    far_a.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"before");
    far_a.write_all(b"after").await.unwrap();
    let mut back = [0u8; 5];
    conn_a.read_exact(&mut back).await.unwrap();
    assert_eq!(&back, b"after");
    // A new stream is on the new link.
    let (_conn_b, index, _far_b) = rig.open().await;
    assert_eq!(index, 1, "the new stream is on the new link");
    assert_eq!(rig.links_until(|ids| ids.len() != 2).await, [1, 2]);
    // The old link ends when its last stream ends.
    drop(conn_a);
    drop(far_a);
    assert_eq!(
        rig.links_until(|ids| ids == [2]).await,
        [2],
        "the old link left"
    );
}
