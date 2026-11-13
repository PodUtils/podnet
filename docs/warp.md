# WARP

`podnet warp` is a WARP client (the Cloudflare WARP client, in one binary). It uses MASQUE (`CONNECT-IP` over HTTP/2) on
TCP 443. It needs no root and no TUN device.

```sh
podnet warp register                # register a device, or load the saved one
podnet warp status                  # the device and the account
podnet warp status --json           # the account record as JSON
podnet warp license <KEY>           # attach a WARP+ license key
podnet warp rotate                  # make and enrol a new MASQUE key
podnet warp delete                  # remove the device and the saved file
podnet warp probe                   # test TCP connects through the tunnel
podnet warp exit                    # the exit colo, country, address and IPv6 address (--no-ipv6: no IPv6 address)
podnet warp scan --candidate 162.159.198.1   # measure front doors (TCP and TLS; --timeout-ms 100 to 60000, default 3000)
podnet warp export --out warp.conf  # a WireGuard device for other clients (UDP)
podnet warp export --format json --keepalive 0 --endpoint HOST:PORT   # the JSON form; no keepalive line; another endpoint
podnet warp export --export-identity dev.json   # the file of the exported device (default: warp-wg.json in the state directory)
podnet warp export --delete         # remove the exported device from the service and delete its file
podnet warp proxy                   # local SOCKS5 and HTTP CONNECT proxy
podnet warp forward -L 5432=db.internal:5432   # as ssh -L, through WARP
podnet warp forward --list          # the forwards of the detached instances
podnet warp run -- curl https://cloudflare.com/cdn-cgi/trace
podnet warp tun --interface podnet0        # TUN interface (needs CAP_NET_ADMIN)
podnet warp up                      # a proxy in the background for this sandbox
eval "$(podnet warp env)"           # a shell joins it
podnet warp exec -- git clone ...   # a program under the pod of the proxy
podnet warp nc HOST PORT            # a byte pipe, for an SSH ProxyCommand
podnet warp down                    # stop it
```

## Device and state

* The device and its key are in `warp.json` in the state directory: the
  first that podnet can write of `PODNET_STATE_DIR`, `$XDG_STATE_HOME/podnet`,
  `~/.local/state/podnet`, `$XDG_RUNTIME_DIR/podnet` and `podnet-<uid>` in the
  temporary directory. The file mode is 0600. podnet checks the directory
  before it registers a device. podnet writes no file in a directory of
  another user (root with the `HOME` of a user): the state directory skips
  that `HOME`, and `warp ssh-config --write`, `warp up --persist` and
  `--env-d`, and `autostart` exit 78. `PODNET_FOREIGN_HOME=allow` writes
  anyway. A WARP identity file that another user owns is refused (exit 78).
  `PODNET_FOREIGN_IDENTITY=allow` reads it when `--identity PATH` names it:
  the mode must be 0600, podnet logs a warning and writes nothing next to
  the file, and the default file of the state directory is not covered.
  Use `--identity PATH` for a different file,
  `--ephemeral` or `PODNET_STATE_DIR=none` to use a new device for one run.
* IPv6: the stack has the IPv4 and the IPv6 address of the device. The
  proxy asks for A and AAAA records through the tunnel and tries IPv4
  first, then IPv6, so a host with only AAAA records works too.
  `warp tun` adds the IPv6 address (/128) to the interface.
* What a destination sees (measured by a tester on 0.3.0-beta.8; issue
  183): not the address of the device (`172.16.0.2`, `2606:4700:110:...`),
  but an egress address of Cloudflare. The egress address stays the same
  in one MASQUE session, and a new session gets a new one (IPv6 from a
  `2a09:bac1::/32` or `2a09:bac5::/32` prefix). The `--listen` listeners of
  one proxy share one session and one address; each `--listen-identity`
  has its own device, session and address. `warp rotate` changes the
  MASQUE key only. `cdn-cgi/trace` gives the IPv4 `ip=` also for an IPv6
  flow.
* `--session-max-age SECS` (or `PODNET_WARP_SESSION_MAX_AGE`) closes each
  session at that age and opens a new one, with a new egress address, for
  `warp proxy`, `forward`, `run`, `up` and `tun`. The TCP flows of the old
  session end. Default: off. The log gives the cause of each close of a
  session.
