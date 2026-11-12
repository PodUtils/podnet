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
* `podnet_netns::main_with_args` runs the command line of `podnet pod`. The modules
  `podnet_netns::upstream`, `socks`, `dns` and `tailscale` are also public.
