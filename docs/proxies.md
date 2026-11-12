# Proxies, DNS and TLS roots

This page gives how podnet reaches the network from a host with a proxy: the proxy URL forms, the order of
the proxy sources, the DNS chain and the TLS roots. `--egress` is the same choice as a network spec
([egress.md](egress.md)).

## Proxies and DNS

Each connection of podnet (the quick-tunnel API, DNS over HTTPS, the HTTP/2
edge leg, the WARP API, MASQUE, the fetch of the public URL) takes the same
proxy:

```sh
podnet --proxy socks5h://127.0.0.1:1080 tunnel --url http://127.0.0.1:8080
PODNET_PROXY="$PROXY_URL" podnet demo --exit-on-verify    # PROXY_URL: http://[USER:PASSWORD@]HOST:PORT
NO_PROXY=127.0.0.1,.internal podnet warp register
podnet --no-proxy tunnel --url http://127.0.0.1:8080
```

* The order: `--proxy` (or `PODNET_PROXY`), then `HTTPS_PROXY`,
  `https_proxy`, `ALL_PROXY`, `all_proxy`. A variable that is empty or does
  not parse is skipped. `--no-proxy` uses none.
* Percent-encode `/ ? # @ :` in the user and the password of a proxy URL
  (for example `%2F` for `/`).
* `http://` uses `CONNECT`. `socks5://` resolves the name on this host;
  `socks5h://` gives the name to the proxy. podnet refuses `socks4://` and
  `https://`.
* A proxy on a unix socket: `socks5h+unix:/PATH` (also
  `socks5h://unix:/PATH`), or `http+unix:/PATH`.
* A proxy at the stdin and the stdout of a command, when the only way out
  is a byte channel (issue 129): `--proxy 'http+exec:ssh gw nc 127.0.0.1
  3128'`. Each connection starts the command once, with `sh -c`, as the
  `ProxyCommand` of SSH; its stderr is the stderr of podnet. Only `--proxy`
  and `PODNET_PROXY` take `+exec:`: a variable of another tool does not start
  a program. QUIC legs need UDP and do not use it.
* Without `--proxy` and `--no-proxy`, podnet reads `HTTPS_PROXY`,
  `https_proxy`, `ALL_PROXY` and `all_proxy`. `NO_PROXY` names the targets
  that do not use the proxy.
* When no proxy variable is set, podnet takes the proxy of the first tool
  config that names one: `~/.gitconfig` and `~/.config/git/config`
  (`http.proxy`), `~/.npmrc` (`https-proxy`, else `proxy`),
  `~/.config/pip/pip.conf` and `~/.pip/pip.conf` (`proxy`),
  `/etc/gitconfig`, `/etc/pip.conf`, and `/etc/environment`. The log names
  the file. `--no-proxy` turns it off.
* With a proxy, `--protocol auto` does not try QUIC on UDP 7844 (no proxy
  carries UDP), and podnet prints one line that says so. `--protocol quic`
  keeps QUIC, with a warning.
* With a user and a password, podnet sends Basic. A proxy that answers 407
  with a Digest challenge (RFC 7616: MD5, MD5-sess, SHA-256,
  SHA-256-sess, `qop=auth`) gets the Digest answer on a new connection.
  Another 407 gets a message with the schemes that the proxy asks for. No
  NTLM.

A direct connection resolves the name with this chain: a pin of
`--resolve HOST=IP` or `PODNET_RESOLVE`, the system resolver, then DNS over
HTTPS by IP address at Cloudflare (1.1.1.1, 1.0.0.1) and Google (8.8.8.8,
8.8.4.4). It tries each address of the name, and starts the next attempt
250 ms after the one before.

## TLS roots

podnet checks the certificate of each TLS server that it connects to. Behind a
proxy that inspects TLS, give the root of the proxy:

```sh
SSL_CERT_FILE=/etc/corp-root.pem podnet tunnel --url http://127.0.0.1:8080
podnet --ca-file /etc/corp-root.pem warp register
```

`--ca-file` and `SSL_CERT_FILE` replace the other roots. Without them, podnet
trusts `podnet-ca.pem` beside the binary, the first system bundle that it can
read (for example `/etc/ssl/certs/ca-certificates.crt`), and the Mozilla
roots compiled into podnet. `podnet status` shows the roots in `tls_roots`. See
`SECURITY.md`.

## Front doors

A front door is a listener that speaks SOCKS5 and HTTP CONNECT. The listener detects the protocol from the
first byte. These commands run the same door (`crates/podnet-netns/src/door.rs`, T-061):

| Command | The way of a flow |
| --- | --- |
| `podnet pod --serve LISTEN -x UPSTREAM` | The chain of `-x`, or the chain that a `-r` rule chooses. |
| `podnet net proxy` | The virtual network of the stack of the command (`--map`, `--forward`, `--switch`). |
| `podnet net tailscale` | `tailscaled`, through its LocalAPI. |
| `podnet route` | A network or a group of the router ([router.md](router.md)). |

`podnet warp proxy` has its own door until the take of `pod-warp` (T-050).

The door has these rules for each client:

* The client has 30 s to send its first byte and its request. A client that is too slow gets SOCKS5 `0x04`
  or HTTP 504, and the door closes the connection.
* The door serves 1024 live clients at one time. It closes a client that comes over the cap.
* The door waits at most 30 s for the connection to the far end. `net tailscale` waits 10 s and `net proxy`
  waits 5 s.
* The failure of a dial gives the reply by its class: a refusal of a rule or of a policy is SOCKS5 `0x02` and
  HTTP 403; a time limit is SOCKS5 `0x04` and HTTP 504; any other failure is a general failure (or a
  host unreachable, when the text says refused or denied) and HTTP 502. An HTTP answer has a short text
  body.
* A request with port 0 takes the port of `--port` (`net proxy` and `net tailscale`).
* `pod --serve`, `net proxy` and `net tailscale` take SOCKS5 and CONNECT only. A plain `http://` request
  gets 405. `net proxy` and `net tailscale` end a flow after 300 s.
* BIND gets SOCKS5 `0x07` (command not supported). UDP ASSOCIATE goes through the first upstream for
  `pod --serve`, and gets `0x07` for `net proxy` and `net tailscale`.
