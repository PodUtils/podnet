//! The routes and the resolver of `podnet warp tun --default-route` (issue
//! 77): `pod-warp`'s routes over the routes of the host, with podnet's
//! resolvers, its marker of the resolver file and its state directory.
//!
//! * Host routes through the gateway of the default route for each
//!   address that the MASQUE connection uses (the proxy, the front door,
//!   the relay), so that the connection does not go into the tunnel.
//! * Default routes on the TUN: `0.0.0.0/1` and `128.0.0.0/1`, and
//!   `::/1` and `8000::/1`. They go away while the session is down, so
//!   that traffic fails fast.
//! * DNS: `resolvectl` on the TUN when systemd-resolved runs, else
//!   `/etc/resolv.conf` with 1.1.1.1 and 1.0.0.1 (a copy is kept).
//!
//! [`Routes`] puts each change back when it is dropped.

use anyhow::Result;

use super::Warped as _;

pub use pod_warp::route::{
    nameservers_of_service, parse_entries, parse_entries6, parse_route_table, plan_exempt,
    plan_exempt6, Entry, Entry6, Exempt, Exempt6, Gateway,
};

/// The resolver file of the host: its record and its lock in podnet's
/// state directory, and podnet's marker.
pub mod resolv {
    use anyhow::Result;

    pub use pod_warp::route::resolv::{replace_file, set_file, undo, Dns, Paths, RESOLV};

    /// The first line of the resolver file that podnet writes.
    pub const MARKER: &str = "# podnet warp tun --default-route: DNS through WARP";

    /// `/etc/resolv.conf` and the files in the state directory (else the
    /// run directory), with podnet's marker.
    pub fn system() -> Result<Paths> {
        let dir = match crate::state::state_dir()? {
            Some(dir) => dir,
            None => crate::instances::run_dir()?,
        };
        let mut paths = Paths::in_dir(RESOLV.into(), &dir);
        paths.marker = MARKER.into();
        Ok(paths)
    }

    /// Restore a record of a process that ended, when no live tun holds the
    /// lock.
    pub fn reconcile(paths: &Paths) -> Result<()> {
        pod_warp::route::resolv::reconcile(paths).map_err(super::super::failed)
    }

    /// [`reconcile`] for `/etc/resolv.conf`; a failure is a message.
    pub fn reconcile_system() {
        if let Err(err) = system().and_then(|paths| reconcile(&paths)) {
            eprintln!("podnet: the resolver file is not restored: {err:#}");
        }
    }
}

/// The nameservers of `--default-route`: the UDP resolvers of `--resolver`
/// on port 53 (at most 3; issue 145), else 1.1.1.1 and 1.0.0.1.
pub fn nameservers() -> Vec<std::net::IpAddr> {
    pod_warp::route::nameservers(&super::proxy::resolver_list())
}

/// The routes and the resolver of `--default-route`; dropped, it puts each
/// change back.
#[derive(Debug)]
pub struct Routes(pod_warp::route::Routes);

impl Routes {
    /// Add a host route for each address of `required` and `optional` that
    /// the default routes of the TUN would capture, and set the resolver.
    /// The default routes come with [`Routes::up`]. An address of
    /// `required` with no route is an error; one of `optional` is skipped.
    pub fn setup(
        tun: &str,
        required: Vec<std::net::IpAddr>,
        optional: Vec<std::net::IpAddr>,
        ipv6: bool,
    ) -> Result<Self> {
        let tables = pod_warp::route::Tables::system();
        let mut routes =
            pod_warp::route::Routes::setup(tun, required, optional, ipv6, &tables).warped()?;
        routes
            .set_dns(&nameservers(), &resolv::system()?)
            .warped()?;
        Ok(Routes(routes))
    }

    /// The addresses that have a host route outside the tunnel.
    pub fn exempted(&self) -> Vec<std::net::IpAddr> {
        self.0.exempted()
    }

    /// The default routes on the TUN (the session is up).
    pub fn up(&mut self) -> Result<()> {
        self.0.up().warped()
    }

    /// No default routes on the TUN (the session is down: traffic fails
    /// fast).
    pub fn down(&mut self) {
        self.0.down()
    }
}
