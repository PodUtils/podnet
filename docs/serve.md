# File server, file transfer and git hosting

This page gives `podnet serve` (a directory), `podnet share` (one transfer with a PIN)
and `podnet git` (git repositories). Each one has a local listener, a public quick tunnel
and the relay as roads to the visitors.

## Serve a directory

```sh
podnet serve ./public                   # DIR, or --dir DIR
podnet serve ./app --spa                # send index.html for unknown paths
podnet serve ./public --local 127.0.0.1:8080      # this host only, no tunnel
podnet serve ./public --local unix:/run/site.sock # a unix socket, no tunnel
podnet serve ./public --local 127.0.0.1:8080 --ingress tunnel  # both
podnet serve ./public --ingress relay --pair NAME     # through a relay pair
```

`--ingress` gives the road to the visitors:

* `tunnel` (the default with no `--local`): a public quick tunnel, or the
  named tunnel of `--token`.
* `relay`: the node of a relay pair (`podnet relay pair`). `--pair NAME`
  names the pair; the default is the pair with the most time left. The
  files listen on a private unix socket in the run directory, and the node
  connects each session of an operator to it. The operator runs
  `podnet relay connect NAME --listen tcp://127.0.0.1:8080` and opens
  `http://127.0.0.1:8080/`.
* `none` (the default with `--local`): only the local listeners.

`--local ADDR` (more than one is permitted) serves the same files on a
local listener:

* `ADDR` is `HOST:PORT`, `tcp://HOST:PORT` or `unix:PATH`.
* Port 0 takes a free port. podnet prints the bound address
  (`podnet: serve DIR on http://127.0.0.1:PORT/`). With `--json`, the
  `ready` line has `url` and `listen`.
* A unix socket has mode 0600. A socket that answers is refused (exit
  69; `--replace` takes it over). A stale socket is removed. A file that
  is not a socket is refused (exit 64). podnet removes its socket at the
  end.
* An address in use is refused (exit 69).
* `--check` is refused with `--local`, and is for `--ingress tunnel` only.
  With `--ingress none`, the tunnel flags have no effect, except `--auth-user`
  (see below). The `--json` ready line has `listens`, each listener.

`podnet serve` serves only the files in `--dir`:

* A symlink is served only when its real path is in `--dir`.
* A name that starts with a dot (`.git`, `.env`) is not served.
* Only regular files and directories are served (no FIFO, socket or
  device).

Each other path answers 404. With `--spa`, an unknown path answers the
`index.html` with status 200.

With `--tar`, `GET /PATH/?tar` gives the directory `PATH` as one ustar
archive (`application/x-tar`, `PATH.tar`). The archive has the same rules:
no dot name, no link out of `--dir`, no FIFO, socket or device. A link to a
directory is not followed. A name that does not fit in ustar (more than 255
bytes) is left out with a warning. Without `--tar`, `?tar` has no effect.

```sh
podnet serve --dir ./public --tar --local 127.0.0.1:8080
curl -fsS 'http://127.0.0.1:8080/docs/?tar' | tar -xf -
```

With `--list`, a directory with no `index.html` answers a listing of its
files. A browser gets HTML. A request with `Accept: application/json` gets
`{"path": "/docs/", "entries": [{"name", "type", "size", "modified"}]}`,
with `type` `dir` or `file` and `modified` in seconds after the Unix epoch.
The listing has the same rules: no dot name, no link out of `--dir`, no
FIFO, socket or device. A directory path with no `/` at its end goes to
the path with the `/` (308).

```sh
podnet serve --dir ./public --list --local 127.0.0.1:8080
curl -fsS -H 'Accept: application/json' http://127.0.0.1:8080/docs/
```

### Users, roles and a PIN

The file server checks its visitors itself, on each listener (`--local`,
the tunnel and the relay):

* `--users FILE`: the users (HTTP Basic). Each line is
  `USER:ROLE:HASH` or `USER:ROLE:@PASSWORD-FILE`. `ROLE` is `read` or
  `write`; a write user can also read. A relative `PASSWORD-FILE` starts at
  the directory of `FILE`. `FILE` and each password file must have mode
  0600. An empty line and a line that starts with `#` are skipped.