* `--probe IP:PORT` (default `1.1.1.1:443`) is the TCP probe of the data
  path. A session can carry ICMP and UDP but no TCP. Each 30 s podnet
  connects to the target through the session (`warp tun`: one SYN from the
  tunnel address and port 61999; the answer does not go to the TUN). After
  3 failures in a row, a new session takes the new flows (the log names
  `reason=tcp-probe`). `--probe off` stops it.
* New devices: the WARP service limits the registrations of each address.
  Observed: HTTP 429 (error 1015) after about 10 new devices in 90 minutes,
  cleared after about 30 minutes. podnet then says so. A saved identity,
  `warp rotate` and new sessions do not register. `--ephemeral`, a new
  `--listen-identity` file and `warp export` each register one device.
## Proxy

* `podnet warp proxy` listens on `tcp://127.0.0.1:40000` by default. Use
  `--listen unix:/path` on a host that refuses `AF_INET` binds. Each
  `--listen` is bound on its own: a listener that the host refuses is
  reported, and the proxy starts with the others (`--strict` exits 69
  instead). With no listener, the exit code is 69. Names are resolved
  through the tunnel. A pin of `--resolve HOST=IP` or `PODNET_RESOLVE`
  comes first and sends no query. This applies also to `warp forward` and
  `warp nc`. SOCKS5 UDP ASSOCIATE is supported.
* `--proxy-user USER --proxy-password-file FILE` (also on `warp up`): each
  client must give the user and the password of the first line of FILE
  (mode 0600), with SOCKS5 (RFC 1929) or HTTP `Proxy-Authorization: Basic`.
  A listener on an address that is not loopback, with no credentials, gets
  a warning. `--max-connections N` limits the clients; `--dns ADDR`
  answers DNS through WARP.
## WARP as a network of the registry

`warp` and `warp:identity=FILE` are specs of the registry of networks ([egress.md](egress.md)): TCP and UDP
through a WARP session, with names resolved inside the tunnel. `podnet forward --network warp` and a
`networks:` entry of a router file take them ([router.md](router.md), "WARP as a member": health, the egress
check by the exit trace, the renewal of a member, `--rotate` and `--reconnect`). The device is kept in `FILE`,
else in the default identity file of `podnet warp`, else (in a router with `state_dir`) in
`state_dir/NETWORK/warp.json`. The network has none of the flags of `podnet warp` (`--cc`, `--masque-endpoint`, `--session-max-age` and the
others). It reads the variables `PODNET_WARP_API`, `PODNET_WARP_CC`, `PODNET_WARP_TCP_BURST` and
`PODNET_WARP_NO_CHECK`.

## Transports and carriers

* `--fragment-client-hello [off|record|sni]` (default `off`; a flag of `podnet warp`, before or after the subcommand; `PODNET_WARP_FRAGMENT`):
  the TLS ClientHello of the MASQUE connection goes in more than one record
  of `--fragment-size` bytes (16 to 4096; default 64-160). `sni` ends the first record in the
  server name. The server joins the records (RFC 8446, section 5.1); a
  middlebox that reads the name from the first record does not find it.
* `--carry wg` (a flag of `podnet warp`, before or after the subcommand; `PODNET_WARP_CARRY`): a second WireGuard device
  (`warp-carry.json` in the state directory) inside the tunnel, on a UDP
  socket of the first stack. The tools use the stack of the second device
  (IPv4, MTU 1200), so a destination sees its exit. With `--transport
  wireguard`, both hops are WireGuard. Not for `warp tun`.
* `warp proxy --l4` (`PODNET_WARP_L4`): each TCP client is one HTTP/3
  `CONNECT` stream to the L4 door of WARP (UDP 443, TLS name of `--l4-sni`,
  `consumer-masque-proxy.cloudflareclient.com`), with no IP stack in
  between. UDP (SOCKS5 UDP ASSOCIATE) is not served. The rules apply (`-r`,
  `--rules-file`, `--rule-default`): a `cidr:` rule decides an address, and a
  name with no other rule takes `--rule-default`.
* `--transport wireguard` (a flag of `podnet warp`, before or after the subcommand; `PODNET_WARP_TRANSPORT`): the tunnel on
  WireGuard (UDP, port 2408 of the record, or `--wireguard-endpoint`) in
  place of MASQUE. The device is the one of `podnet warp export`
  (`warp-wg.json` in the state directory); podnet registers it when the file
  has none. The stack, the proxy and the commands are the same. There is no
  relay and no proxy: WireGuard is UDP.
