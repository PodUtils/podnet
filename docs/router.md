# Router: `podnet route`

`podnet route --config FILE` runs a router from a YAML file. The router has networks, groups of
networks, health probes, policies and doors. A door is a SOCKS5 and HTTP CONNECT listener. A guest wire lets a guest use the router as its gateway. Each flow
that comes to a door goes out through the network or the group that the door names. The kernel mode
writes policy routing for WANs, with no `ip` program.

The router needs no root, no TUN device and no namespace, except the kernel mode, which needs
`CAP_NET_ADMIN`. A guest wire (below) needs a unix socket, or a TUN device that exists.

```sh
podnet route --config router.yml                        # run in the foreground
podnet route --config router.yml --detach --name edge   # run in the background
podnet route --config router.yml --check                # read the file and bring up nothing
podnet route --config router.yml --check --to example.com:443   # a dry run for one target
podnet route --status unix:/run/r.ctl                   # the state of a running router
podnet route --status unix:/run/r.ctl --json
podnet route --status unix:/run/r.ctl --metrics         # the rows of the metrics (Prometheus text)
podnet route --control unix:/run/r.ctl --select egress=proxy    # pin a group to one member
podnet route --control unix:/run/r.ctl --select egress=auto     # give the choice back to the policy
podnet route --control unix:/run/r.ctl --rotate proxy           # a new identity of a network
podnet route --control unix:/run/r.ctl --reconnect proxy        # a new link, the same identity
podnet route --control unix:/run/r.ctl --interval 5             # seconds between two probes
podnet route --control unix:/run/r.ctl --add-door '{"listen":"tcp://127.0.0.1:0","via":"egress"}'
podnet route --control unix:/run/r.ctl --remove-door tcp://127.0.0.1:1081
podnet route --control unix:/run/r.ctl --add-network lab=socks5h://127.0.0.1:1090
podnet route --control unix:/run/r.ctl --remove-network lab
podnet route --config router.yml --export mwan-json     # the WANs of the kernel section as JSON
```

## The file

```yaml
networks:                 # NAME: SPEC (see egress.md)
  home: direct
  proxy: socks5h://127.0.0.1:1080
groups:                   # NAME: MEMBER*WEIGHT, MEMBER*WEIGHT, ...
  egress: home*2, proxy*1
policies:                 # optional, NAME: POLICY of a group
  egress: minrtt
splits:                   # optional, NAME: up=MEMBER down=MEMBER of a group
  egress: up=home down=proxy
health:                   # the probe of the members of each group
  target: 1.1.1.1:443
  interval: 10
  down_after: 4
  up_after: 3
  cooldown: 30
  timeout: 5
  window: 8
  egress:                 # optional: the egress check of a network (see below)
    proxy: 203.0.113.9:7=198.51.100.4
resolvers:                # optional: NETWORK: HOST:PORT, a DNS server at the far end of the network
  proxy: 10.0.0.53:53
control: unix:/run/r.ctl  # optional: the socket of `route --status` and `--control`
state_dir: /var/lib/r     # optional: DIR/NETWORK holds the identity of a network
doors:
  - listen: unix:/run/r.sock    # HOST:PORT, tcp://HOST:PORT or unix:/PATH
    via: egress                 # a network or a group of the file
    rules: /etc/r.rules         # optional: rules that go before `via`
    allow_shadowed_rules: false
    dns: 127.0.0.1:5353         # optional: a DNS door over the same way
    dns_upstream: 10.0.0.53:53  # the resolver at the far end (needed with dns)
```

* The file uses the YAML reader of podnet (block mappings and sequences, comments; no flow collections
  except `[]` and `{}`). An unknown key is a usage error (exit 64, with or without `--check`), and the
  error names the key or the line.
* A name is letters, digits, `-` and `_`. A group and a network cannot have the same name.
* `networks` maps a name to a network spec (see [egress.md](egress.md)): `direct`, a proxy URL, `relay`,
  `tailscale:/PATH`, `netbird:[PORT]`, `wg:FILE`, `turn://HOST[:PORT]` (UDP only) or `unix:/PATH`. A `tailscale:/PATH` network is a member of a group
  and the `via` of a door, as each other network. When `tailscaled` is not `Running` at the start, the
  router prints one warning and goes on: each dial through that network fails until the daemon is logged
  in.
