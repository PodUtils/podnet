# Library

```rust
use podnet::{Service, Tunnel};

# async fn run() -> anyhow::Result<()> {
let handle = Tunnel::quick()
    .service(Service::parse("http://127.0.0.1:8080")?)
    .ha_connections(2)
    .build()?
    .start()
    .await?;
println!("{:?}", handle.url());
handle.verify(None).await?;
handle.shutdown().await;
# Ok(())
# }
```

* `podnet::Ingress` holds ordered rules for a tunnel with more than one service.
* `podnet::warp::WarpClient` registers a device and opens a `WarpStack`, which
  has `tcp_connect` and `udp_bind` through the tunnel.
* `podnet::vnet::NetStack` is the userspace IP stack.
* `podnet::identity` keeps the Ed25519 key of an install in the state directory
  (`link-identity.key`, mode 0600) and shows its fingerprint in base32 in five
  groups of eight characters with a checksum. `AllowList` is the list of peers:
  default deny, 4096 entries at most, one fingerprint on each line. A key file
  with a wide mode exits 71, a missing key or list exits 66, and a peer that
  the list refuses exits 77 (`AllowList::check`). The seed is never printed,
  logged or put in JSON (`Identity::report`).
* `podnet::session` carries a link over the resumable layer `pod-session`. A
  link survives the loss of its carrier for 10 minutes (`RESUME_WINDOW`). A
  resume after the window fails with `ResumeExpired`, which exits 69.
* `podnet_netns::main_with_args` runs the command line of `podnet pod`. The modules
  `podnet_netns::upstream`, `socks`, `dns` and `tailscale` are also public.