* The reserved bytes: WARP reads bytes 1 to 3 of each WireGuard message,
  the `client_id` of the device (`# Reserved = a,b,c` in the output of
  `warp export`, `reserved` in the JSON form). Kernel WireGuard and
  wg-quick send zeros there. `--transport wireguard` sends the bytes. A
  device that needs them has fresh handshakes and no data path in kernel
  WireGuard. `podnet warp export --check` runs the WireGuard leg of podnet
  with the bytes and with zeros, with a TCP connect to the target of
  `--probe` through each. It prints `device ok` (exit 0), `needs reserved
  bytes` (exit 1) or `device data path dead, use --new` (exit 69). The
  TCP probe of `--probe` also runs on the stack of `--transport
  wireguard`: a leg with fresh handshakes and no TCP gets a new session.
* `--masque-transport h3` (a flag of `podnet warp`, before or after the subcommand; `PODNET_WARP_MASQUE_TRANSPORT`): the MASQUE
  session over HTTP/3 on UDP 443 in place of HTTP/2 on TCP 443. Each IP
  packet is an HTTP datagram (RFC 9297), so a lost packet does not stop the
  other flows. A packet larger than a datagram goes as a capsule on the
  request stream. There is no relay and no proxy: they carry TCP only.
* `--cc none|reno|cubic` (a flag of `podnet warp`, before or after the subcommand; `PODNET_WARP_CC`): the congestion
  controller of the TCP connections of the userspace stack, for the data
  that it sends (an upload). The default is `none`: only the window of the
  peer limits it.
## Listeners, identities and rules

* `--listen-identity LISTEN=PATH` (repeatable): the clients of LISTEN use
  the WARP device of the identity file PATH (podnet makes it when it does not
  exist), with a session of its own and its own address. The `--listen`
  listeners use `--identity`. One file is for one listener only. The ready
  document lists the listeners and their devices in `identities`.
* Rules of `warp proxy` (`-r KIND:VALUE=ACTION`, repeatable): KIND is
  `domain` (`NAME`, or `.NAME` for the name and the names under it) or
  `cidr` (`ADDR/PREFIX`, IPv4 or IPv6, or `@FILE` with one prefix on each
  line, for example the prefixes of a country). ACTION is `tunnel`,
  `direct` (from the host, with the resolver of the host and no proxy) or
  `reject` (SOCKS5 reply 0x02, HTTP 403). The first rule that matches wins.
  A name that no domain rule matches is resolved through the tunnel when a
  cidr rule exists. Then a loopback, a link-local or an unspecified address
  and `localhost` are refused, then `--rule-default` (`tunnel`) applies.
  `--rules-file FILE` adds the rules of FILE (one on each line, `#` starts a
  comment); podnet reads it again when it changes and keeps the rules before
  when it has an error. `--check` prints the rules and exits. The rules
  apply to TCP; UDP goes through the tunnel. The health line counts the
  decisions.

  ```sh
  podnet warp proxy -r domain:.corp.example=direct -r cidr:10.0.0.0/8=direct \
      -r domain:.ads.example=reject --rules-file ~/.config/podnet/rules
  ```
## Run a program

* `podnet warp run --backend netns-raw` runs the program in a user and network
  namespace whose TUN goes to WARP as raw IP packets: TCP, UDP, ICMP and
  IPv6, with the kernel stack of the namespace and no root. The TUN has the
  WARP addresses and the default routes, and the program gets a
  `resolv.conf` with 1.1.1.1. `--backend auto` takes it when this host can
  make the namespace (and no `-r` rule is given), else the pod ([pods.md](pods.md)).
* `podnet warp run` starts the proxy on a unix socket and runs the program with
  `podnet pod` (`auto` takes the `seccomp` backend first, for UDP). It passes
  `-r` (for example `-r cidr:10.0.0.0/8=direct`), `--keep-proxy-env` and
  `--inbound-dir` to the pod. A connect to 127.0.0.1 goes to the host, not
  to WARP. The TCP connections and the DNS queries of the program go
  through WARP, also for static binaries. With the `seccomp` backend, other
  UDP datagrams also go through WARP (SOCKS5 UDP ASSOCIATE). ICMP (`ping`)
  goes through WARP only with `--backend netns-raw`; the pod does not carry
  it.
