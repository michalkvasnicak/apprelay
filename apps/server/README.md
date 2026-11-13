# AppRelay server

Two processes: a root `vpn-agent` owns WireGuard; the unprivileged application server manages it through a restricted Unix socket.

## Platform support

| Platform        | Status                                                                         |
| --------------- | ------------------------------------------------------------------------------ |
| macOS           | Implemented using `wireguard-go` and a dynamically allocated `utun` interface. |
| Linux           | VPN agent not implemented yet.                                                 |
| Other platforms | Not supported.                                                                 |

On macOS, `wireguard-go` and `wg` (from `wireguard-tools`) must be on the agent's `PATH`.

## Running on macOS

From `apps/server`, with `APPRELAY_VPN_PRIVATE_KEY_FILE` exported, build as your normal user and start the agent as root:

```sh
cargo build --bins
sudo env PATH="$PATH" \
  APPRELAY_VPN_PRIVATE_KEY_FILE="$APPRELAY_VPN_PRIVATE_KEY_FILE" \
  APPRELAY_VPN_AGENT_UID="$(id -u)" \
  APPRELAY_VPN_ADDRESS="${APPRELAY_VPN_ADDRESS:-10.77.0.1/24}" \
  APPRELAY_VPN_PORT="${APPRELAY_VPN_PORT:-51820}" \
  ./target/debug/vpn-agent
```

Keep the agent running. In another terminal, run `pnpm start` from `apps/server` **without sudo**. Stopping the server does not stop the VPN; stopping the agent cleans it up.

## Local PostgreSQL and HTTPS

See [local infrastructure](../../infra/README.md) for the root Docker Compose setup. PostgreSQL and Caddy run in containers; this server and the VPN agent remain native on the host. Caddy proxies HTTPS to this server on port 8080. PostgreSQL integration is not implemented yet.

### Health endpoint

Run `pnpm run server` from the repository root, then open `https://<configured-host>:8443/health` on your Mac or iPhone after trusting Caddy's CA. A successful response is HTTP 200 with `{"status":"ok"}`.

The native listener defaults to `127.0.0.1:8080`. Override it with the exported `APPRELAY_HTTP_ADDRESS` environment variable if needed for Colima host connectivity. For example, `APPRELAY_HTTP_ADDRESS=0.0.0.0:8080 pnpm run server` listens on all IPv4 interfaces; use firewall restrictions to avoid unwanted direct HTTP access. TLS is handled by Caddy, not this listener.

This is process liveness only: it does not verify PostgreSQL, VPN connectivity or WireGuard handshakes. An unavailable VPN agent is logged at startup but does not prevent HTTP serving.

## Configuration

Configuration comes from exported environment variables, not `.env` files. VPN settings below belong to the agent:

| Variable                        | Default        | Description                                                                          |
| ------------------------------- | -------------- | ------------------------------------------------------------------------------------ |
| `APPRELAY_VPN_PRIVATE_KEY_FILE` | Required       | Path to an existing WireGuard server private-key file.                               |
| `APPRELAY_VPN_ADDRESS`          | `10.77.0.1/24` | Server IPv4 address and prefix length. The server also adds a route for this subnet. |
| `APPRELAY_VPN_PORT`             | `51820`        | UDP listen port, between 1 and 65535.                                                |

`APPRELAY_VPN_AGENT_UID` selects the permitted non-root server user (defaults to `SUDO_UID`). On macOS, both processes use `/var/db/apprelay/vpn-agent.sock`; to override it, set `APPRELAY_VPN_AGENT_SOCKET` for both, including explicitly in the `sudo env` command. Its directory must be root-owned and not group/world-writable.

The private-key file must be a regular file with no group or other permissions, typically mode `600`. Generate it once and reuse it across restarts:

```sh
(umask 077; wg genkey > /path/to/server.key)
```

Keep the key outside version control. Only the agent needs access to it; it passes the key to `wg` through stdin.

## Current scope

The socket API supports status/start/stop; the server queries status on startup and serves `GET /health`. HTTP shuts down gracefully on SIGINT or SIGTERM. Peer provisioning, iOS client configuration, forwarding, and NAT are not implemented yet.
