//! The facts of podnet in the form of `pod-manual`: the variables of the
//! environment, the files, the exit codes and the examples. The manual, the
//! help and the checks of `src/cli/manual_tests.rs` read them from here.

use pod_manual::{Code, Example, Exit, Facts, FileFact, Var, VarGroup};

/// One variable, or a set of variables with one meaning: the names (the
/// first is the main name), the commands that use them, and the meaning.
type VarRow = (&'static [&'static str], &'static str, &'static str);

/// Each variable that podnet, its crates and the shims read or set. The
/// variables of the pager of `podnet man` come from `pod-manual`
/// ([`pod_manual::pager::variables`]).
pub const VARIABLES: &[VarRow] = &[
    (&["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"], "tunnel, warp, edge discovery, pod", "The proxy for each connection of podnet, when --proxy and --no-proxy are not given: http://, socks5:// (this host resolves the name) or socks5h:// (the proxy resolves it), [user:password@]host[:port]. The first variable that is set and can be parsed is used. With a proxy, --protocol auto does not try QUIC on UDP 7844. podnet pod uses the value as its default upstream."),
    (&["NO_PROXY", "no_proxy"], "all commands", "The targets that do not use the proxy, separated by commas: * (all), a name (it matches the names under it too), .name, an IP address, a network (10.0.0.0/8), each with an optional :port."),
    (&["PODNET_RESOLVE"], "all commands", "Addresses for names, in place of DNS: HOST=IP[,HOST=IP]. --resolve adds to it. Without a pin, podnet asks the system resolver, then DNS over HTTPS at 1.1.1.1, 1.0.0.1, 8.8.8.8 and 8.8.4.4. A bad value is a configuration error (exit 78) for each command."),
    (&["PODNET_TAILNET"], "net tailscale", "The tailnet that tailscaled must run in, in place of --tailnet: the name or the DNS suffix. The door starts only when the daemon is Running in it, and refuses new dials when the daemon changes to another tailnet."),
    (&["TS_FORCE_NOISE_443"], "net doctor", "Read only, to show its value in the tailscaled row of net doctor. It is a variable of tailscaled: set it to true when tailscaled starts on a host that reaches only port 443 (the control connection tries port 80 first)."),
    (&["NB_SOCKS5_LISTENER_PORT"], "net doctor", "Read only, to find the SOCKS5 listener of NetBird netstack mode in the NetBird row of net doctor (default 1080). It is a variable of the NetBird daemon. The network spec netbird:[PORT] gives its own port."),
    (&["HTTP_PROXY", "http_proxy"], "pod", "The default upstream of podnet pod when HTTPS_PROXY and ALL_PROXY are not set."),
    (&["TUNNEL_TOKEN"], "tunnel, serve, share, demo", "The named-tunnel token, in place of --token. Other users of the host can read a command line, but not the environment of a process of another user."),
    (&["TUNNEL_TOKEN_FILE"], "tunnel, serve, share, demo", "A file with the named-tunnel token, in place of --token-file. TUNNEL_TOKEN wins when both are set. An empty file is a usage error (exit 64)."),
    (&["REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE", "GIT_SSL_CAINFO"], "warp up", "Bundles of TLS roots of other programs, as SSL_CERT_FILE. One can name a small bundle: the roots of a proxy that ends TLS. Then the env file of podnet warp up sets it to one bundle of it and the system bundle. Then the proxy and WARP both work."),
    (&["SSL_CERT_FILE"], "all commands", "The only file of TLS roots (PEM), when --ca-file is not given. Without both, podnet trusts podnet-ca.pem beside the binary, the first system bundle that it can read, and the roots compiled into podnet."),
    (&["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_SESSION_TOKEN", "AWS_PROFILE", "AWS_SHARED_CREDENTIALS_FILE"], "link listen, dial, connect --carrier s3", "The key of the bucket of the s3 carrier, when --s3-credentials FILE is not given: the id and the secret (and the token of a temporary key), else the profile file of AWS_SHARED_CREDENTIALS_FILE or $HOME/.aws/credentials, section AWS_PROFILE (default: default). With --s3-credentials, the file alone (AWS_PROFILE names its section). No key exits 77; no message shows a value."),
    (&["PODNET_STATE_DIR"], "warp, tunnel --transport warp", "The state directory (the WARP identity). The value none keeps no file. Without it, podnet takes the first directory that it can write of $XDG_STATE_HOME/podnet, $HOME/.local/state/podnet, $XDG_RUNTIME_DIR/podnet and podnet-<uid> in the temporary directory. On a host whose home is on a shared disk, set it to a directory of this host. Then two hosts do not share one WARP device."),
    (&["PODNET_WARP_SOCKET"], "warp exec, warp shell, warp nc, agent", "The socket of the running podnet warp up; podnet warp up sets it in its env file and for the program of warp exec. These commands try --socket, else this variable, then warp.sock in the run directory. Last, they try the socket of the pointer file that podnet warp up writes (mode 0600). The file is $XDG_RUNTIME_DIR/podnet/warp-up.json, else podnet-<uid>/warp-up.json in the temporary directory. So they find a podnet warp up that has another state directory."),
    (&["XDG_STATE_HOME", "HOME", "XDG_RUNTIME_DIR", "TMPDIR"], "warp, net shim, net socksify, autostart", "The candidates of the state directory, and of the directory of a shim (a directory that is not noexec). podnet reads the home from HOME only, not from the user database."),
    (&["XDG_CONFIG_HOME"], "autostart", "The base of the systemd user unit: $XDG_CONFIG_HOME/systemd/user/podnet-NAME.service, else $HOME/.config/systemd/user."),
    (&["PODNET_LEG_FAILOVER"], "tunnel, serve, share, demo", "The value of --leg-failover when the flag is not given: staggered or serial."),
    (&["NO_COLOR"], "all commands", "When set, the log has no colours. The log has colours only when stderr is a terminal."),
    (&["PODNET_EDGE"], "tunnel, serve, share, demo, doctor, net doctor", "A comma-separated list of ip:port edge addresses. It replaces edge discovery. A value that cannot be parsed is a configuration error (exit 78)."),
    (&["PODNET_PROXY"], "all commands", "The value of --proxy when the flag is not given: http://, socks5:// or socks5h://HOST:PORT, SCHEME+unix:/PATH, or SCHEME+exec:COMMAND (a proxy at the stdin and stdout of a command). It keeps a proxy password out of the command line; podnet autostart puts it in the private env file of an entry. The env file of podnet warp up with no TCP listener sets it to the unix socket."),
    (&["PODNET_WARP_SHELL"], "warp shell, the block of warp up --persist", "Set in a shell that podnet warp shell started; the block of --persist then does not start it again."),
    (&["SHELL"], "warp shell", "The shell that podnet warp shell runs (default /bin/sh)."),
    (&["PODNET_RUN_DIR"], "--detach, ps, stop, logs, wait", "The directory of the detached instances (mode 0700): NAME.json, NAME.lock and NAME.log for each, and NAME.secrets (mode 0600) for an instance with a token or a proxy password. Default: $XDG_RUNTIME_DIR/podnet/run, else run in the state directory."),
    (&["PODNET_LINK_CONTROL"], "forward, any command with a peer:FINGERPRINT/SERVICE network", "The control socket of the link instance that holds the link to the peer. The ready document of podnet link connect or listen names it (control). Its token is in PATH.token. Without it, podnet asks the control socket of each link instance in the run directory."),
    (&["PODNET_DETACH_NAME", "PODNET_DETACH_HASH", "PODNET_DETACH_COMMAND", "PODNET_DETACH_SUPERVISED", "PODNET_DETACH_READY_TIMEOUT", "PODNET_SELF_EXE"], "--detach", "podnet sets them for the child of --detach: the name of the instance, the hash of its arguments, the command, and whether a supervisor runs it. PODNET_DETACH_READY_TIMEOUT is the --ready-timeout of the first start: a provision of a quick tunnel does not wait past it. PODNET_SELF_EXE is the path of the binary, for a host with no /proc. Do not set them."),
    (&["PODNET_MAILBOX"], "send, receive", "The value of --mailbox when the flag is not given: the mailboxes of magic-wormhole (wss:// or ws://), separated by commas, in order. The default is each mailbox of pod-services."),
    (&["PODNET_PIPING"], "send, receive", "The value of --piping when the flag is not given: the piping servers (https:// or http://), separated by commas, in order, for the last way of the transit between two podnet ends; none gives no piping server. The default is each piping server of pod-services."),
    (&["PODNET_RELAY"], "tunnel, serve, share, demo, relay", "The value of --relay when the flag is not given: the WebSocket-to-TCP relay of --transport relay (https:// and a host), or a comma list of hosts of one deployment. The default is https://tcp.ssh.relay.ajam.dev; when a host does not answer, podnet tries hosts of its pool."),
    (&["PODNET_REPO"], "update", "OWNER/NAME of the releases for scripts/install.sh. podnet update does not use it: its repository names the signer of the binary. A warning says so."),
    (&["PODNET_POLICY"], "all commands", "The egress policy of this process, as one JSON file. A rule names a host, a family, a port and an action: direct, proxy, relay, refuse or queue. The default action covers a destination that no rule matches. podnet classifies a destination before it dials, and a refusal names its rule. A file that cannot be read or parsed is a configuration error (exit 78), as for podnet rules check FILE."),
    (&["PODNET_NODE_CONFIG"], "all commands", "The node configuration that podnet init writes. When PODNET_POLICY is not set, its egress object is the policy. A file that cannot be read or parsed is a configuration error (exit 78)."),
    (&["PODNET_FOREIGN_HOME"], "the state directory, warp ssh-config --write, warp up --persist and --env-d, autostart", "allow: write in a directory of another user anyway (with a warning). By default, podnet writes no file in a directory that belongs to another user (root with the HOME of a user): the state directory skips that HOME, and the other commands exit 78."),
    (&["PODNET_FOREIGN_IDENTITY"], "the commands of warp and the tunnel commands with --identity or --warp-identity", "allow: read a WARP identity file of another user, named with --identity or --warp-identity (with a warning). podnet checks the mode (0600) and writes nothing next to the file. The default file of the state directory is not covered. By default, a file of another user exits 78."),
    (&["PODNET_AUTH_USER", "PODNET_AUTH_PASSWORD_FILE"], "tunnel, serve, share, demo, connect", "The values of --auth-user and --auth-password-file: each visitor of the tunnel must give this user and password (HTTP Basic); podnet connect sends them in the WebSocket upgrade. The file has the password on its first line and mode 0600."),
    (&["PODNET_RELAY_TOKEN"], "tunnel, serve, share, demo", "A token for the relay of --transport relay. It goes only in the X-Relay-Token header. Without it, podnet mints a token with POST /v1/mint on the relay and keeps it in relay-token-HOST.json (mode 0600) in the state directory."),
    (&["PODNET_RELAY_CONNECT_TOKEN"], "relay connect", "The connect token of a pair, for the operator side of the reverse relay. It goes only in the X-Relay-Token header. Without it, podnet reads the local pair file."),
    (&["PODNET_NETNS_TEST_MEM"], "pod (tests only)", "A test hook of the seccomp backend: no-fd does not open the memory of the program for writing (process_vm_writev is used); none also does not use process_vm_writev (getsockname and getpeername fail with EOPNOTSUPP)."),
    (&["PODNET_TEST_UMASK"], "pod (tests only)", "A test hook of the unix listeners: the test of bind_private runs again in a new process with this umask (octal)."),
    (&["PODNET_PROBE_CHILD"], "pod (internal)", "Set by podnet for its own child process of a namespace probe (net, user-net or veth): the process runs the probe before main and exits with the result code. Do not set it."),
    (&["PODNET_NETNS_TEST_NO_NETNS"], "pod (tests only)", "A test hook of the netns backend: set and not empty, the netns backend cannot run. auto takes the seccomp backend, --backend netns exits 3, and pod doctor says fail."),
    (&["PODNET_NETNS_TEST_VETH"], "pod (tests only)", "A test hook of the netns backend: set and not empty, the namespace uses the veth wire (a veth pair and a packet socket) also when it can open /dev/net/tun."),
    (&["PODNET_WARP_API"], "warp (each command that calls the WARP API)", "The one base of the WARP API: https://HOST[:PORT] (http:// for a loopback host only). Without it, podnet tries https://api.cloudflareclient.com, then https://api.devices.cloudflare.com after a dial, TLS or 5xx failure. Another value is a configuration error (exit 78)."),
    (&["PODNET_WARP_TCP_BURST"], "warp proxy, warp run, warp up, agent up", "The TCP window of the userspace stack of WARP, in segments of the MTU: a number from 1 to 430, or none (no limit, the default): for a measurement of throughput. Another value is a configuration error (exit 78)."),
    (&["PODNET_WARP_MASQUE_TRANSPORT"], "warp (each command that opens a MASQUE session), warp up (its proxy)", "The value of --masque-transport: h2 (HTTP/2 on TCP 443, the default) or h3 (HTTP/3 on UDP 443; each IP packet is an HTTP datagram). h3 has no relay and no proxy."),
    (&["PODNET_WARP_TRANSPORT", "PODNET_WARP_WG_ENDPOINT"], "warp (each command that opens a session), warp up (its proxy)", "The value of --transport (masque, the default, or wireguard) and of --wireguard-endpoint (IP:PORT). wireguard carries the tunnel on UDP to the WireGuard front door (port 2408 of the record), with the device of warp export. The device is warp-wg.json of the state directory; podnet registers it when the file has none. It has no relay and no proxy."),
    (&["PODNET_WARP_CARRY", "PODNET_WARP_CARRY_ENDPOINT"], "warp (each command with a userspace stack), warp up (its proxy)", "The value of --carry (none, or wg) and of --carry-endpoint (IP:PORT). wg: a second WireGuard device goes inside the tunnel, on a UDP socket of the first stack. Its file is warp-carry.json of the state directory; podnet registers it when the file has none. the tools use the stack of the second device (IPv4, MTU 1200). Its exit is the exit of the second device. With --transport wireguard, both hops are WireGuard."),
    (&["PODNET_WARP_L4_FORM"], "warp proxy --l4", "The form of the CONNECT of the L4 door, for a test of the door: plain (RFC 9114, the default), extended (:protocol cf-connect-tcp) or header (cf-connect-proto: cf-connect-tcp)."),
    (&["PODNET_WARP_L4"], "warp proxy", "Set to true: the value of --l4. Each TCP client of the proxy is one HTTP/3 CONNECT stream to the L4 door of WARP on UDP 443, with no IP stack. --l4-sni gives the TLS name. UDP is not served."),
    (&["PODNET_WARP_SESSION_MAX_AGE"], "warp proxy, warp forward, warp run, warp up", "The value of --session-max-age: each MASQUE session is closed on purpose at this age (seconds), and a new one opens, with a new egress address. The TCP flows of the old session end. Off when not set."),
    (&["PODNET_WARP_H3_SHAPE"], "warp (each command with an HTTP/3 session), warp status, doctor", "The value of --h3-shape: the congestion profile of the HTTP/3 endpoint of `warp h3` (common, the default; cubic; newreno; bbr) and, with -no-control, no control stream. warp status and doctor report the shape. Another value is a configuration error (exit 78)."),
    (&["PODNET_WARP_TUIC"], "warp nc", "The value of --tuic: a TUIC version 5 server (QUIC) for `warp nc HOST PORT`, as tuic://UUID:PASSWORD@HOST:PORT?PARAM. allow_insecure=1 admits a self-signed certificate. This path uses no WARP identity. TCP is one stream. UDP is the bounded queue of the road, which stops reading stdin when it is full."),
    (&["PODNET_WARP_CC"], "warp (each command with a userspace stack), warp up (its proxy), tunnel --transport warp", "The value of --cc: the congestion controller of the TCP connections of the userspace stack of WARP, for the data that the stack sends: none (the default; only the window of the peer limits it), reno (RFC 5681) or cubic (RFC 9438). Another value is a configuration error (exit 78)."),
    (&["PODNET_WARP_NO_CHECK"], "warp proxy, warp run, warp up, warp probe, warp exit, warp nc, agent up", "Set and not empty: a new MASQUE session is used with no check of its data path. By default, one DNS query through the session must get an answer (5 s) before the session is up; else the session is closed and podnet tries again. For a measurement."),
    (&["PODNET_NETNS_QUIET"], "pod, warp exec, warp shell, warp run", "Set and not empty: the pod prints no warnings (errors stay). podnet -q sets it."),
    (&["PODNET_PROXY_USER", "PODNET_PROXY_PASSWORD_FILE"], "warp proxy, warp up", "The value of --proxy-user and --proxy-password-file: each client of the proxy must give this user and the password with SOCKS5 (RFC 1929) or HTTP Basic. The password is the first line of the file (mode 0600). A client with no or wrong credentials gets 0xff, 0x01 or 407."),
    (&["PODNET_DEVTUNNEL_TOKEN_FILE",
            "PODNET_DEVTUNNEL_TOKEN",
            "PODNET_DEVTUNNEL_SCHEME",
            "PODNET_DEVTUNNEL_ACCESS_TOKEN_FILE",
            "PODNET_DEVTUNNEL_ACCESS_TOKEN",
            "PODNET_DEVTUNNEL_ANONYMOUS",
            "PODNET_DEVTUNNEL_MANAGEMENT",], "the network devtunnel://TUNNEL (forward --network, a networks: entry of route)", "The credential of Microsoft Dev Tunnels: give one form. A user token (the management token): the first line of the file in PODNET_DEVTUNNEL_TOKEN_FILE, else the value of PODNET_DEVTUNNEL_TOKEN (give one of them); PODNET_DEVTUNNEL_SCHEME is github (default), aad or bearer. A tunnel access token that the owner of the tunnel gives: the first line of the file in PODNET_DEVTUNNEL_ACCESS_TOKEN_FILE, else the value of PODNET_DEVTUNNEL_ACCESS_TOKEN. No token, for a tunnel with anonymous access: PODNET_DEVTUNNEL_ANONYMOUS=1. More than one form is a usage error (exit 64) that names the variables. PODNET_DEVTUNNEL_MANAGEMENT replaces the base of the management API (https://HOST[:PORT], or http:// to a loopback host); the default is the fact of pod-services. A spec holds no token, and no token is in a log line, an error or --json. A missing token is a usage error (exit 64)."),
    (&["PODNET_TURN_USER",
            "PODNET_TURN_PASSWORD_FILE",
            "PODNET_TURN_PASSWORD",
            "PODNET_TURN_REALM",
            "PODNET_TURN_LIFETIME",
            "PODNET_TURN_RATE_LIMIT",], "the network turn://HOST[:PORT] and turns://HOST[:PORT] (forward --network, link p2p --relay, a networks: entry of route)", "The user, the password and the realm of the TURN server (RFC 8656). The password is the first line of the file in PODNET_TURN_PASSWORD_FILE, else the value of PODNET_TURN_PASSWORD (give one of them). The realm is optional: with it, a server of another realm is refused. PODNET_TURN_LIFETIME is the lifetime of each allocation in seconds (1 to 3600); PODNET_TURN_RATE_LIMIT is the rate limit of the relay in bytes for each second, which the status reports beside the lifetime. A spec holds no credential, and no credential is in a log line, an error or --json. A missing user or password is a usage error (exit 64). One account for each process."),
    (&["PODNET_HTTPS_PROXY_AUTH_FILE",
            "PODNET_HTTPS_PROXY_AUTH",
            "PODNET_HTTPS_PROXY_VERSION",], "the network https://[USER@]HOST[:PORT] (forward --network, a networks: entry of route, net doctor --network)", "The credentials of a forward proxy over TLS: the one line USER:PASSWORD of the file in PODNET_HTTPS_PROXY_AUTH_FILE, else the value of PODNET_HTTPS_PROXY_AUTH (give one of them). When the URL names the user, the line is the password alone. With neither, no Proxy-Authorization goes. PODNET_HTTPS_PROXY_VERSION is the versions of HTTP that the client offers: auto (the default: HTTP/2, else HTTP/1.1), 1.1 or 2. A network NAME of a router file reads the variables with the suffix _NAME when one of them is set. A spec holds no password, and no credential is in a log line, an error or --json. A bad value is a usage error (exit 64) that names the variable."),
    (&["PODNET_WSTUNNEL_SECRET_FILE",
            "PODNET_WSTUNNEL_SECRET",], "the network wstunnel:wss://HOST[:PORT] (forward --network, a networks: entry of route, net doctor --network)", "The secret of a wstunnel server: the text of the file in PODNET_WSTUNNEL_SECRET_FILE, else the value of PODNET_WSTUNNEL_SECRET (give one of them). The lines are path-prefix PREFIX, credentials USER[:PASSWORD] and authorization VALUE, or one line with the prefix alone. With neither, the client sends the defaults of wstunnel (the prefix v1). A secret over ws:// to a host that is not the loopback is a usage error. A network NAME of a router file reads the variables with the suffix _NAME when one of them is set. A spec holds no secret, and no secret is in a log line, an error or --json. A bad secret is a usage error (exit 64) that names the variable and the number of the line."),
    (&["PODNET_VLESS_ID_FILE",
            "PODNET_VLESS_ID",], "the network vless://HOST:PORT?... (forward --network, a networks: entry of route, net doctor --network)", "The ID of the user of a VLESS server: the text of the file in PODNET_VLESS_ID_FILE, else the value of PODNET_VLESS_ID (give one of them). The ID is a UUID, or a word of 1 to 30 bytes, which Xray-core maps to a UUID. The ID is required. A network NAME of a router file reads the variables with the suffix _NAME when one of them is set. A spec holds no ID, and no ID is in a log line, an error or --json. A missing or bad ID is a usage error (exit 64) that names the variable."),
    (&["PODNET_SHADOWSOCKS_KEY_FILE",
            "PODNET_SHADOWSOCKS_KEY",], "the network ss://HOST:PORT (forward --network, a networks: entry of route, net doctor --network)", "The key of a Shadowsocks server: the one line of the file in PODNET_SHADOWSOCKS_KEY_FILE, else the value of PODNET_SHADOWSOCKS_KEY (give one of them). The line is METHOD:PASSWORD, or the ss:// URI of the server (SIP002), which must name the host and the port of the spec. The methods are chacha20-ietf-poly1305, aes-256-gcm, aes-128-gcm and the three of Shadowsocks 2022. The key is required. A network NAME of a router file reads the variables with the suffix _NAME when one of them is set. A spec holds no key, and no key is in a log line, an error or --json. A missing or bad key is a usage error (exit 64) that names the variable."),
    (&["PODNET_WARP_DEVICE_NAME", "PODNET_WARP_DEVICE_MODEL", "PODNET_WARP_DEVICE_LOCALE"], "warp (each command that registers a device), tunnel --transport warp", "The value of --device-name, --device-model and --device-locale: what a new device declares at its registration (a name of 1 to 64 characters, a model of 1 to 32, a locale as en_US). With none, the request is as before (model PC, locale en_US, no name). A bad value is a usage error (exit 64). A saved device does not change."),
    (&["PODNET_WARP_FRAGMENT", "PODNET_WARP_FRAGMENT_SIZE"], "warp (each command that opens a MASQUE session), warp up (its proxy)", "The value of --fragment-client-hello (off, record or sni) and --fragment-size (N or MIN-MAX, 16 to 4096 bytes; default 64-160): the TLS ClientHello of the MASQUE connection goes in more than one record. sni ends the first record in the server name. A bad value is a configuration error (exit 78)."),
    (&["PODNET_WARP_RULES", "PODNET_WARP_RULES_FILE"], "warp proxy, warp up (its proxy reads them)", "The value of -r/--rule (comma-separated KIND:VALUE=ACTION) and of --rules-file: the rules of the destinations of the proxy. KIND is domain or cidr (IPv4, IPv6, or @FILE of prefixes); ACTION is tunnel, direct or reject. The file is read again when it changes. A bad rule is a usage error (exit 64)."),
    (&["PODNET_WARP_FORWARD"], "warp forward", "The value of -L/--local, comma-separated: LISTEN=HOST:PORT. Each connection to LISTEN (PORT on 127.0.0.1, ADDR:PORT, tcp://ADDR:PORT or unix:/PATH) goes through WARP to HOST:PORT; HOST is resolved inside the tunnel. A loopback target is a usage error (exit 64)."),
    (&["PODNET_WARP_RESOLVERS"], "warp proxy, warp up, warp run, warp tun, warp probe, warp nc, tunnel --transport warp", "The value of --resolver: the DNS resolvers inside the tunnel, comma-separated, in place of 1.1.1.1:53. IP[:PORT] is DNS on UDP; tls://IP[:PORT][#NAME] is DNS over TLS (RFC 7858, port 853), with the certificate checked for NAME or the IP address. The first that answers wins. warp tun --default-route writes the UDP ones on port 53 to resolv.conf."),
    (&["PODNET_UDP"], "tunnel, serve, share, demo, doctor", "QUIC on UDP 7844: off (no QUIC on UDP 7844), on (no probe), or probe (the default). With probe and --protocol auto, podnet sends one QUIC packet to the edge while it starts; when the send or the path refuses it, podnet does not try QUIC on UDP 7844. No answer keeps the order of the legs. Another value is a configuration error (exit 78). podnet doctor shows the quic leg refused with off; podnet net doctor always sends its probe."),
    (&["PODNET_CRASH_LOG_DIR"], "all commands", "The directory of crash.log. A panic writes the backtrace, the versions of podnet, podcrates, the system and the architecture, and the last 64 events there. Each secret is hidden at write time. Without it, a crash writes in the state directory, which it makes then. The write is bounded and rotated (LOG-1 to LOG-3)."),
    (&["PODNET_CRASH_LOG_MAX"], "all commands", "The limit of one crash log in bytes (default 65536). A longer text is cut, and the bound holds for each file."),
    (&["PODNET_CRASH_LOG_SLOTS"], "all commands", "The number of crash logs that a directory keeps (default 3): crash.log, crash.log.1 and so on. The oldest file goes at the next crash."),
    (&["PODNET_ERROR_PROGRAM"], "all commands", "A program that gets a JSON report on its stdin when podnet fails (exit code not 0). It runs with no shell and no arguments, for at most 30 s. The report has the version, the system, the command, the exit code, the error (\"error\": the error and its causes, once) and the log lines before it (\"messages\": at most 64), with credentials masked. A usage error (exit 64) sends a report too. The program does not get TUNNEL_TOKEN, the relay tokens, or a proxy variable with a user and a password."),
    (&["PODNET_VERIFY_STRICT"], "tunnel, serve, share, demo", "When set and not 0: a verify that only failed at name resolution, or at the CONNECT of a proxy, is an error (exit 69). By default the tunnel stays up, and the log says that the URL is not proved."),
    (&["RUST_LOG"], "all commands", "The log filter, for example podnet=debug. The default is podnet=info."),
    (&["PODNET_DEBUG"], "serve (SSH relay)", "When set, log each visitor channel of the SSH relay transport."),
    (&["PATH"], "pod, net shim", "podnet pod finds the program to run in PATH, and podnet net shim finds the C compiler."),
    (&["CC"], "net shim, net socksify, net passwd-shim, net ts-shims", "The C compiler that builds the LD_PRELOAD shims. The default is cc, then gcc, then clang."),
    (&["USER"], "net passwd-shim, net ts-shims", "The default login name of the synthetic user."),
    (&["LD_PRELOAD"], "net shim, net socksify", "The printed environment adds the shim in front of the current value."),
    (&["PODNET_SWITCH_TOKEN"], "net switch, net proxy --switch", "The shared token of a TCP switch, when --switch-token-file is not given. net switch --listen tcp://... needs it (else exit 64), and each TCP peer sends it first; a peer with no token or a wrong one is closed. A unix switch needs no token: it takes the same user only."),
    (&["PODNET_VNET_SHIM", "PODNET_SOCKS_SHIM"], "net shim, net socksify", "The path of a shim that is already built. podnet uses it instead of a build."),
    (&["PODNET_VNET_LOG", "PODNET_VNET_MAP_LOOPBACK", "PODNET_VNET_LOCAL_ADDR", "PODNET_VNET_PREFIX", "PODNET_VNET_HOSTS", "PODNET_VNET_CONTROL", "PODNET_VNET_MAX_FDS", "PODNET_VNET_SOCKS", "PODNET_VNET_SOCKS_ROUTES", "PODNET_VNET_FORWARD", "PODNET_VNET_HOST_NETLINK"], "libpodnet-vnet.so", "The shim of the virtual network: log each translation; map 127.0.0.1 to the local virtual address; the local virtual IPv4 address; the prefix of the abstract names (1 to 59 bytes of letters, digits, '.', '-' or '_'; net shim prints one for each run, and a value that cannot build a name is a usage error); a name=address table; the abstract name of the control socket; the size of the descriptor table; the unix socket of a SOCKS5 front door; the comma list of destinations (tailnet, ADDR, ADDR/BITS, IPV4:PORT, [IPV6]:PORT) whose stream connects go to that door (net shim --socks-route); the comma list of forwards IPV4:PORT=unix:/PATH or [IPV6]:PORT=unix:/PATH (net shim --forward, 16 at most). A stream connect to a forward goes to the unix socket PATH before a SOCKS route. With PODNET_VNET_HOST_NETLINK=1, the host answers NETLINK_ROUTE, getifaddrs and if_nameindex (by default the shim gives lo and vnet0 only)."),
    (&["PODNET_SOCKS_PROXY", "PODNET_SOCKS_LOG", "PODNET_SOCKS_ALL", "PODNET_SOCKS_CIDRS", "PODNET_SOCKS_MAX_FDS"], "libpodnet-socks.so", "The socks shim: the unix socket of the SOCKS5 front door; log each route and each redirect; redirect each connect; redirect only the destinations of a comma list (tailnet, ADDR, ADDR/BITS, IPV4:PORT, [IPV6]:PORT; net socksify --cidr) in place of the tailnet default; the size of the descriptor table."),
    (&["SANDHOME_PASSWD", "UNIXSOCKD_LOG"], "net passwd-shim, net ts-shims, unixsockd", "The passwd file that fakepwd.so reads; log each connection of unixsockd."),
    (&["PODNET_BUILD_COMMIT"], "the build", "Read when podnet is compiled: the commit that --version prints."),
    (&["SOURCE_DATE_EPOCH"], "the build", "Read when podnet is compiled: the date of the manual (podnet man --roff). Without it, the date of the commit."),
    (&["PODNET_S3_BUCKET"], "harness", "The bucket of the s3 harness case. The s3 rows are a measurement only when the fixture captures/s3.cap is present and this variable is set; otherwise they are a named skip."),
];