* `podnet warp up [--port 40000] [--persist]` starts `warp proxy` in the
  background (instance `warp`, see [background.md](background.md)) on
  `127.0.0.1:PORT` and on a unix socket, and writes an env file with the
  proxy variables. When the host refuses the TCP listener, `warp up` starts
  with the socket only: the env file has no proxy variables, `warp env` says
  so and exits 69 with no output, and `warp exec` and `warp shell` work. Later shells join it:
  * `eval "$(podnet warp env)"`: `ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY`,
    `NO_PROXY` (and the lower-case forms), `CARGO_HTTP_PROXY` and the npm
    variables. The proxy also forwards `http://` URLs (for apt).
  * `podnet warp exec -- CMD` and `podnet warp shell`: the command runs under
    the pod of the socket: TCP, DNS and UDP go through WARP, also for static
    binaries, with no proxy variable. `--backend auto|netns|seccomp` selects
    the backend of the pod (default `auto`: netns first, then seccomp when
    the user namespace or the TUN fails). The pod writes one line on stderr
    that names the backend that runs, its limits and the reason of the
    choice; `-q` removes the line. With `-v`, the pod logs each flow, and
    the seccomp backend logs each socket and each `setsockopt` that it
    emulates.
  * `podnet warp exec --detach --name N -- CMD`: the pod is an instance
    (see [background.md](background.md)). It lives until each process of the pod
    ends, also a daemon that CMD starts (`rad node start`). `podnet ps`
    lists it, `podnet logs N` shows the output of the pod, and
    `podnet stop N` sends SIGTERM to each process of the pod (SIGKILL after
    5 s). The supervisor starts the pod again when it ends; `--no-restart`
    turns it off. `--name N` with no `--detach` runs the pod in the
    foreground as the instance N.
  * `podnet warp exec --join N -- CMD`: CMD runs in the pod of the instance
    N. It reaches the loopback listeners of the pod N, and a name gets the
    address that the pod N gave it. With the netns backend, CMD enters the
    namespaces of the pod N. With the seccomp backend, a new pod shares the
    inbound listeners and the fake DNS table of the pod N
    (`--session $RUN/N.pod` of `podnet pod`). `podnet stop N` does not stop
    CMD.
  * `--persist`: a marked block in `~/.bashrc` and `~/.profile` loads the env
    file in each new shell, and an interactive shell runs under the pod.
  * `--env-d`: `$XDG_CONFIG_HOME/environment.d/90-podnet-warp.conf`
    (`~/.config/environment.d/...` when the variable is not set) has the proxy
    variables. The systemd user manager gives them to the programs of the
    next login (a desktop, a browser, a user unit). A file of that name that
    podnet did not write stays, and `warp up --env-d` stops with an error.
  * `podnet warp down` stops the instance and removes the env file and the
    blocks (and the environment.d file of podnet); the rest of each file
    stays.
  * The CA bundle: when `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`,
    `CURL_CA_BUNDLE` or `GIT_SSL_CAINFO` names a bundle of fewer than 10
    certificates (the roots of a proxy that terminates TLS), the tools that
    use it fail through WARP, which shows the real chains. `warp up` then
    writes `warp-ca.pem` (that bundle and the system bundle) and the env
    file, `warp exec` and `warp shell` name it, so that the proxy and WARP
    both work. With no system bundle, a warning says to unset the variable.
## SSH over WARP

* `podnet warp nc HOST PORT`: a byte pipe from stdin and stdout to HOST:PORT
  through the running `podnet warp up` (else a WARP session of its own), for
  `ssh -o ProxyCommand='podnet warp nc %h %p'` and git over SSH where port 22
  is closed. `--via-relay` goes through the WebSocket relay to HOST:PORT
  with no WARP. `podnet warp ssh-config [--host PATTERN]` prints the SSH
  config block and a `GIT_SSH_COMMAND`. SSH checks the host key as on a
  direct connection. At the end of stdin, podnet sends no FIN: the pipe ends
  when the far end closes, or `--idle` seconds (default 5) after the last
  data. `-N` (`--shutdown`) sends the FIN at the end of stdin, then waits for the far end to close, for a
  server that waits for it. `-u` (`--udp`) sends each read of stdin as one datagram to HOST:PORT, and writes
  each datagram that comes back to stdout. An error of the far end exits 69.
