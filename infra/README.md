# Local infrastructure

Docker Compose runs PostgreSQL, Redis, Temporal, and Caddy. The Rust API, pairing worker,
and root VPN agent stay native on macOS; containers need no VPN privileges or
server private key.

```text
iPhone / Mac -> Caddy :8443 -> host.lima.internal:8080 -> native Rust API
                                                         | PostgreSQL via 127.0.0.1:5432
                                                         | VPN agent via host Unix socket
```

This is a development configuration, not a production deployment. Caddy will
return 502 until the native server is running and reachable at its upstream.

## Start

From the repository root, with Colima running and its Docker context selected:

```sh
cp .env.example .env
# Edit .env and choose a local PostgreSQL password.
docker compose up -d
docker compose ps
docker compose logs -f
```

The default HTTPS URL is `https://localhost:8443`. PostgreSQL is published only on `127.0.0.1:5432`; development Redis is published only on `127.0.0.1:6379`. Redis rate-limit counters are ephemeral. Only HTTPS TCP is published for Caddy; its administration port and HTTP/3 UDP are not exposed.

Named volumes persist PostgreSQL data, Temporal's SQLite database, Caddy
certificates and its local CA. Services are independent: Caddy does not need
PostgreSQL or Temporal to start.

## Temporal

The pinned `temporalio/temporal:1.6.2` image runs `server start-dev` with persistence
in `temporal-data`. Its gRPC API is exposed only at `127.0.0.1:7233` and its UI at
<http://localhost:8233>. The development server creates the `default` namespace.
This is not a production Temporal deployment and has no authentication.

Run the pairing worker natively: it needs the host's WireGuard interface and
VPN-agent Unix socket. See the [server startup guide](../apps/server/README.md#run-locally)
for commands and required environment settings.

## Native application configuration

Start the native HTTP server from the repository root:

```sh
pnpm run server
```

After trusting Caddy's CA, open `https://<configured-host>:8443/health`. Expect HTTP 200 with `{"status":"ok"}`. This verifies the HTTPS proxy path, not PostgreSQL or VPN readiness. The VPN agent need not be running for this endpoint.

The listener defaults to `127.0.0.1:8080`; export `APPRELAY_HTTP_ADDRESS` to change it. If Colima cannot reach loopback, select a reachable interface and restrict access using the firewall. A 502 response indicates Caddy cannot reach the native API.

The API and pairing worker connect to PostgreSQL using:

```text
DATABASE_URL=postgres://apprelay:<URL-encoded-password>@127.0.0.1:5432/apprelay
```

Export application settings in the shell running each native process. Compose's
root `.env` is for Compose interpolation; native Rust processes do not load it
automatically.

Caddy connects to `host.lima.internal:8080`, not container localhost. This hostname relies on Colima/Lima DNS; do not override it with Docker's `host-gateway`, which may point at the Linux VM rather than macOS. The API listener must be reachable through this host route. Depending on Colima's networking, loopback may work; otherwise use a reachable interface and firewall restrictions. Do not expose an unauthenticated API broadly as a workaround.

`APPRELAY_API_UPSTREAM` can override the upstream without modifying the Caddyfile.

## Physical iPhone and Colima networking

1. Give the Mac a stable LAN IPv4 address, preferably using a DHCP reservation.
2. Set both values in the root `.env` to that address:

   ```dotenv
   APPRELAY_HTTPS_HOST=192.168.1.42
   APPRELAY_HTTPS_BIND=192.168.1.42
   ```

3. Enable specific-host-address publishing in Colima. In `colima start --edit`, preserve the existing configuration and set:

   ```yaml
   network:
     hostAddresses: true
   ```

   If Colima is already running, stop it first, then start with the edited configuration. This restarts the VM and affects other containers. This setting enables publishing to a specific Mac IP; it is not needed for Caddy's outbound connection to the host.

4. Run `docker compose up -d` again to apply the environment and port binding.
5. Allow TCP 8443 through the Mac firewall. The iPhone and Mac need LAN connectivity without Wi-Fi client isolation.
6. Use `https://192.168.1.42:8443` as the eventual mobile API URL.

Bind Caddy to the chosen LAN IP rather than all interfaces. PostgreSQL remains loopback-only. If the Mac's IP changes, update both values and recreate Caddy. The published LAN port should be checked from another device; container startup alone does not establish reachability.

WireGuard continues listening directly on the host's UDP port (normally 51820). Compose does not publish or manage that port.

## Trust the development HTTPS certificate

Caddy uses its internal CA, so no public domain or ACME certificate is required. Export **only the root certificate** after Caddy starts:

```sh
docker compose cp caddy:/data/caddy/pki/authorities/local/root.crt /tmp/apprelay-local-ca.crt
```

On macOS, optionally trust it using Keychain Access, or explicitly:

```sh
sudo security add-trusted-cert -d -r trustRoot \
  -k /Library/Keychains/System.keychain /tmp/apprelay-local-ca.crt
```

Transfer the root certificate to your iPhone through a trusted channel, install its configuration profile in Settings, then enable full trust under **Settings > General > About > Certificate Trust Settings**. Trusting it on the Mac does not trust it on the iPhone.

Do not export the CA private key, commit certificates, or disable TLS verification. Installing this root grants trust to certificates issued by this development CA; remove it when no longer needed.

Keep `caddy-data` across container recreation so the CA stays stable. Losing that volume generates a new CA that clients must trust again. Caddy's admin endpoint is not a public container port.

## Stop and reset

```sh
docker compose down
```

This keeps all data. **Destructive reset**, only when intentional:

```sh
docker compose down --volumes
```

This deletes PostgreSQL data, Temporal workflow history, and the Caddy CA. You
must trust the replacement CA on clients afterward.

PostgreSQL initialization settings only apply to an empty data volume. Changing `POSTGRES_PASSWORD` in `.env` does not change the password in an existing database; change that role's password explicitly or intentionally reset the local database.

## Production boundary

Production needs separate credentials, backups, a public domain and publicly trusted TLS certificates. Do not reuse this internal-CA configuration or publish PostgreSQL publicly. API authentication, device storage and provisioning are subsequent implementation steps.
