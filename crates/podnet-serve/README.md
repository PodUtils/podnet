# podnet-serve

`podnet-serve` holds the services of
[podnet](https://github.com/PodUtils/podnet): the file server and the file
transfer, the gate of a shared page, the ingress rules, the origins, the
features beside the data path, the client of the SSH relay and git
hosting. It uses
`podnet-core`. It does not use the other crates of podnet.

The module `git::ssh` is an SSH server for git. It runs
`git upload-pack`, `git receive-pack` and `git upload-archive` of the
installed `git` for a repository under a root. A key of the
`authorized_keys` file can push. Only ed25519 and ECDSA keys are
accepted. With anonymous read, the user `anonymous` and each other key
can clone and fetch. The server refuses a shell, a pty and each port
forward. The command `podnet git serve` (T-094) uses it.

It is a library of podnet, not a stable interface. Use the crate `podnet`.

## License

0BSD. See [LICENSE](https://github.com/PodUtils/podnet/blob/main/LICENSE).