* `groups` maps a name to a list of members. A member is a network name with an optional weight
  (`home*2`). The default weight is 1.
* The file needs one door or more, unless it has a `guests` entry or a `kernel` section. A file with `kernel` needs no door.
* A flow takes one member and never moves to another. A group balances flows, not the bytes of one flow: one
  flow never uses the bandwidth of two members (no bonding).

## Doors

* A door accepts SOCKS5 (CONNECT and UDP ASSOCIATE) and HTTP CONNECT.
* UDP ASSOCIATE (RFC 1928) carries the datagrams of a client over the datagrams of the network or the
  group of the door. The door answers with a UDP socket on its own address (the address of `listen`;
  the loopback address for a `unix:` door). The first datagram fixes the source of the client; a datagram
  of another source and a fragment are dropped. An answer goes back only from an address that the client
  sent to. The association ends when the client closes the control connection, or after 300 s with no
  datagram. A name in a datagram resolves with the resolver of this host (the datagrams of a network take
  an address). The reply is 0x07 (command not supported) when the network carries no UDP (a SOCKS5 or
  HTTP proxy), and 0x02 (not allowed) when the group has no member that takes flows. The bytes count
  for the member that carries the socket. The rules of the door choose by the target of a CONNECT only:
  a UDP association goes over `via`.
* `via` names the network or the group for a target that no rule matches.
* `rules` is a rules file. A rule is `domain:NAME=WAY`, `domain:.NAME=WAY` (the name and each name under
  it), `cidr:NET=WAY` or `cidr:@FILE=WAY` (one prefix on each line of FILE), `port:N=WAY` or `port:N-M=WAY`
  (these ports of each host), or `tcp:HOST:PORT=WAY` (one host and one port: a name, an IPv4 address or
  `[IPv6]`; `tcp:127.0.0.1:5432=home` sends a CONNECT to that port of the loopback to `home`, and a CONNECT
  to another loopback port does not match). A rule of a port matches a TCP connection only; a UDP flow
  or association has no port for the rules. `WAY` is a network or a
  group of the router, or `reject` (or `block`). The first rule that matches wins. `#` starts a comment.
  podnet reads the file again when it changes. A rule that an earlier rule shadows gives a warning; a
  shadowed `reject` rule is an error, and the file does not load unless the door has
  `allow_shadowed_rules: true`.
* `dns` and `dns_upstream` start a DNS door (UDP and TCP) on `dns`. It sends each query over the same
  way as the door, to the resolver `dns_upstream` at the far end of the way. The router names no resolver
  of its own, so give both keys or none.
* `resolvers` gives a network a DNS server at the far end of the network: `NETWORK: HOST:PORT`. A name
  that a flow brings to a member of that network resolves there, with DNS over TCP (RFC 7766) over the
  dial of the member, and never with the resolver of this host. A, then AAAA, are asked over one
  connection; IPv4 addresses go first, and the flow tries up to 8 addresses. A name that the network
  cannot resolve is an error: there is no fallback to the host. A network whose far end resolves names
  (a `socks5h://` proxy, `relay`) gets the name as it is, and needs no entry. A flow that has an address
  needs no query.

## Guest wires

A guest (a VM, a container, a host behind one wire) uses the router as its gateway. The wire carries IPv4
packets. A userspace stack ends the TCP and the UDP of the guest, and the router sends each flow to a network
or a group, as it does for a door.

```yaml
guests:
  - wire: unix:/run/g.sock    # unix:/PATH or tun:NAME
    net: 10.66.0.0/24         # the guest net
    via: egress               # a network or a group of the file
    rules: /etc/r.rules       # optional: rules that go before `via`
    allow_shadowed_rules: false
    mtu: 1500                 # optional, 576 to 9000
```