* `--auth-user USER --auth-password-file FILE`: one more user, with the
  write role.
* `--pin`: the files are locked with a 6-digit PIN, as `share --password`.
  podnet prints the PIN (`pin` in the `--json` line). With no users, the
  PIN gives the write role.
* `--anon-read`: a GET or HEAD needs no user and no PIN. A change still
  needs a write user (or the PIN).

A request with no user or a wrong password gets 401 with a Basic
challenge. A read user that sends a change gets 403.

`podnet serve --hash-password` reads a password from the first line of
stdin and prints its hash (`pbkdf2-sha256$ITERATIONS$SALT$KEY`, PBKDF2 with
HMAC-SHA256, hex):

```sh
printf '%s\n' "$PASSWORD" | podnet serve --hash-password
# pbkdf2-sha256$210000$...$...
install -m 600 /dev/null users
echo "alice:read:$(printf '%s\n' "$PASSWORD" | podnet serve --hash-password)" >> users
podnet serve --dir ./public --list --users users --local 127.0.0.1:8080
curl -fsS -u alice http://127.0.0.1:8080/
```

### Uploads

With `--upload`, a write user (or a visitor with the PIN) can add files:

* `PUT /PATH` writes the body to the file `PATH` (`curl -T`). A new file
  gets 201, a replaced file 204. The directory of `PATH` must exist (else
  409).
* A multipart form `POST` to a directory adds each file of the form to the
  directory. A name in use gets a number (`name-1.ext`); no file is
  replaced. The listing of `--list` shows the form to a write user.
* A file goes to a hidden part file (mode 0600) first. A rename gives it
  its name when it is complete: a reader never sees a part of a file.
* `--max-upload BYTES` (default 100000000) is the most bytes of one
  request. A larger `Content-Length` gets 413 before podnet reads the
  body. A larger body with no `Content-Length` gets 413 at the limit. No
  file stays.
* A name that starts with a dot (also `..`), a link out of `--dir` and a
  read user are refused (403 or 409).

```sh
podnet serve --dir ./share --list --upload --users users --local 127.0.0.1:8080
curl -fsS -u bob -T report.pdf http://127.0.0.1:8080/inbox/report.pdf
```

The library type `podnet::origin::StaticDir` has the same rules. It also
applies them to the directory index and to the SPA index.
`podnet::feature::StaticServer::respond` gives a `StaticResponse`. Its
`body` is a reader (`std::io::Read`) that reads the file when you read
it, and `write_to` writes the head and the body. A file of any size is
served with a bounded memory.

## Git hosting

`podnet git` serves the git repositories of a root with the installed
`git` (`git http-backend`, `git upload-pack`, `git receive-pack`):

```sh
podnet git init --root ./repos app      # the bare repository ./repos/app.git
podnet git serve --root ./repos --anon-read \
  --push-user me --push-password-file ./pass   # smart HTTP through a quick tunnel
podnet git serve --root ./repos --local 127.0.0.1:8080 --ssh 127.0.0.1:2222 \
  --users users --authorized-keys ~/.ssh/authorized_keys --ingress none
podnet git serve --root ./repos --ingress relay --pair NAME \
  --authorized-keys ~/.ssh/authorized_keys     # SSH through a relay pair
podnet git urls --root ./repos          # the clone URLs and the host key
```

- A fetch needs the read role: `--anon-read`, or a user of `--users`. A
  push needs the write role: `--push-user`, a write user of `--users`, or
  a key of `--authorized-keys` (ed25519 and ECDSA only).
- `--create-on-push`: a push of a write user to a new path makes a bare
  repository there.
- The SSH host key is ed25519. podnet makes it one time
  (`git/ssh_host_ed25519_key` in the state directory, or `--host-key`).
- The operator of a relay pair clones with
  `GIT_SSH_COMMAND="ssh -o ProxyCommand='podnet relay connect NAME'" git clone ssh://git@NAME/REPO`.
