# Profile `ipv6-only`

The host has no IPv4 route. A connect to an IPv4 address outside fails with
`ENETUNREACH`. IPv6 reaches out. The address `127.0.0.1` stays.

## The data (form of `pod-lab` at `93bf133a`)

```rust
pub(super) const IPV6_ONLY: &[Part] = &[
    net(
        "no-ipv4-route",
        "a TCP connect to an IPv4 address outside fails",
        &[
            needs("a TCP connect to an IPv4 address outside that fails with ENETUNREACH"),
            needs("a TCP connect to 127.0.0.1 that reaches a listener of the lab"),
        ],
    ),
    net(
        "ipv6-reaches",
        "a TCP connect to an IPv6 address outside succeeds",
        &[needs("a TCP connect to an IPv6 address outside")],
    ),
];

// In PROFILES (table.rs), as now:
class("ipv6-only", "no IPv4 route", "podnet", IPV6_ONLY),
```

The change to the table at `v0.56.0`: the errno `ENETUNREACH` and the
loopback that stays, as podnet measured.

## The parts and how a lab holds them

| Part | Observation | No root | User namespace | Root only | Mark |
| --- | --- | --- | --- | --- | --- |
| `no-ipv4-route` | `connect(192.0.2.1:443)` gives `ENETUNREACH`; `connect(127.0.0.1:PORT)` reaches a listener of the lab. | Not possible: seccomp cannot read the address behind the pointer of `connect(2)`, and Landlock rules have no address. | A network namespace: `lo` up, an IPv6 address (`fd00:6c61:62::2/64`) on a dummy or TUN device, an IPv6 default route, no IPv4 route. | Not needed for the observation. | measured (podnet, below) |
| `ipv6-reaches` | A TCP connect to an IPv6 address gets a connection. | Not possible. | A stand-in server at `[fd00:6c61:62::1]:443` in the namespace (on `lo` or behind the TUN). This shows the route, not the outside. | Outside reach: NAT66, or a tunnel out of the namespace (podnet's `scripts/v6only/` uses `podnet warp tun --default-route` in a namespace `v6up` as root). | measured (podnet, below); the lab stand-in: assumed |

## What podnet does in the class

* Measured with the IPv6-only kit (2026-11-02,
  `docs/target-environment.md`, An IPv6-only host): a connect to an IPv4
  address gives `ENETUNREACH`; `podnet warp run` gives IPv4 and UDP to the
  program through WARP; a tunnel with TCP 443 and DNS only is ready in
  13 s. `scripts/v6only/` (`up.sh`, `run.sh`, `acceptance.sh`, `down.sh`)
  makes the host as root on a runner; the step "An IPv6-only host" of
  `.github/workflows/live.yml` runs it.
* On a host with no IPv4 default route, the IPv6 servers of DoH go first
  (`src/resolve.rs`; `pod_dns::route::has_default_route`).
* Direct dials alternate the families and go on to the next address after
  250 ms (`PodUtils/podcrates:crates/pod-proxy/src/race.rs`, `dial_addrs`).
  Inferred. The rows `net-bench` and `net-exit` of the lane row
  `ipv6-host` dial a name with A and AAAA records directly on the host of
  `scripts/v6only/` (`docs/development.md`, The row ipv6-host).
* The lab of `podlab run ipv6-only` has no route out: its IPv6 address
  `fd00:6c61:62::1` is on the loopback of the lab. A row in the lab does
  not reach a real IPv6 network. The row `ipv6-host` runs the rows of the
  profile on a real IPv6-only host, with no lab.

## The commands of podnet in the class

| Command | Expected |
| --- | --- |
| `podnet doctor` | Exit 0 or 1; the IPv4 rows give `ENETUNREACH`, the IPv6 rows run. |
| `podnet tunnel --quick` | Ready over IPv6 (measured with the kit: 13 s). |
| `podnet warp proxy` | MASQUE over IPv6; the program gets IPv4 through WARP. |
