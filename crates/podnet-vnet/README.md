# podnet-vnet

`podnet-vnet` is the userspace virtual network of
[podnet](https://github.com/PodUtils/podnet) (`podnet net`): the addresses,
the smoltcp stack, the switch, the policy, the virtual resolver, the SOCKS5
and HTTP `CONNECT` proxy, and the `LD_PRELOAD` shims. The C sources of the
shims are in `shim/`. It uses the crates `podnet-core` and `podnet-netns`.

The module `wg_quick` reads a wg-quick file (`[Interface]` and `[Peer]`)
into a WireGuard peer. A `Reserved` field of a `[Peer]` gives bytes 1 to
3 of each message. With no `Reserved` field, these bytes are zero. The
private key comes from the file only.

It is a library of podnet, not a stable interface. Use the crate `podnet`,
which gives this crate at its old path (`podnet::vnet`).

## License

0BSD. See [LICENSE](https://github.com/PodUtils/podnet/blob/main/LICENSE).
