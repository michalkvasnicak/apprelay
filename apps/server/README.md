# AppRelay server

Two processes: a root `vpn-agent` owns WireGuard; the unprivileged application server manages it through a restricted Unix socket.

## Platform support

| Platform        | Status                                                                         |
| --------------- | ------------------------------------------------------------------------------ |
| macOS           | Implemented using `wireguard-go` and a dynamically allocated `utun` interface. |
| Linux           | Implemented using kernel WireGuard and the dedicated `wg-apprelay` interface.  |
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

## Running on Linux

The agent requires Linux kernel WireGuard support, with `ip` (from `iproute2`) and `wg` (from `wireguard-tools`) on its `PATH`. On Ubuntu, install them with `sudo apt-get install iproute2 wireguard-tools`. If WireGuard is a loadable module, load it on the host with `sudo modprobe wireguard` before starting a container. No `wireguard-go` process or `/dev/net/tun` device is needed.

Use the same build and agent command as in the macOS section. The agent creates `wg-apprelay`, assigns `APPRELAY_VPN_ADDRESS`, and configures the connected subnet route and UDP listener. It refuses to take over an existing interface. Configuration failures roll back the newly created interface; graceful shutdown deletes it. An abrupt termination can leave the interface and agent socket behind: after confirming the old agent is no longer running, remove only its stale resources before restarting (`sudo ip link delete dev wg-apprelay` and `sudo rm /var/run/apprelay/vpn-agent.sock` for the default paths).

The default agent socket on Linux is `/var/run/apprelay/vpn-agent.sock`. The server and pairing worker must run under the non-root UID selected by `APPRELAY_VPN_AGENT_UID`. They must share the agent's network namespace to access the VPN address/interface, and have access to its Unix socket.

Linux pairing binds its TCP and unicast DNS-SD sockets to the VPN interface using `SO_BINDTODEVICE`, rather than macOS's `IP_BOUND_IF`. Provision `CAP_NET_RAW` for the non-root pairing worker; a binding failure aborts the session instead of falling back to another interface. For a native deployment, a service manager can grant this capability (for example, systemd `AmbientCapabilities=CAP_NET_RAW`). The API and notification worker do not need it. Pairing and notification workers and workflow dispatch are enabled on both platforms; their existing Temporal, PostgreSQL, encryption, and APNs settings still apply. No system Bonjour/Avahi service or USB access is used by this pairing flow.

For a future container deployment, the agent must run as root with `NET_ADMIN` and sufficient permissions to prepare/chown the Unix socket. Do not grant `SYS_MODULE` or full privileged mode: the kernel module belongs to the host. Docker packaging is not included here.

## Local PostgreSQL and HTTPS

See [local infrastructure](../../infra/README.md) for the root Docker Compose setup. PostgreSQL and Caddy run in containers; this server and the VPN agent remain native on the host. Caddy proxies HTTPS to this server on port 8080. PostgreSQL integration is not implemented yet.

The native listener defaults to `127.0.0.1:8080`. Override it with the exported `APPRELAY_HTTP_ADDRESS` environment variable if needed for Colima host connectivity. For example, `APPRELAY_HTTP_ADDRESS=0.0.0.0:8080 pnpm run server` listens on all IPv4 interfaces; use firewall restrictions to avoid unwanted direct HTTP access. TLS is handled by Caddy, not this listener.

## Production Linux firewall

The intended production policy is default-deny inbound and forwarded traffic, with only these exceptions:

| Source          | Destination     | Purpose                                                      |
| --------------- | --------------- | ------------------------------------------------------------ |
| Public internet | TCP 443         | Caddy HTTPS, proxied to the HTTP service on port 8080        |
| Public internet | UDP 51820       | WireGuard tunnel (adjust if configured differently)          |
| VPN interface   | TCP 8080        | HTTP service on the server's VPN address                     |
| VPN interface   | UDP 5354        | Device-specific unicast DNS-SD pairing discovery             |
| VPN interface   | TCP 20000–20049 | Up to 50 pairing listeners bound to the server's VPN address |

For example, on a dedicated host using UFW, the following allows HTTPS, WireGuard, and VPN HTTP only. Pairing additionally requires inbound UDP 5354 and TCP ports 20000–20049 on `wg-apprelay` to `10.77.0.1`; restrict any such rules to the VPN subnet and do not expose these ports publicly. The agent does not install firewall rules.

```sh
sudo ufw default deny incoming
sudo ufw default deny routed
sudo ufw default allow outgoing
sudo ufw allow 443/tcp
sudo ufw allow 51820/udp
sudo ufw allow in on wg-apprelay to 10.77.0.1 port 8080 proto tcp
sudo ufw enable
```

Before enabling, ensure console access or add a restricted SSH rule for your administrator IP to avoid locking yourself out. Apply the policy to IPv4 and IPv6 (`IPV6=yes` in `/etc/default/ufw`), and mirror public-port restrictions in the hosting provider's firewall. Preserve essential ICMP/ICMPv6 traffic for network operation.

Do not publicly expose TCP 8080 or PostgreSQL's TCP 5432. Bind PostgreSQL to loopback/private container networking and expose the HTTP listener only where Caddy and the VPN interface can reach it. Keep forwarding denied: VPN devices must not reach one another, private networks, or the internet through this server. Firewall ports do not provide HTTP route authorization.

Docker-published ports can bypass UFW's input rules; do not publish database/backend ports publicly, and enforce equivalent restrictions in Docker's forwarding path if using containers. Configure Caddy certificate issuance without inbound TCP 80 (for example, TLS-ALPN-01 or DNS-01) if keeping only the two public ports above.