| Key | Meaning |
| --- | --- |
| `wire` | `unix:/PATH`: the router listens on a `SOCK_SEQPACKET` socket (mode 0600). Each message is one IPv4 packet. One guest is connected at a time; a new connection replaces the old one. `tun:NAME`: the router opens the TUN device `NAME`, which must exist or be allowed (`CAP_NET_ADMIN`). The operator sets its address and routes. |
| `net` | `A.B.C.D/N`, N from 8 to 30. The router is the first host of the net (the gateway of the guest). |
| `via`, `rules`, `allow_shadowed_rules` | As for a door. |
| `mtu` | The MTU of the wire. Default 1500. Set it to the MTU of the guest. |

* The router drops a packet whose source is outside `net`, whose destination is in `net`, a multicast,
  broadcast, loopback or link-local address, or whose protocol is not TCP or UDP. IPv6 is dropped.
* TCP: the router holds the first SYN of a flow and dials the target (30 s limit) before the handshake. A dial
  that fails, or a rule that rejects, answers the SYN with a RST: `connect` of the guest fails at once. A copy of
  the SYN waits. At 2048 flows a new SYN is dropped, and the guest sends it again.
* UDP: each flow (guest address and target) has one socket from `Dial::datagrams` of the way that the rules
  choose, and takes one member of a group. A flow has a queue of 32 datagrams; a datagram for a full queue is
  dropped and counted. A flow ends after 60 s with no datagram. At most 512 flows and 256 ports. IPv4
  fragmentation (the feature `ipv4-fragmentation` of `pod-netstack`) carries a datagram over the MTU, up to
  65507 bytes.
* The guest resolves names itself (a flow to its resolver is a UDP or TCP flow), so a `domain:` rule does not
  match a flow of a guest. Use `cidr:` rules.
* `--status` shows each guest with its counters. The metrics have `podnet_route_guest_*` rows with the label
  `guest` (the wire): `tcp_dialed`, `tcp_refused`, `tcp_syn_dropped`, `tcp_open`, `udp_flows`, `udp_up`,
  `udp_down`, `udp_queue_dropped`, `udp_oversize`, `udp_refused` and `dropped`. The counters of the stack are
  `stack_udp_rx_dropped` (the backlog of a port was full), `stack_udp_tx_dropped` and `stack_udp_oversize`.
* `--check` prints a line for each guest.

## Groups, health and policies

The probe connects to `health.target` through each member of each group, each `interval` seconds
(default 10).

| Key | Default | Meaning |
| --- | --- | --- |
| `target` | none | `HOST:PORT` that each member must reach. |
| `interval` | 10 | Seconds between two probes. 1 or more. |
| `down_after` | 4 | K: the misses in a row that take a member down. |
| `up_after` | 3 | R: the passes in a row that take a member up. |
| `cooldown` | 30 | The least seconds in cooldown before a member is up again. |
| `timeout` | 5 | Seconds of one probe. 1 or more. |
| `window` | 8 | How many passing probe times the floor of `minrtt` reads (0 to 64). |
| `egress` | none | A map `NETWORK: HOST:PORT=IP`, the egress check of a network (below). |

The values must give K > R > 1 and R > K/2. A member in `cooldown` or `probation` is not up. A group
with no member that takes flows refuses new flows.

### Groups of links

A member can be a link: a network `peer:FINGERPRINT/SERVICE`. A group whose members are links is a peer
way. A group does not mix links and other networks. The pick, the policies, the health steps and the kill
switch are the same as for other groups. A flow takes one link when it opens, and the stream stays on it.
A group of links has 1 to 16 members, and each weight is 1 to 100. The probe of a link opens one stream
to its service. See [link.md](link.md), "Link groups".

### Egress check

A probe that connects says that a network reaches the target. It does not say by which address the
traffic leaves. `egress` adds a check for the networks that it names. After the probe passes, the router
connects through the same member to `HOST:PORT`, an echo service of the operator, and reads at most 128
bytes. The service writes the address that it sees for the peer (an IP address, `IP:PORT` or `[IP]:PORT`,
then a new line or the end) and closes. The address must be the `IP` of the entry. podnet names no echo
service of another party: give one that you run or trust.

* The same address: the probe passes, and its time is the time of both connects.
* Another address: the result is `WrongEgress`. The member goes `down` at once, with no K misses, whatever
  the control probe says. No other way replaces it: the group has no direct way, so a group with no other
  member refuses its flows. One pass puts the member on `probation`, as after any `down`.
