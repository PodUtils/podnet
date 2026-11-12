# Router: `podnet route`

`podnet route --config FILE` runs a router from a YAML file. The router has networks, groups of
networks, health probes, policies and doors. A door is a SOCKS5 and HTTP CONNECT listener. Each flow
that comes to a door goes out through the network or the group that the door names. The kernel mode
writes policy routing for WANs, with no `ip` program.

The router needs no root, no TUN device and no namespace, except the kernel mode, which needs
`CAP_NET_ADMIN`.

```sh
podnet route --config router.yml                        # run in the foreground
podnet route --config router.yml --detach --name edge   # run in the background
podnet route --config router.yml --check                # read the file and bring up nothing
podnet route --config router.yml --check --to example.com:443   # a dry run for one target
podnet route --status unix:/run/r.ctl                   # the state of a running router
podnet route --status unix:/run/r.ctl --json
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
control: unix:/run/r.ctl  # optional: the socket of `route --status`
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
  `tailscale:/PATH`, `netbird:[PORT]` or `unix:/PATH`. A `tailscale:/PATH` network is a member of a group
  and the `via` of a door, as each other network. When `tailscaled` is not `Running` at the start, the
  router prints one warning and goes on: each dial through that network fails until the daemon is logged
  in.
* `groups` maps a name to a list of members. A member is a network name with an optional weight
  (`home*2`). The default weight is 1.
* The file needs one door or more, unless it has a `kernel` section. A file with `kernel` needs no door.
* A flow takes one member and never moves to another.

## Doors

* A door accepts SOCKS5 (TCP) and HTTP CONNECT. The door has no UDP ASSOCIATE.
* `via` names the network or the group for a target that no rule matches.
* `rules` is a rules file. A rule is `domain:NAME=WAY`, `domain:.NAME=WAY` (the name and each name under
  it), `cidr:NET=WAY` or `cidr:@FILE=WAY` (one prefix on each line of FILE). `WAY` is a network or a
  group of the router, or `reject` (or `block`). The first rule that matches wins. `#` starts a comment.
  podnet reads the file again when it changes. A rule that an earlier rule shadows gives a warning; a
  shadowed `reject` rule is an error, and the file does not load unless the door has
  `allow_shadowed_rules: true`.
* `dns` and `dns_upstream` start a DNS door (UDP and TCP) on `dns`. It sends each query over the same
  way as the door, to the resolver `dns_upstream` at the far end of the way. The router names no resolver
  of its own, so give both keys or none.

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

The values must give K > R > 1 and R > K/2. A member in `cooldown` or `probation` is not up. A group
with no member that takes flows refuses new flows.

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
TCP flows only.

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
flows, the bytes and the head. `--json` prints the document. Exit 69 when no router answers. `--status`
cannot be used with `--config`.

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