/// Each variable that podnet sets for a child, and does not read.
pub const SET_VARIABLES: &[VarRow] = &[
    (&["GATEWAY_INTERFACE", "SERVER_PROTOCOL", "SERVER_SOFTWARE", "REQUEST_METHOD", "QUERY_STRING", "REMOTE_ADDR", "CONTENT_TYPE", "CONTENT_LENGTH", "PATH_INFO", "REMOTE_USER", "AUTH_TYPE", "GIT_PROJECT_ROOT", "GIT_HTTP_EXPORT_ALL", "LC_ALL"], "git serve", "The CGI request for git http-backend: the method, the path, the query, the body, the user of a push and the root of the repositories. The child keeps PATH, HOME, LANG, LC_ALL, TZ and TMPDIR of podnet, and no other variable."),
    (&["GIT_PROTOCOL", "GIT_DIR"], "git serve", "The protocol version that an SSH client asks for, for git upload-pack and git receive-pack. podnet passes only a value that git can send. It removes GIT_DIR from the environment of each git child."),
    (&["GIT_ALLOW_PROTOCOL", "GIT_TERMINAL_PROMPT"], "forge clone", "Set for the git child of podnet forge clone: only the file protocol is allowed, so a seed cannot reach a network, and git does not prompt on a terminal."),
    (&["npm_config_proxy", "npm_config_https_proxy"], "warp up", "Set in the env file of podnet warp up, with the proxy variables and CARGO_HTTP_PROXY: the HTTP proxy of WARP, for npm."),
];