* An answer that is no address, or a read that fails: a miss that is not clean (as a time out), so it
  alone never takes the member down.

A WARP member has its own check, on by default: the trace request of the WARP session (`/cdn-cgi/trace` of
the trace endpoint of `cloudflare-dns`, over HTTPS through the session). The answer must say `warp=on` or
`warp=plus`. Another answer is `WrongEgress`, with the steps above. A trace that fails is a miss. An entry of
`egress` for the same network goes first: the router runs the entry and not the trace.

### Renewal of a WARP member

A member whose network is `warp` (see "WARP as a member") gets a new link when its probe asks for it:

* The step to `Down` (K clean misses, with a control that passed), `WrongEgress`, and `AuthExpired` ask for it.
  `AuthExpired` is a failure of class `Refused` or `Revoked` when the member opens a session: the edge refused
  the key of the device (the answer 403, or the TLS alert `access_denied`).
* The router makes the link as `--reconnect` does: the new link first, then the new flows, then the old link
  ends after 30 s. The link reads the identity file again, so it registers no new device. If the edge refuses
  the stored key, the link enrols a new key for the same device once and tries again.
* Two renewals of one network keep a gap of 30 s. Each renewal is logged on stderr.
* The control gates the renewal. When each member of a group fails, the host is out, no member goes `Down`,
  and nothing is renewed. The session of a WARP member of a router has no probe of its own
  (`pod-warp` `Live` with `probe` off): the probe of the router decides.
* A network that is not WARP is not renewed by its probe.

A policy chooses the member of a new flow:

| Policy | Choice |
| --- | --- |
| `weighted` (default) | A random pick by weight. |
| `rr` | The members that take flows, in turn. The weights do not count. |
| `minrtt` | The head: the member with the least round-trip floor. The head changes only when another member is better by a margin (the larger of 5 ms and 10 percent) for 3 probe rounds. |
| `leastload` | The least busy member (by the bytes and the open flows, with a decay). |
| `protect` | A weighted pick among the members that are `up`. A member in `cooldown` or `probation` gets no flow while a member is `up`. |

`splits` sends the UDP datagrams of a group out through one member and receives them on another:
`NAME: up=MEMBER down=MEMBER`. It is off by default. TCP is not split. The byte meter of a member counts
TCP flows and UDP datagrams, in both directions. With a split, `up` counts the datagrams that it sends
and `down` counts the datagrams that it receives.

### WARP as a member

`warp` and `warp:identity=FILE` are networks of the registry ([egress.md](egress.md)). A `networks:` entry
`w: warp` gives TCP and UDP through a WARP session. A name is resolved inside the tunnel. The session is
`pod-warp` `Live`: a session that the edge ends is replaced, and a dial waits for the new one (20 s at most).

The device is kept in the identity file: `FILE`, else `warp.json` in `state_dir/NETWORK`, else the default
file of `podnet warp` ([warp.md](warp.md)). A file that is new is registered, and the registration is logged
on stderr. A network of the router with a `state_dir` has a device of its own. `warp` cannot go over another
way (`Ctx::below`): that gives `cannot go over another way`.

A door over a WARP member carries SOCKS5 CONNECT, HTTP CONNECT and UDP ASSOCIATE through the session; the
datagrams use `Dial::datagrams` of the session. `podnet forward --network warp` takes the same network: TCP,
`-L udp:...` and the DNS door go through the session.

## Dry run: `--check --to`

`--check` reads the file and brings up no network. For each `tailscale:/PATH` network, it prints a line
`podnet: route  tailnet NAME: socket=PATH BackendState=... tailnet=...` after one read of the LocalAPI of
`tailscaled`. A daemon that is not `Running`, or that does not answer, gives the reason in that line; the
exit code does not change. It exits 0 when the file is valid, and 64 with the key
that is wrong.

`--check --to HOST:PORT` adds a dry run for one target. It prints, for each door (`--door INDEX` selects
one, counted from 0), the rule, the network or the group, the member and its health, and it dials
nothing. It exits 1 when a door refuses the target or when its group has no member that is up.

```text
door	listen	rule	way	member	health
0	unix:/run/r.sock	default	egress	home	up
```

