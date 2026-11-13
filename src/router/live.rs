//! The doors and the networks of a router that run (T-327). The registry
//! holds each door with its counters and its tasks, and the networks that
//! the control socket added or removed. The verbs that change it are in
//! [`super::change`].

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use podnet_netns::listen::ProxyListen;
use podnet_vnet::door::Door;
use tokio::task::AbortHandle;

use super::door_stats::DoorStats;
use super::{Router, RouterDoor};
use crate::network::{Target, Way};

/// One door of the registry.
pub struct Entry {
    id: u64,
    /// The address that the door listens on (the bound port, for port 0).
    pub listen: String,
    /// The label of the dial of the door.
    pub label: String,
    /// The network or the group of `via`.
    pub via: String,
    /// The rules file of the door, if any.
    pub rules: Option<PathBuf>,
    pub stats: Arc<DoorStats>,
    tasks: Vec<AbortHandle>,
}

/// The first failure of a door of the file: it ends the router.
#[derive(Default)]
struct Failed {
    wake: tokio::sync::Notify,
    text: Mutex<Option<String>>,
}

/// The registry of a router.
pub struct Live {
    /// One change at a time.
    pub(super) ops: tokio::sync::Mutex<()>,
    doors: Mutex<Vec<Entry>>,
    next: Mutex<u64>,
    /// The networks that the control socket added.
    nets: Mutex<HashMap<String, Way>>,
    /// The networks of the file that the control socket removed.
    gone: Mutex<HashSet<String>>,
    /// The `via` and the rules file of each guest of the file.
    guests: Vec<(String, Option<PathBuf>)>,
    failed: Arc<Failed>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Live {
    pub(super) fn new(doors: &[RouterDoor], guests: Vec<(String, Option<PathBuf>)>) -> Live {
        let live = Live {
            ops: Default::default(),
            doors: Default::default(),
            next: Default::default(),
            nets: Default::default(),
            gone: Default::default(),
            guests,
            failed: Default::default(),
        };
        for d in doors {
            live.insert(d);
        }
        live
    }

    /// Put `door` in the registry; the id of the entry.
    pub(super) fn insert(&self, d: &RouterDoor) -> u64 {
        let id = {
            let mut next = lock(&self.next);
            *next += 1;
            *next
        };
        lock(&self.doors).push(Entry {
            id,
            listen: d.listen.to_string(),
            label: d.label.clone(),
            via: d.via.clone(),
            rules: d.rules_path.clone(),
            stats: d.stats.clone(),
            tasks: Vec::new(),
        });
        id
    }

    /// The door `id` listens on `listen` and runs `tasks`.
    pub(super) fn opened(&self, id: u64, listen: String, tasks: Vec<AbortHandle>) {
        if let Some(e) = lock(&self.doors).iter_mut().find(|e| e.id == id) {
            e.listen = listen;
            e.tasks = tasks;
        }
    }

    /// The door that listens on `listen`: remove it and stop its tasks.
    pub(super) fn take(&self, listen: &str) -> Option<Entry> {
        let mut doors = lock(&self.doors);
        let at = doors.iter().position(|e| e.listen == listen)?;
        let entry = doors.remove(at);
        for task in &entry.tasks {
            task.abort();
        }
        Some(entry)
    }

    pub(super) fn has_door(&self, listen: &str) -> bool {
        lock(&self.doors).iter().any(|e| e.listen == listen)
    }

    pub(super) fn drop_id(&self, id: u64) {
        let mut doors = lock(&self.doors);
        if let Some(at) = doors.iter().position(|e| e.id == id) {
            let entry = doors.remove(at);
            for task in &entry.tasks {
                task.abort();
            }
        }
    }

    /// The doors with their counters, for the status document.
    pub(super) fn doors_json(&self) -> Vec<serde_json::Value> {
        lock(&self.doors)
            .iter()
            .map(|e| {
                let mut v = e.stats.json();
                v["listen"] = e.listen.clone().into();
                v["via"] = e.label.clone().into();
                v
            })
            .collect()
    }

    /// What uses the network `name`: a door or a guest, as text.
    pub(super) fn user_of(&self, name: &str) -> Option<String> {
        let names = |path: &Option<PathBuf>| -> bool {
            path.as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .is_some_and(|text| {
                    text.lines()
                        .map(|l| l.split('#').next().unwrap_or(""))
                        .filter_map(|l| l.rsplit_once('='))
                        .any(|(_, way)| way.trim() == name)
                })
        };
        for e in lock(&self.doors).iter() {
            if e.via == name || names(&e.rules) {
                return Some(format!("the door {}", e.listen));
            }
        }
        self.guests
            .iter()
            .position(|(via, rules)| via == name || names(rules))
            .map(|i| format!("the guest {i}"))
    }

    pub(super) fn add_net(&self, name: &str, way: Way) {
        lock(&self.nets).insert(name.to_string(), way);
    }

    /// Forget the network `name` (added or of the file).
    pub(super) fn drop_net(&self, name: &str) {
        if lock(&self.nets).remove(name).is_none() {
            lock(&self.gone).insert(name.to_string());
        }
    }

    pub(super) fn live_names(&self) -> Vec<String> {
        lock(&self.nets).keys().cloned().collect()
    }

    pub(super) fn live_way(&self, name: &str) -> Option<Way> {
        lock(&self.nets).get(name).cloned()
    }

    pub(super) fn is_gone(&self, name: &str) -> bool {
        lock(&self.gone).contains(name)
    }

    /// Run `serve` as a task of door `id`. A door of the file that fails
    /// ends the router; a door that was added only leaves the registry.
    pub(super) fn spawn(
        self: &Arc<Self>,
        id: u64,
        file: bool,
        serve: impl std::future::Future<Output = Result<()>> + Send + 'static,
    ) -> AbortHandle {
        let live = self.clone();
        tokio::spawn(async move {
            let Err(e) = serve.await else { return };
            if file {
                *lock(&live.failed.text) = Some(format!("{e:#}"));
                live.failed.wake.notify_one();
            } else {
                eprintln!("podnet: route  door stopped: {e:#}");
                live.drop_id(id);
            }
        })
        .abort_handle()
    }
}

pub(super) type Serving = std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>;

/// Bind `listen` and give back what serves it: the bound address and the
/// future of the accept loop.
pub(super) async fn bind(
    listen: &ProxyListen,
    door: Arc<Door>,
    replace: bool,
) -> Result<(ProxyListen, Serving)> {
    match listen {
        ProxyListen::Tcp(addr) => {
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .with_context(|| format!("door {addr}"))
                .map_err(crate::net::bind_fault)?;
            let bound = ProxyListen::Tcp(listener.local_addr()?);
            Ok((bound, Box::pin(door.serve_tcp(listener))))
        }
        ProxyListen::Unix(path) => {
            crate::net::claim_socket_path(path, replace)?;
            let listener = podnet_netns::listen::bind_private(path)
                .with_context(|| format!("door unix:{}", path.display()))
                .map_err(crate::net::bind_fault)?;
            Ok((listen_clone(listen), Box::pin(door.serve_unix(listener))))
        }
    }
}

fn listen_clone(l: &ProxyListen) -> ProxyListen {
    match l {
        ProxyListen::Tcp(a) => ProxyListen::Tcp(*a),
        ProxyListen::Unix(p) => ProxyListen::Unix(p.clone()),
    }
}

impl Router {
    /// Bind each door of the file and
    /// serve each door, and its DNS door, on tasks. A bind error is the error
    /// of the call. Print a line for each door.
    pub async fn open_doors(&self, replace: bool) -> Result<()> {
        let ids: Vec<u64> = {
            let doors = lock(&self.live.doors);
            doors.iter().map(|e| e.id).collect()
        };
        for (d, id) in self.doors.iter().zip(ids) {
            let (bound, serve) = bind(&d.listen, d.door.clone(), replace).await?;
            println!("podnet: route  door {bound}  via {}", d.label);
            let mut tasks = vec![self.live.spawn(id, true, serve)];
            tasks.extend(self.start_dns(id, true, d));
            self.live.opened(id, bound.to_string(), tasks);
        }
        Ok(())
    }

    /// Serve the DNS door of `d`, if it has one.
    pub(super) fn start_dns(&self, id: u64, file: bool, d: &RouterDoor) -> Option<AbortHandle> {
        let (addr, upstream): &(std::net::SocketAddr, Target) = d.dns.as_ref()?;
        println!("podnet: route  dns {addr}  via {}  to {upstream}", d.label);
        let serve = crate::forwards::serve_dns(*addr, d.dial.clone(), upstream.clone());
        Some(self.live.spawn(id, file, serve))
    }

    /// Resolve when a door of the file stops: the error that ends the router.
    pub async fn door_failure(&self) -> anyhow::Error {
        loop {
            self.live.failed.wake.notified().await;
            if let Some(text) = lock(&self.live.failed.text).clone() {
                return anyhow::anyhow!(text).context("a door stopped");
            }
        }
    }
}