## Configuration

Configuration comes from exported environment variables, not `.env` files.

| Variable                           | Process                 | Default                                   | Description                                                                                                                                                                                                                                                                                                                         |
| ---------------------------------- | ----------------------- | ----------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `REDIS_URL`                        | Server                  | Required                                  | Private Redis URL for shared registration and public pairing-attempt rate limits, e.g. `redis://127.0.0.1:6379/0`. Supports credentials and `rediss://` TLS; Redis must permit `EVALSHA`, `SCRIPT LOAD`, `GET`, `SET`, and `TIME` plus connection commands.                                                                         |
| `APPRELAY_TRUSTED_PROXY_CIDRS`     | Server                  | Empty (trust none)                        | Comma-separated CIDRs of trusted TCP proxy peers. Only these peers may supply `CF-Connecting-IP`, which must contain exactly one valid IP. Other peers are limited by their socket IP and forwarding headers are ignored. Trust only proxies that overwrite this header, never VPN/client networks. Required explicitly in Coolify. |
| `APPRELAY_API_KEY`                 | Server                  | Required                                  | 64-character hexadecimal app key required in the `X-App-Key` header on device API requests; public pairing-attempt creation and status do not require it. Must exactly match the mobile app's `EXPO_PUBLIC_API_KEY`; an extractable deterrent, not a replacement for device authentication.                                         |
| `DATABASE_URL`                     | Server and both workers | Required                                  | PostgreSQL connection URL, e.g. `postgres://apprelay:<URL-encoded-password>@127.0.0.1:5432/apprelay`.                                                                                                                                                                                                                               |
| `PAIRING_ENCRYPTION_KEY`           | Server and both workers | Required                                  | Base64-encoded random 32-byte encryption key. Generate once with `openssl rand -base64 32`; store securely and reuse across restarts and workers.                                                                                                                                                                                   |
| `APPRELAY_APNS_ENVIRONMENT`        | Server                  | Required                                  | APNs environment for uploaded push tokens: `sandbox` for development builds or `production` for TestFlight/App Store builds.                                                                                                                                                                                                        |
| `APPRELAY_APNS_SIGNING_KEY_FILE`   | Notification worker     | Required                                  | Path to the Apple APNs P-256 signing key (`.p8`, PKCS#8 PEM). Keep outside version control with no group/other permissions, e.g. `chmod 600 /path/to/AuthKey.p8`.                                                                                                                                                                   |
| `APPRELAY_APNS_KEY_ID`             | Notification worker     | Required                                  | Apple APNs signing key ID.                                                                                                                                                                                                                                                                                                          |
| `APPRELAY_APNS_TEAM_ID`            | Notification worker     | Required                                  | Apple Developer team ID.                                                                                                                                                                                                                                                                                                            |
| `APPRELAY_APNS_TOPIC`              | Notification worker     | Required                                  | App bundle ID authorized by the signing key; the topic is configured by the server, not supplied by the device.                                                                                                                                                                                                                     |
| `APPRELAY_TEMPORAL_ADDRESS`        | Server and both workers | `http://127.0.0.1:7233`                   | Temporal connection URL.                                                                                                                                                                                                                                                                                                            |
| `APPRELAY_TEMPORAL_NAMESPACE`      | Server and both workers | `default`                                 | Temporal namespace; it must exist on the Temporal server.                                                                                                                                                                                                                                                                           |
| `APPRELAY_HTTP_ADDRESS`            | Server                  | `127.0.0.1:8080`                          | HTTP listener address; TLS is handled by Caddy. Bind to `0.0.0.0:8080` to also accept direct VPN clients. Push token management and pairing-code retrieval require a direct VPN connection, not a reverse proxy.                                                                                                                    |
| `APPRELAY_DEVICE_LIFETIME_SECONDS` | Server                  | `604800` (7 days)                         | Positive 32-bit integer lifetime for new or expired registrations. Active registrations keep their expiry; changing this does not alter existing deadlines.                                                                                                                                                                         |
| `APPRELAY_VPN_PRIVATE_KEY_FILE`    | Agent                   | Required                                  | Existing regular private-key file with no group/other permissions (typically `600`). Generate once with `(umask 077; wg genkey > /path/to/server.key)` and reuse across restarts. Keep outside version control; only the agent needs access and passes it to `wg` through stdin.                                                    |
| `APPRELAY_VPN_ADDRESS`             | Agent                   | `10.77.0.1/24`                            | Server IPv4 address and prefix length; the agent also adds a route for this subnet.                                                                                                                                                                                                                                                 |
| `APPRELAY_VPN_PORT`                | Agent                   | `51820`                                   | WireGuard UDP listen port, between 1 and 65535.                                                                                                                                                                                                                                                                                     |
| `APPRELAY_VPN_AGENT_UID`           | Agent                   | `SUDO_UID`                                | Permitted non-root server user's UID.                                                                                                                                                                                                                                                                                               |
| `APPRELAY_VPN_AGENT_SOCKET`        | Server and agent        | `/var/db/apprelay/vpn-agent.sock` (macOS) | Unix socket path; Linux defaults to `/var/run/apprelay/vpn-agent.sock`. Set for the server, pairing worker, and agent when overriding, including explicitly in `sudo env`. Its directory must be root-owned and not group/world-writable.                                                                                           |

## API documentation

The OpenAPI specification is available at `/openapi.json`.