## State: `--status`

A router with the key `control` serves a unix socket (mode 0600). `podnet route --status unix:/PATH`
prints the state of that router: each group, each member with its health, round-trip time and floor, the
flows, the bytes, the pinned member and the head. `--json` prints the document. Exit 69 when no router
answers. `--status` cannot be used with `--config`.

Each door shows its counters: `flows` (the flows that came through the door, and the UDP associations),
`open` (the flows that have not ended) and `bytes` (TCP and UDP, both directions, counted at the door). The
counters of a door start at 0 when it is added and go with it when it is removed. A door that the file
names and a door that `--add-door` added have the same counters.

`podnet route --status unix:/PATH --metrics` prints the rows of the metrics in the text format of
Prometheus (it cannot be used with `--json`). There is one row for each member and metric, with the
labels `group` and `member`:

| Row | Type | Meaning |
| --- | --- | --- |
| `podnet_route_member_up` | gauge | 1 when the member is `up`; 0 in `cooldown`, on `probation` or `down`. |
| `podnet_route_member_takes_flows` | gauge | 1 when the member is not `down`. |
| `podnet_route_member_weight` | gauge | The weight of the member. |
| `podnet_route_member_rtt_seconds` | gauge | The time of the last probe that passed (no row before the first). |
| `podnet_route_member_rtt_floor_seconds` | gauge | The least time in the window of passing probes. |
| `podnet_route_member_open_dials` | gauge | The dials that have not ended. |
| `podnet_route_member_flows_total` | counter | The flows that the pick gave the member. |
| `podnet_route_member_bytes_total` | counter | The bytes of TCP and UDP, both directions. |
| `podnet_route_group_refused_total` | counter | The flows that the kill switch of the group refused (label `group` only). |
| `podnet_route_door_flows_total` | counter | The flows that came through the door (label `door` only: the listen address). |
| `podnet_route_door_open_flows` | gauge | The flows of the door that have not ended (label `door`). |
| `podnet_route_door_bytes_total` | counter | The bytes of TCP and UDP, both directions, that the door carried (label `door`). |

## Control: `--control`

`podnet route --control unix:/PATH` changes a running router with no restart. Flows that run go on. Give one of
these flags. Each prints the new state (`--json` for the JSON document).

| Flag | What it does |
| --- | --- |
| `--select GROUP=MEMBER` | Pins the group to one member. Each new flow takes it, whatever the policy says. A pinned member that is down refuses the flows: the group does not fall back to another member or to direct. |
| `--select GROUP=auto` | Gives the choice back to the policy. |
| `--rotate MEMBER` | Makes a new identity of the network (WARP: a new MASQUE key for the stored device). The router brings up the new link first, gives it to the new flows, and ends the old link after 30 s (a flow on it can end before). The network keeps its state directory (`state_dir/NETWORK`), so it registers no new device. One rotate for each network and minute: a second one gets exit 75. Each rotate is logged on stderr. |
| `--reconnect MEMBER` | Makes a new link of the network with the same identity, make before break. |
| `--interval SECONDS` | Sets the seconds between two probes (1 to 86400). The wait of the probe ends at once. The file needs a `health` section. |
| `--add-door DOOR` | Adds a door. The value has the keys of a door of the file (`listen`, `via`, `rules`, `allow_shadowed_rules`, `dns`, `dns_upstream`) as a JSON object or as YAML text. The door listens at once. A `via` is a network or a group of the file, or a network that `--add-network` added. With port 0 the router picks a port; `--status` shows it. |
| `--remove-door LISTEN` | Removes the door that listens on `LISTEN` (the text of `--status`). The listener stops and the unix socket file goes. The flows that the door has open end at their own end. The other doors are not cut. A door of the file can be removed too. |
| `--add-network NAME=SPEC` | Brings up a network (a spec of [egress.md](egress.md)) with the name `NAME`. A door can use it at once as `via` or in its rules. It is not a member of a group. It has a swap, so `--rotate` and `--reconnect` reach it. The identity goes in `state_dir/NAME`. |
| `--remove-network NAME` | Removes a network. Exit 64 when a group has it as a member, or when a door or a guest names it in `via` or in its rules file. The link ends after 30 s, so the flows on it can end. A network of the file that nothing uses can be removed. |