* `podnet warp ssh-config --write --host github.com` puts the block in
  `~/.ssh/config` (or `--config FILE`), between two marker lines, before
  the first `Host` or `Match` line. A second `--write` replaces it (for
  example after podnet moved). `--check` exits 1 when the block is missing or
  stale; `--remove [--host PATTERN]` removes it. No other line of the file
  changes. A `Host` line of the file with the same pattern is an error, and
  the files of `Include` are not read. `podnet warp down` keeps the block.
  `--host` is one pattern of 1 to 255 characters from `A-Z a-z 0-9 . _ -
  * ? ! : @ % , [ ]`; a space, a newline or another control character
  exits 64, and nothing is printed or written.
## TUN interface

* `podnet warp tun --default-route` (CAP_NET_ADMIN) sends the whole host
  through WARP: a host route keeps each address of the MASQUE connection
  (the proxy, the front door, the relay) outside the TUN when the TUN
  would capture it. An address on loopback or on a connected subnet needs
  none, so a host with no default route works when its proxy is there; an
  address with no route at all is refused with its name. `0.0.0.0/1`,
  `128.0.0.0/1`, `::/1` and `8000::/1` go to the TUN; DNS goes to 1.1.1.1
  (`resolvectl` on the TUN, else `/etc/resolv.conf` with a copy kept). The
  default routes go away while the session is down, so traffic fails fast.
  The end (Ctrl-C, SIGTERM, `podnet stop`) puts back each route and the
  resolver. The changes are ioctls; no script runs as root.
  `podnet warp up --tun` starts it detached (instance `warp-tun`), and
  `podnet warp down` stops it.
* `podnet warp tun` makes a TUN interface with the tunnel address and moves its
  packets through WARP. It writes 0 to the `rp_filter` of the interface; `--keep-rp-filter` keeps the value. It does not change the routes: add routes for the
  destinations to send through WARP, and keep a route for the MASQUE endpoint
  outside the interface. The command prints examples.
## Sessions, relay and the registration API

* `podnet warp proxy`, `podnet warp run` and `podnet warp tun` open a new MASQUE
  session when the session ends (a network change, the front door, the
  limit of a relay): at once, then with a backoff of 1 s doubled to 30 s.
  A new client takes the new session; a client that comes while the
  session is down waits for it (20 s at most). `warp tun` keeps its
  interface. The log has `a new WARP session is open` for each.
* When the direct MASQUE path fails (a proxy that refuses CONNECT to an
  IP address, or that answers and carries nothing), podnet opens the same
  MASQUE TLS through the WebSocket relay of `--relay` (default
  `https://tcp.ssh.relay.ajam.dev`, see [tunnels.md](tunnels.md), "Edge protocol and transport"). The
  client certificate and the pinned key stay end to end. The path that
  worked goes first at the next session. `--no-relay` turns this off.
  `--masque-endpoint IP:PORT` and `--masque-sni NAME` replace the front
  door and its TLS name. With no IPv4 default route, the IPv6 door of the device record goes
  first. `podnet tunnel` takes `--masque-endpoint` too.
* `--resolver SPEC` (repeatable; `PODNET_WARP_RESOLVERS`): the resolver of the DNS inside the tunnel, in
  place of `1.1.1.1:53`. `SPEC` is `IP[:PORT]` (UDP) or `tls://IP[:PORT][#NAME]` (DNS over TLS, port 853;
  the certificate must have `NAME`, or the IP address). The first resolver that answers wins.
* `--device-name NAME` (1 to 64 characters), `--device-model MODEL` (default `PC`) and `--device-locale
  LOCALE` (default `en_US`; `PODNET_WARP_DEVICE_NAME`, `PODNET_WARP_DEVICE_MODEL`,
  `PODNET_WARP_DEVICE_LOCALE`) set what a new device declares to the WARP service.
* `--wireguard-endpoint IP:PORT` (`PODNET_WARP_WG_ENDPOINT`) replaces the WireGuard front door of the device
  record (port 2408). `--carry-endpoint IP:PORT` (`PODNET_WARP_CARRY_ENDPOINT`) replaces the front door of
  the carried device.
* The registration API calls and the MASQUE connection use the proxy (see
  [proxies.md](proxies.md)). A proxy that terminates TLS cannot carry the MASQUE
  connection, because the connection uses a client certificate.
