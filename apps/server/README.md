# AppRelay server

Rust service that manages a WireGuard interface: startup, key and port configuration, IPv4 addressing, subnet routing, and cleanup on shutdown or failure.

## Platform support

| Platform        | Status                                                                         |
| --------------- | ------------------------------------------------------------------------------ |
| macOS           | Implemented using `wireguard-go` and a dynamically allocated `utun` interface. |
| Linux           | Not implemented yet; the server exits with an unsupported-platform error.      |
| Other platforms | Not supported.                                                                 |

On macOS, `wireguard-go` and `wg` (from `wireguard-tools`) must be available on the server process's `PATH`. Running the server requires root privileges to manage interfaces, routes, and runtime files under `/var/run`.

## Running on macOS

The server must run as root. From `apps/server`, build as your normal user, then run the binary with `sudo`:

```sh
cargo build
sudo -E env PATH="$PATH" ./target/debug/apprelay-server
```

This requests preservation of exported configuration variables and retains your `PATH` for Homebrew tools. Environment preservation is subject to sudo policy. Do not run Cargo or pnpm as root.

## Configuration

Configuration is read from environment variables:

| Variable                        | Default        | Description                                                                          |
| ------------------------------- | -------------- | ------------------------------------------------------------------------------------ |
| `APPRELAY_VPN_PRIVATE_KEY_FILE` | Required       | Path to an existing WireGuard server private-key file.                               |
| `APPRELAY_VPN_ADDRESS`          | `10.77.0.1/24` | Server IPv4 address and prefix length. The server also adds a route for this subnet. |
| `APPRELAY_VPN_PORT`             | `51820`        | UDP listen port, between 1 and 65535.                                                |

The private-key file must be a regular file with no group or other permissions, typically mode `600`. Generate it once and reuse it across restarts:

```sh
(umask 077; wg genkey > /path/to/server.key)
```

Keep the key outside version control. The server passes it to `wg` through stdin, not command-line arguments.

## Current scope

The server monitors `wireguard-go` and cleans up on Ctrl+C, SIGTERM, startup failure, or daemon exit. Peer provisioning, iOS client configuration, traffic forwarding, and NAT are not implemented yet; devices cannot connect until peers are configured.
