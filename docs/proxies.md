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