/// One file or directory: the path and the meaning.
type FileRow = (&'static str, &'static str);

/// Each file that podnet reads or writes.
pub const FILES: &[FileRow] = &[
    ("<state directory>/router-usage.json", "The usage of the quota of each door of podnet route that has one: the start of the period and the bytes, by the listen of the door. Mode 0600. podnet route writes it each 10 s while bytes pass, and at the end."),
    ("<state directory>/warp.json", "The WARP device, its token and its MASQUE key (mode 0600, directory mode 0700). The state directory is the first that podnet can write of PODNET_STATE_DIR, $XDG_STATE_HOME/podnet, $HOME/.local/state/podnet, $XDG_RUNTIME_DIR/podnet and podnet-<uid> in the temporary directory; never the current directory. podnet warp writes it; podnet warp delete removes it. --identity names another file."),
    ("$XDG_RUNTIME_DIR/podnet/warp-up.json, else $TMPDIR/podnet-<uid>/warp-up.json", "The pointer file of podnet warp up (mode 0600, directory mode 0700): its socket and its state directory. warp exec, warp shell, warp nc and agent read it last, so that they find a podnet warp up with another state directory. podnet reads it only when this user owns it and no other user can read it. podnet warp down removes it."),
    ("<state directory>/link-identity.key", "The Ed25519 key of this install for podnet link (mode 0600; a wider mode exits 71). podnet link listen, dial and status --create make it on first use. podnet link status prints its fingerprint. The seed is never printed, logged or put in JSON."),
    ("<run directory>/link-PID-NONCE.ctl, link-PID-NONCE.ctl.token (podnet link listen --control, link connect --control)", "The control socket of a link instance (mode 0600) and its token (32 random bytes in hex, mode 0600, new for each run). One JSON object for each line: status, peer, open. A request is 4 KiB at most, an answer 64 KiB, 16 clients; the user of the peer must be the user of the instance or root; a wrong token exits 77. The instance removes both files at its stop. There is no TCP listener and no HTTP endpoint."),
    ("ALLOW (podnet link listen --allow, podnet link dial --allow)", "An allow list of the link: one peer on each line (the fingerprint of podnet link status, a SHA256: fingerprint, or a key in hex), 4096 lines at most. A line that starts with # is a comment. The list is default deny. The file must belong to this user and must not be writable by others (exit 71); a missing file exits 66."),
    ("podnet-ca.pem (beside the binary)", "Extra TLS roots (PEM), for example the root of a TLS-inspecting proxy. podnet reads it when --ca-file and SSL_CERT_FILE are not set."),
    ("/etc/ssl/certs/ca-certificates.crt", "The first system bundle of TLS roots. When it is not there, podnet tries these files in this order, and reads the first that it can. The first is /etc/pki/tls/certs/ca-bundle.crt. The next is /etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem. The next is /etc/ssl/ca-bundle.pem. The next is /etc/ssl/cert.pem. The last is /usr/local/share/certs/ca-root-nss.crt."),
    ("config.yml (--config)", "A cloudflared configuration file: tunnel, credentials-file, ha-connections, protocol, ingress. The path: of an ingress rule is a regular expression, as in cloudflared; it matches a part of the request path (^/api/ for a prefix)."),
    ("~/.cloudflared/<tunnel-id>.json (--credentials-file)", "The credentials of a named tunnel, as cloudflared writes them."),
    ("/tmp/podnet-vnet.socks", "The default listen address of podnet net proxy."),
    ("/tmp/podnet-vnet.switch", "The default listen address of podnet net switch."),
    ("/tmp/podnet-socks.sock", "The default listen address of podnet net tailscale."),
    ("/tmp/podnet-warp-<pid>.sock", "The proxy socket of podnet warp run, for the life of the command."),
    ("$XDG_RUNTIME_DIR/podnet-netns-inbound/<port>.sock, else $TMPDIR/podnet-netns-inbound-<uid>/<port>.sock", "A port that a program under podnet pod listens on (--inbound-dir). The directory has mode 0700; a directory of another user, or one that the group or others can write, is refused."),
    ("$TMPDIR/podnet-vnet, $TMPDIR/podnet-socks, $TMPDIR/podnet-ts-shims", "The default build directories of the shims."),
];