A `MEMBER` is a network of the file. A rotate or a reconnect reaches each group, rule and door that
uses the network. The networks of the registry (`direct`, a proxy, the relay) have no identity, so a
rotate makes a new link as a reconnect does. A network with an identity takes the rotate. A WARP network
loads its device from `state_dir/NETWORK/warp.json`, enrols a new MASQUE key for it (the old key stops
working), saves the file, and opens a session with the new key. It registers no new device, so the service
sees one device however many rotates run. The old link keeps its session for the 30 s of the drain.

The socket answers an HTTP/1.0 request with JSON. The gate is the mode 0600 of the socket and the user
of the peer: the user of the router, or root. A peer that is not allowed gets 403 and nothing changes.

| Request | Body |
| --- | --- |
| `GET /route/status` | none |
| `GET /route/metrics` | none; the answer is the text format of Prometheus, not JSON |
| `POST /route/select` | `{"group": "egress", "member": "proxy"}`; `"member": null` or `"auto"` frees the group |
| `POST /route/rotate` | `{"member": "proxy"}` |
| `POST /route/reconnect` | `{"member": "proxy"}` |
| `PATCH /route/interval` | `{"seconds": 5}` |
| `POST /route/add-door` | the keys of a door: `{"listen": "tcp://127.0.0.1:0", "via": "egress"}` |
| `POST /route/remove-door` | `{"listen": "tcp://127.0.0.1:1081"}` |
| `POST /route/add-network` | `{"name": "lab", "spec": "direct"}` |
| `POST /route/remove-network` | `{"name": "lab"}` |

A write answers with the status document of the new state. An error answers `{"error": TEXT}` with
400 (a bad request, an unknown key of a door), 403 (the gate), 404 (an unknown path, group, member, door or
network), 405 (a wrong method), 409 (no `health` section; a door that listens there; a name that is taken; a
network that a door, a guest or a group uses), 429 (a rotate too soon) or 502 (the network did not come
up, or the door could not bind; the router stays as it was). The exit codes of the flags: 64 for 4xx, 75 for 429, 69 when no router answers.

## Kernel mode

The key `kernel` makes the router write policy routing with rtnetlink (no `ip` program). It needs
`CAP_NET_ADMIN`. At the end of the run, the router removes what it wrote.

```yaml
kernel:
  wans:
    - name: a            # letters, digits, - and _
      dev: tun0          # the interface of the WAN
      gateway: 10.0.0.1  # optional IPv4 next hop
      weight: 2          # 1 to 256 in the aggregate route (default 1)
      table: 101         # optional (default 100 + position)
      mark: 1            # optional fwmark (default 1 + position)
    - name: b
      dev: tun1
  table: 200             # the aggregate table (default 200)
  priority: 1000         # the priority of the first rule (default 1000)
  masquerade: true       # a masquerade on each WAN through nf_tables (default true)
  mss_clamp: true        # clamp the TCP MSS to the route MTU (default true)
  kill_switch: true      # an unreachable default route in the aggregate table (default true)
  rp_filter: true        # rp_filter 0 on each WAN (default true)
```

The plan has these rules, in this order of priority:

1. `lookup main suppress_prefixlength 0`: the routes of the local subnets win, and the default route of
   the main table does not.
2. `fwmark M lookup T` for each WAN: a marked packet leaves by its WAN.
3. `lookup AGGREGATE`: the other packets use the weighted multipath default route.

A change is made before the old state is broken: podnet writes the new routes with `NLM_F_REPLACE`, adds
the new rules, and then deletes the old rules that the new plan does not hold. There is no moment with no rule. With `kill_switch`, the aggregate table has an unreachable default route, so the traffic fails
when each WAN is down.

`podnet route --config FILE --export mwan-json` prints the WANs of the kernel section in the JSON form
of the mwan director (`wans` and `clients`) and runs nothing. It exits 64 when the file has no kernel
section.

## Background and autostart

`--detach --name NAME` runs the router as an instance (see [background.md](background.md)).
`podnet autostart add NAME -- route --config FILE` starts it at boot.
