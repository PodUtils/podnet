//! The verbs of the control socket that change the doors and the networks of
//! a router that runs (T-327): `add-door`, `remove-door`, `add-network` and
//! `remove-network`. The flows that run go on. A door that is removed stops
//! its listener; its open flows end at their own end.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::control::Answer;
use super::{DoorConfig, Router, RouterDoor};
use crate::network::{Swap, Way};
use podnet_netns::listen::ProxyListen;

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl Router {
    /// The way named `name`: a network that was added, a network or a group
    /// of the file that was not removed.
    fn way_named(&self, name: &str) -> Option<Way> {
        self.live.live_way(name).or_else(|| {
            if self.live.is_gone(name) {
                None
            } else {
                self.ways.get(name).cloned()
            }
        })
    }

    /// Each way that a door can name now.
    fn ways_now(&self) -> HashMap<String, Way> {
        let mut ways: HashMap<String, Way> = self
            .ways
            .iter()
            .filter(|(n, _)| !self.live.is_gone(n))
            .map(|(n, w)| (n.clone(), w.clone()))
            .collect();
        for name in self.live.live_names() {
            if let Some(w) = self.live.live_way(&name) {
                ways.insert(name, w);
            }
        }
        ways
    }

    /// Add a door: the keys of a door of the file. The door listens at once.
    pub(super) async fn add_door(&self, body: Value) -> Result<(), Answer> {
        let config: DoorConfig =
            serde_json::from_value(body).map_err(|e| Answer::new(400, format!("the door: {e}")))?;
        let _one = self.live.ops.lock().await;
        if self.way_named(&config.via).is_none() {
            return Err(Answer::new(
                404,
                format!("via: {:?} is not a network or a group", config.via),
            ));
        }
        let door = RouterDoor::build("door", &config, &self.ways_now())
            .map_err(|e| Answer::new(400, format!("{e:#}")))?;
        if self.live.has_door(&door.listen.to_string()) {
            return Err(Answer::new(
                409,
                format!("{}: a door listens here", door.listen),
            ));
        }
        let id = self.live.insert(&door);
        let opened = super::live::bind(&door.listen, door.door.clone(), false).await;
        let (bound, serve) = match opened {
            Ok(opened) => opened,
            Err(e) => {
                self.live.drop_id(id);
                return Err(Answer::new(502, format!("{e:#}")));
            }
        };
        let mut tasks = vec![self.live.spawn(id, false, serve)];
        tasks.extend(self.start_dns(id, false, &door));
        self.live.opened(id, bound.to_string(), tasks);
        eprintln!("podnet: route  add-door {bound}  via {}", door.label);
        Ok(())
    }

    /// Remove the door that listens on `listen`.
    pub(super) async fn remove_door(&self, listen: &str) -> Result<(), Answer> {
        let parsed = ProxyListen::parse(listen)
            .map_err(|e| Answer::new(400, format!("listen: {e}")))?
            .to_string();
        let _one = self.live.ops.lock().await;
        let entry = self
            .live
            .take(&parsed)
            .ok_or_else(|| Answer::new(404, format!("{parsed}: no door listens here")))?;
        if let Ok(ProxyListen::Unix(path)) = ProxyListen::parse(&parsed) {
            use std::os::unix::fs::FileTypeExt as _;
            if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_socket()) {
                let _ = std::fs::remove_file(&path);
            }
        }
        eprintln!(
            "podnet: route  remove-door {parsed}  flows {}  bytes {}",
            entry.stats.flows(),
            entry.stats.bytes()
        );
        Ok(())
    }

    /// Add the network `name` with `spec` (see `egress.md`).
    pub(super) async fn add_network(&self, name: &str, spec: &str) -> Result<(), Answer> {
        if !valid_name(name) {
            return Err(Answer::new(
                400,
                format!("the name {name:?}: letters, digits, - and _"),
            ));
        }
        let _one = self.live.ops.lock().await;
        if self.way_named(name).is_some() || self.groups.iter().any(|(g, _)| g == name) {
            return Err(Answer::new(
                409,
                format!("{name}: a network or a group has this name"),
            ));
        }
        let network = self
            .control
            .connect(spec)
            .map_err(|e| Answer::new(400, format!("{name}: {}", e.text())))?;
        let ctx = self.control.ctx_of(name)?;
        let mut link = network
            .up(&ctx)
            .await
            .map_err(|e| Answer::new(502, format!("{name}: {e:#}")))?;
        let Some(way) = link.way.take() else {
            link.close().await;
            return Err(Answer::new(
                502,
                format!("{name}: the network gives no way out"),
            ));
        };
        let swap = Arc::new(Swap::new(way.dial));
        self.live.add_net(
            name,
            Way {
                dial: swap.clone(),
                addressing: way.addressing,
            },
        );
        self.control.add_network(name, spec, swap, link).await;
        eprintln!("podnet: route  add-network {name}");
        Ok(())
    }

    /// Remove the network `name`. A door, a guest or a group that uses it
    /// stops the removal (409).
    pub(super) async fn remove_network(&self, name: &str) -> Result<(), Answer> {
        let _one = self.live.ops.lock().await;
        if self.way_named(name).is_none() || self.groups.iter().any(|(g, _)| g == name) {
            return Err(Answer::new(404, format!("{name:?} is not a network")));
        }
        if let Some((group, _)) = self
            .groups
            .iter()
            .find(|(_, g)| g.members().iter().any(|m| m.name == name))
        {
            return Err(Answer::new(
                409,
                format!("{name}: the group {group} uses it"),
            ));
        }
        if let Some(user) = self.live.user_of(name) {
            return Err(Answer::new(409, format!("{name}: {user} uses it")));
        }
        self.live.drop_net(name);
        self.control.remove_network(name).await;
        eprintln!("podnet: route  remove-network {name}");
        Ok(())
    }
}