/// The examples: the command line, then what it does. A test of the binary
/// parses each `podnet` line with the parser of the binary.
pub const EXAMPLES: &[(&str, &str)] = &[
    ("podnet doctor", "Test the path from this host to the edge; the last line names the step to take."),
    ("podnet agent up", "In a sandbox: test what works, start WARP when it is needed, and print the commands to use."),
    ("podnet tunnel --url http://127.0.0.1:8080 --detach", "A quick tunnel in the background: one JSON document with the URL, then exit 0."),
    ("podnet ps --json", "The instances that run in the background, as one JSON document."),
    ("podnet relay pair", "Ask the relay for a pair: a name, and the tokens of the node and of the operator."),
    ("podnet relay node --to 127.0.0.1:22", "Give the SSH server of this host to the operators of the pair, through the relay."),
    ("podnet derp node --to 127.0.0.1:22 --peer CLIENT-FINGERPRINT", "Give the SSH server of this host to one client key at a DERP server of the fleet. The node prints its derp: ticket; the client runs ssh -o ProxyCommand='podnet derp connect TICKET'."),
    ("podnet ssh-relay --socket /run/user/1000/origin.sock --detach", "Publish a unix socket through the default SSH relay (pinggy). The endpoint, the user and the URL rule are data of pod-services. The URL is in the ready document; podnet ps shows the URL after each reconnect."),
    ("podnet ssh-relay --service serveo --socket /run/user/1000/origin.sock --detach", "Publish a unix socket through a named SSH relay. podnet tries its plain SSH endpoints in order."),
    ("podnet ssh-relay --server relay.example.net:443 --user podnet --url-skip dashboard.example.net --socket /run/user/1000/origin.sock --detach", "Publish a unix socket through an SSH relay that pod-services does not know: give the server, the user and the URL rule."),
    ("podnet update --check", "Say whether a newer release exists. podnet update installs it after it checks the cosign signature, the entry of Rekor and the SHA-256."),
    ("podnet demo --exit-on-verify", "Make a quick tunnel to the built-in origin, and fetch its public URL."),
    ("podnet tunnel --url http://127.0.0.1:8080", "A quick tunnel to a local HTTP server."),
    ("podnet tunnel --unix /run/app.sock", "A quick tunnel to an HTTP server on a unix socket."),
    ("podnet tunnel --protocol http2 --url http://127.0.0.1:8080", "Use the HTTP/2 edge leg on TCP 7844 (no UDP)."),
    ("podnet tunnel --transport warp --hello-world", "Reach the edge through WARP on TCP 443 only."),
    ("podnet tunnel --token TOKEN --url http://127.0.0.1:8080", "A named tunnel."),
    ("podnet tunnel --config config.yml", "Ingress rules from a cloudflared configuration file."),
    ("podnet tunnel --forward unix:/run/app.sock", "Carry a raw stream through the quick tunnel."),
    ("podnet connect wss://NAME.trycloudflare.com/__podnet/ws -L tcp://127.0.0.1:8080", "The client side of --forward."),
    ("podnet serve --dir ./public", "Serve a directory."),
    ("podnet serve --dir ./public --local 127.0.0.1:8080", "Serve a directory to this host only, on a local listener. No tunnel."),
    ("podnet serve --dir ./public --local unix:/run/site.sock", "Serve a directory on a unix socket. No tunnel."),
    ("podnet git init --root ./repos app", "Make the bare repository ./repos/app.git."),
    ("podnet git serve --root ./repos --anon-read --push-user me --push-password-file ./pass", "Serve the repositories through a quick tunnel: anyone can clone, the user me can push."),
    ("podnet git serve --root ./repos --ingress relay --authorized-keys ~/.ssh/authorized_keys", "Serve the repositories over SSH through a relay pair: ssh -o ProxyCommand='podnet relay connect NAME'."),
    ("podnet git urls --root ./repos", "The clone URLs of each repository, and the fingerprint of the SSH host key."),
    ("podnet share --mode upload --password", "Receive files, with a PIN."),
    ("podnet share --mode proxy --unix /run/api.sock", "Share an HTTP server on a unix socket, behind a PIN."),
    ("podnet warp status", "The WARP device and account."),
    ("podnet warp proxy --listen tcp://127.0.0.1:40000", "A SOCKS5 and HTTP CONNECT proxy through WARP."),
    ("podnet warp run -- curl https://www.cloudflare.com/cdn-cgi/trace", "Run a program with its traffic through WARP."),
    ("podnet pod -x socks5://127.0.0.1:1080 -- curl https://example.com", "Run a program with all its traffic through a proxy."),
    ("podnet net proxy --forward 10.66.0.2:8080=unix:/run/app.sock", "A virtual address for a real service."),
    ("podnet send ./report.pdf", "Print a code such as 7-crossover-clockwork-adroitness, then send the file to the receiver of the code (magic-wormhole's mailbox and transit)."),
    ("podnet receive --out ~/Downloads", "Ask for the code, show the offer, and write the file or the directory under ~/Downloads. It never overwrites."),
    ("podnet net switch --listen tcp://127.0.0.1:7000 --switch-token-file /run/secrets/switch", "A switch on TCP. Each stack gives the same token: --switch-token-file, or PODNET_SWITCH_TOKEN."),
    ("podnet link listen /run/user/1000/app.sock --allow peers.txt --to unix:/run/user/1000/origin.sock --detach", "A link listener on a unix socket (mode 0600). A peer must prove its key and be in peers.txt."),
    ("podnet link dial /run/user/1000/app.sock --peer FINGERPRINT --check", "Connect to a link listener, check its key, and print its fingerprint."),
    ("podnet link connect /run/user/1000/app.sock --peer FINGERPRINT --control /run/user/1000/link.ctl --detach --name peer1", "Hold a link to a listener and run a control socket for it."),
    ("podnet forward -L 8080=peer:FINGERPRINT/http", "Open one stream of the link to the peer for each connection to 127.0.0.1:8080, for the service http of the peer. A running link connect or link listen instance holds the link (PODNET_LINK_CONTROL names its control socket)."),
    ("podnet status", "One JSON line about this host, for a program."),
    ("podnet man --roff", "This manual as a man(7) page."),
];

