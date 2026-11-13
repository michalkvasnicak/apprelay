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

## Production Linux firewall

Linux VPN agent support is still pending. The intended production policy is default-deny inbound and forwarded traffic, with only these exceptions:

| Source          | Destination | Purpose                                               |
| --------------- | ----------- | ----------------------------------------------------- |
| Public internet | TCP 443     | Caddy HTTPS, proxied to the HTTP service on port 8080 |
| Public internet | UDP 51820   | WireGuard tunnel (adjust if configured differently)   |
| VPN interface   | TCP 8080    | HTTP service on the server's VPN address              |

For example, on a dedicated host using UFW (replace `wg0` with the actual VPN interface):

```sh
sudo ufw default deny incoming
sudo ufw default deny routed
sudo ufw default allow outgoing
sudo ufw allow 443/tcp
sudo ufw allow 51820/udp
sudo ufw allow in on wg0 to 10.77.0.1 port 8080 proto tcp
sudo ufw enable
```

Before enabling, ensure console access or add a restricted SSH rule for your administrator IP to avoid locking yourself out. Apply the policy to IPv4 and IPv6 (`IPV6=yes` in `/etc/default/ufw`), and mirror public-port restrictions in the hosting provider's firewall. Preserve essential ICMP/ICMPv6 traffic for network operation.

Do not publicly expose TCP 8080 or PostgreSQL's TCP 5432. Bind PostgreSQL to loopback/private container networking and expose the HTTP listener only where Caddy and the VPN interface can reach it. Keep forwarding denied: VPN devices must not reach one another, private networks, or the internet through this server. Firewall ports do not provide HTTP route authorization.

Docker-published ports can bypass UFW's input rules; do not publish database/backend ports publicly, and enforce equivalent restrictions in Docker's forwarding path if using containers. Configure Caddy certificate issuance without inbound TCP 80 (for example, TLS-ALPN-01 or DNS-01) if keeping only the two public ports above.

## Configuration

Configuration comes from exported environment variables, not `.env` files.

| Variable                           | Process          | Default                                   | Description                                                                                                                                                                                                                                                                      |
| ---------------------------------- | ---------------- | ----------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `DATABASE_URL`                     | Server           | Required                                  | PostgreSQL connection URL, e.g. `postgres://apprelay:<URL-encoded-password>@127.0.0.1:5432/apprelay`.                                                                                                                                                                            |
| `APPRELAY_HTTP_ADDRESS`            | Server           | `127.0.0.1:8080`                          | HTTP listener address; TLS is handled by Caddy.                                                                                                                                                                                                                                  |
| `APPRELAY_DEVICE_LIFETIME_SECONDS` | Server           | `604800` (7 days)                         | Positive 32-bit integer lifetime for new or expired registrations. Active registrations keep their expiry; changing this does not alter existing deadlines.                                                                                                                      |
| `APPRELAY_VPN_PRIVATE_KEY_FILE`    | Agent            | Required                                  | Existing regular private-key file with no group/other permissions (typically `600`). Generate once with `(umask 077; wg genkey > /path/to/server.key)` and reuse across restarts. Keep outside version control; only the agent needs access and passes it to `wg` through stdin. |
| `APPRELAY_VPN_ADDRESS`             | Agent            | `10.77.0.1/24`                            | Server IPv4 address and prefix length; the agent also adds a route for this subnet.                                                                                                                                                                                              |
| `APPRELAY_VPN_PORT`                | Agent            | `51820`                                   | WireGuard UDP listen port, between 1 and 65535.                                                                                                                                                                                                                                  |
| `APPRELAY_VPN_AGENT_UID`           | Agent            | `SUDO_UID`                                | Permitted non-root server user's UID.                                                                                                                                                                                                                                            |
| `APPRELAY_VPN_AGENT_SOCKET`        | Server and agent | `/var/db/apprelay/vpn-agent.sock` (macOS) | Unix socket path. Set for both processes when overriding, including explicitly in `sudo env`. Its directory must be root-owned and not group/world-writable.                                                                                                                     |

## Current scope

The socket API supports status/start/stop; the server queries status on startup and serves `GET /health`. HTTP shuts down gracefully on SIGINT or SIGTERM. Peer provisioning, iOS client configuration, forwarding, and NAT are not implemented yet.