- Limits: the quick tunnel takes at most 100 MB in one request body, so a
  larger push over HTTPS fails; a clone has no limit. A session through
  the relay ends at 64 MiB, so a larger clone or push through the relay
  fails.
- `podnet doctor` reports a missing `git`.

## Share files

```sh
podnet share --mode upload --password                 # receive files, with a PIN
podnet share --mode download --file ./report.pdf      # send one file
podnet share --file ./photos                          # send a directory as one tar
podnet share --file ./report.pdf --local 127.0.0.1:8080  # no tunnel
podnet share --mode proxy --port 3000                 # share a local port, with a PIN
podnet share --mode proxy --unix /run/api.sock        # share an HTTP unix socket, with a PIN
podnet share --mode upload --dir /tmp/inbox --stop-after --idle-secs 600
```

* Give a mode: `--mode upload` (or `--dir DIR`), `--file FILE`, `--port
  PORT` or `--unix PATH`. A bare `podnet share` is a usage error (exit 64),
  and opens no session. With `--mode upload` and no `--dir`, the files go
  to `~/Downloads/podnet` (or `~/podnet-uploads` when `~/Downloads` does not
  exist).
* The public path is `/s/<token>/`. The token has 32 random bytes, and a
  path with another token answers 404.
* `--password` adds a 6-digit PIN. The host shows the PIN. The PIN check uses
  a constant-time compare. After 10 wrong codes, no code opens the session:
  start a new one. Requests at the same time get at most 10 compares in
  total. A correct code sets the count of wrong codes to 0. After the
  unlock, the browser goes back to a path of the same site only.
* The unlock cookie has `HttpOnly`, `Secure` and `SameSite=Lax`: open the
  session with `https://` (a browser does not keep a `Secure` cookie of
  plain http). An unlock POST from another site (`Sec-Fetch-Site`, or an
  `Origin` that is not the host) gets 403 and uses no try. A client that
  knows the URL can still use the 10 tries: the edge does not give podnet
  the address of the visitor, so the limit is for the session, not for
  each visitor.
* A locked GET or HEAD gets the PIN page with 200 (for a browser). A locked
  request with another method (an upload POST, an API call) gets the page
  with 403, and nothing is done. A wrong PIN gets 403.
* The upload limits are 512 MiB for each file, 32 files and 4 GiB in total
  for each session. A request over a limit gets 413, and no file of that
  request is saved: each file waits in a hidden `.part` file until the
  request ends. A body with no data for 60 s gets 408 and leaves no file.
* An uploaded file has mode 0600 (only the user of podnet can read it),
  with each umask.
* One upload request holds at most `--max-request` bytes (default
  100000000, the limit of the Cloudflare edge for one request body). Each
  field of the request counts, also a field with no file. A
  larger `Content-Length` gets 413 and saves nothing: podnet reads the body
  and drops it first (until its end, a pause of 5 s, or 120 s), because an
  answer before the end of the upload does not reach the visitor reliably
  through the edge. The page refuses such a selection before it sends. To
  send more, use more uploads.
* A directory in `--file` goes as one ustar stream (`DIR.tar`), with the
  rules of `serve --tar`: no dot name, no link out of the directory, no
  FIFO, socket or device.
* `--local ADDR` serves the session on a local listener (HOST:PORT,
  tcp://HOST:PORT or unix:PATH, as `serve --local`) and makes no tunnel.
  podnet prints the session URL (`url` and `path` in the `--json` line).
  It is for upload and download; proxy mode needs the tunnel.
* A download name that is not ASCII goes in `filename*=UTF-8''...` (RFC
  5987), with an ASCII `filename=`.
* `--stop-after` stops the session after the first transfer. `--idle-secs`
  stops it after a time with no activity. The idle clock starts when the
  session serves, not before the provisioning.
* Each reply of a share session has `Cache-Control: no-store`. In proxy
  mode, the replies of the local server keep their own headers.
* A download answers `If-None-Match: *` and a weak tag (`W/"..."`) with
  304.