/// Each exit code of podnet. The constants of the codes are in
/// `podnet_core::exit`; a test of the binary compares the two.
pub fn exits() -> Vec<Exit> {
    use crate::exit::*;
    let row = |code: i32, name: &str, meaning: &str| Exit::new(code, name, meaning);
    vec![
        row(SUCCESS, "success", "The command completed. A server command (tunnel, serve, share, proxy) exits 0 when it is stopped with Ctrl-C or when --run-for ends."),
        row(FAILURE, "failure", "A failure that has no other code. With --check: a check of an origin or of the network failed (a bad flag or configuration has its own code). podnet doctor: no leg can work on this host. podnet bench: no request worked."),
        row(NO_BACKEND, "no-backend", "podnet pod and podnet warp run: no backend can run on this host, the program is not in PATH (or cannot run), or the setup of the backend failed. pod doctor gives 3 when no backend can run."),
        // EXIT_VIOLATION of podnet-netns (crates/podnet-netns/src/learn.rs).
        row(4, "violation", "podnet pod --fail-on-violation: the program exited 0, and a destination was not in the learned file or a rule refused it. The pod writes each violation on stderr."),
        row(USAGE, "usage", "A bad command line: an unknown command or flag, a missing or bad value. Nothing was done."),
        row(DATA_ERROR, "data-error", "Input that podnet refuses: podnet receive got an entry with a bad name or kind, or a name that exists (the receiver never overwrites). Also more files or bytes than its caps, or data whose size or SHA-256 does not agree. Nothing of a refused transfer stays on the disk."),
        row(NO_INPUT, "no-input", "A file that must exist is missing: the key of the link where a command needs an existing key, or the allow list of the link. warp status, license, rotate and delete: the WARP identity."),
        row(NO_HOST, "no-host", "The address of a peer does not resolve."),
        row(UNAVAILABLE, "unavailable", "A service or a network path is not available: no edge leg worked for podnet tunnel, podnet warp or another command that starts a session. Or the quick-tunnel service, the WARP API or the MASQUE endpoint did not answer. podnet doctor and podnet bench give 1 in place of 69."),
        row(OS_ERROR, "os-error", "A file of the host cannot be used: a key file or an allow list with a mode that lets other users change it (the key: any access), a symbolic link in place of the file, or a file of another user. The file is not replaced."),
        row(TEMPFAIL, "tempfail", "The quick-tunnel service refused a provision for now (HTTP 429): the error report (PODNET_ERROR_PROGRAM) has retry_after_s. Also: the edge refused the registration of a tunnel at two starts, and asks for a retry or does not know the new quick tunnel. Also: --detach lost the pipe to the command that started it. Try again later."),
        row(NO_PERM, "no-permission", "A peer is not in the allow list of the link or of a sealed relay session (default deny). Or a peer refused the key of this end: the link, relay --seal, a PresharedKey of wg:FILE. Nothing of a service was offered to it. podnet send and receive: the code did not match at the other end (the code is spent)."),
        row(CONFIG, "config", "A configuration that cannot be used: a variable of the environment such as PODNET_EDGE, or a configuration file given with --config. Also a tunnel token or a credentials file (as a flag or as a key of the file), or a roots file of --ca-file."),
        row(TIMEOUT, "timeout", "A time limit ended, as timeout(1) exits. --detach: the instance was not ready in --ready-timeout seconds (it is stopped). podnet send: no receiver took the code in --timeout seconds. podnet link: the hello did not end in 10 s."),
        Exit {
            code: Code::Signal,
            name: "signal".into(),
            meaning: "A second stop signal during the shutdown, or a signal that ended a program of podnet pod or podnet warp run: 128 and the number of the signal, as a shell gives.".into(),
        },
        Exit {
            code: Code::Payload,
            name: "program".into(),
            meaning: "podnet pod and podnet warp run give the exit code of the program that they run.".into(),
        },
    ]
}

