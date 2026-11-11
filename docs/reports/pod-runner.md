# podnet pod with the runner of actions/runner and CI payloads (T-229)

The output of `scripts/pod-runner-measure.sh`. Do not edit it by hand: run the script again.

* podnet: `podnet 0.4.0-beta.2 (unknown)`; backend `seccomp`; kernel `6.18.44-fc-v338`.
* Payloads: the runner `v2.338.2` (the release asset of `actions/runner`), `actions/checkout` `v4.2.2` and `actions/setup-node` `v4.4.0` (their `dist` bundles, run by `node`).
* Each payload runs in its own pod: `--default-action reject`, the rules `domain:.github.com`, `domain:.githubusercontent.com` and `domain:.nodejs.org` to the witness proxy, and `-x` the witness proxy (`scripts/pod-runner-proxy.py`, HTTP CONNECT on a Unix socket).
* Three witnesses: the flow log of the pod, the CONNECT lines of the witness proxy, and strace (each `connect`, `bind`, `sendto` and `sendmsg` with an address).

## Summary

| Payload | Exit | Flows (flow log) | Refused | CONNECT (proxy) | Checks |
| --- | --- | --- | --- | --- | --- |
| runner-help | 0 | 0 | 0 | 0 | pass |
| runner-config | 1 | 3 | 0 | 3 | pass |
| runner-run | 0 | 0 | 0 | 0 | pass |
| node-fetch | 0 | 1 | 0 | 1 | pass |
| checkout | 0 | 3 | 0 | 3 | pass |
| setup-node | 0 | 3 | 0 | 3 | pass |

A check passes when each CONNECT of the witness proxy has a flow of the flow log with the verdict `allow` (the same targets, the same counts), and no flow goes to a fake address of the pod.

## runner-help

`config.sh --help` of the runner (.NET).

Flows of the flow log (target, rule, verdict):

* none

Other ways (strace, with the count):

* 1 x bind unix socket `/tmp/dotnet-diagnostic-PID-socket`

## runner-config

`config.sh --unattended` with a token that is not valid: the registration is refused.

Flows of the flow log (target, rule, verdict):

* `tcp` `api.github.com:443`: `domain:.github.com=PROXY`, `allow`
* `tcp` `api.github.com:443`: `domain:.github.com=PROXY`, `allow`
* `tcp` `api.github.com:443`: `domain:.github.com=PROXY`, `allow`

Other ways (strace, with the count):

* 3 x bind netlink (local: the addresses and routes of the host)
* 1 x bind unix socket `/tmp/dotnet-diagnostic-PID-socket`
* 3 x connect DNS to 8.8.8.8:53 (the pod answers it)
* 3 x connect a fake address of the pod, port 443 (the pod holds it)
* 6 x connect of a UDP socket to a fake address, port 0 (an address choice: no packet)
* 2 x connect unix socket `/var/run/nscd/socket`
* 3 x sendto netlink (local: the addresses and routes of the host)

## runner-run

`run.sh` of a runner that has no configuration.

Flows of the flow log (target, rule, verdict):

* none

Other ways (strace, with the count):

* 1 x bind unix socket `/tmp/dotnet-diagnostic-PID-socket`

## node-fetch

`fetch` of Node.js to the API of GitHub.

Flows of the flow log (target, rule, verdict):

* `tcp` `api.github.com:443`: `domain:.github.com=PROXY`, `allow`

Other ways (strace, with the count):

* 1 x bind netlink (local: the addresses and routes of the host)
* 1 x connect DNS to 8.8.8.8:53 (the pod answers it)
* 1 x connect a fake address of the pod, port 443 (the pod holds it)
* 2 x connect unix socket `/var/run/nscd/socket`
* 1 x sendto netlink (local: the addresses and routes of the host)

## checkout

`actions/checkout`, run by `node`: a clone of PodUtils/podnet with depth 1.

Flows of the flow log (target, rule, verdict):

* `tcp` `github.com:443`: `domain:.github.com=PROXY`, `allow`
* `tcp` `github.com:443`: `domain:.github.com=PROXY`, `allow`
* `tcp` `github.com:443`: `domain:.github.com=PROXY`, `allow`

Other ways (strace, with the count):

* 1 x bind netlink (local: the addresses and routes of the host)
* 1 x connect DNS to 8.8.8.8:53 (the pod answers it)
* 3 x connect a fake address of the pod, port 443 (the pod holds it)
* 2 x connect of a UDP socket to a fake address, port 443 (an address choice: no packet)
* 2 x connect unix socket `/var/run/nscd/socket`
* 1 x sendto netlink (local: the addresses and routes of the host)

## setup-node

`actions/setup-node`, run by `node`: Node.js 20 into a tool cache.

Flows of the flow log (target, rule, verdict):

* `tcp` `api.github.com:443`: `domain:.github.com=PROXY`, `allow`
* `tcp` `nodejs.org:443`: `domain:.nodejs.org=PROXY`, `allow`
* `tcp` `nodejs.org:443`: `domain:.nodejs.org=PROXY`, `allow`

Other ways (strace, with the count):

* 4 x bind netlink (local: the addresses and routes of the host)
* 3 x connect DNS to 8.8.8.8:53 (the pod answers it)
* 3 x connect a fake address of the pod, port 443 (the pod holds it)
* 6 x connect unix socket `/var/run/nscd/socket`
* 5 x sendto netlink (local: the addresses and routes of the host)
