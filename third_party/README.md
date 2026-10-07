# Third-party patches

## librqbit-tracker-comms 9.0.1

Upstream: https://github.com/ikatson/rqbit (Apache-2.0).

Patch: `UdpTrackerClient::new` binds an IPv6 (dual-stack) UDP socket
unconditionally. On kernels built or booted without IPv6 support this fails
with `EAFNOSUPPORT` and the whole torrent session refuses to start. The patched
copy falls back to an IPv4 socket. No other changes.