/// Each variable as a fact: the variables of podnet, then the variables of
/// the pager of `pod-manual`.
pub fn variables() -> Vec<VarGroup> {
    let vars = VARIABLES.iter().map(|(names, used_by, meaning)| Var {
        used_by: used_by.to_string(),
        ..Var::reads(names, meaning)
    });
    let sets = SET_VARIABLES.iter().map(|(names, used_by, meaning)| Var {
        used_by: used_by.to_string(),
        ..Var::sets(names, meaning)
    });
    vec![
        VarGroup {
            name: String::new(),
            vars: vars.chain(sets).collect(),
        },
        pod_manual::pager::variables(),
    ]
}

/// The examples in the form of `pod-manual`.
pub fn examples() -> Vec<Example> {
    EXAMPLES
        .iter()
        .map(|(command, meaning)| Example {
            command: command.to_string(),
            meaning: meaning.to_string(),
        })
        .collect()
}

/// Each table of facts of podnet.
pub fn facts() -> Facts {
    let files = FILES.iter().map(|(path, meaning)| FileFact {
        path: path.to_string(),
        meaning: meaning.to_string(),
    });
    Facts {
        variables: variables(),
        exits: exits(),
        files: files.collect(),
        config: Vec::new(),
        tables: vec![super::inventory::table()],
    }
}
