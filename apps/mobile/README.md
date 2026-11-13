# AppRelay mobile

## WireGuard build configuration

Copy `.env.example` to `.env.local` in this directory and supply the Mac's Wi-Fi IPv4 address, UDP port, and WireGuard **public** key. Expo embeds `EXPO_PUBLIC_*` values in the JavaScript bundle; they are not secrets. Never put a server or device private key in these variables.

Obtain the server public key from the VPN agent's existing WireGuard private-key file on the Mac:

```sh
wg pubkey < /path/to/server.key
```

Do not generate a new server key just to configure the app. For release builds, provide the same variables in the build environment. Values are embedded when Expo bundles the app, not read from the phone's environment. Restart Expo after changing the local environment; rebuild/rebundle for distribution changes. Keep configuration consistent when publishing updates.

`src/vpn/config.ts` exposes `getVpnServerConfig()`, which validates these settings before native tunnel configuration:

| Variable                           | Meaning                                                        |
| ---------------------------------- | -------------------------------------------------------------- |
| `EXPO_PUBLIC_WG_SERVER_HOST`       | Mac Wi-Fi IPv4 endpoint, e.g. `192.168.1.42`. Required.        |
| `EXPO_PUBLIC_WG_SERVER_PORT`       | WireGuard UDP port, e.g. `51820`. Required.                    |
| `EXPO_PUBLIC_WG_SERVER_PUBLIC_KEY` | Existing server WireGuard public key. Required.                |
| `EXPO_PUBLIC_WG_SERVER_TUNNEL_IP`  | Server IPv4 address inside WireGuard. Defaults to `10.77.0.1`. |

### Single-server routing contract

The configuration produces exactly one peer `AllowedIPs` entry: `<server tunnel IP>/32`, normally `10.77.0.1/32`. It does not accept a user-supplied route list or prefix. The Wi-Fi endpoint carries encrypted WireGuard packets over the normal network; application requests to the private server use its tunnel IP.

The future native provider must use this same single-host route in both WireGuard's peer configuration and Network Extension's IPv4 included routes (subnet mask `255.255.255.255`). It must not enable `includeAllNetworks`, add default routes (`0.0.0.0/0` or `::/0`), add an IPv6 route, or install a VPN DNS override. Other destinations must retain their normal iOS routing. This provides destination-based routing, not per-app routing: any app contacting that private IP can use the tunnel.

### Implementation status

These settings do **not** establish a VPN yet. The native Swift/WireGuardKit packet tunnel extension, React Native bridge, signing/entitlements, and Expo config plugin are still required. The provider must enforce the routing contract above; TypeScript settings alone cannot change iOS routes.

The device must generate and securely store its own keypair, receive an unused tunnel address (e.g. `10.77.0.2/32`), and have its public key registered on the server with that address as the server-side peer `AllowedIPs`. The current server agent does not yet expose peer provisioning. Neither device private keys nor device addresses belong in shared build configuration.

Bonjour discovery is not implemented by these settings or by a basic WireGuard tunnel.
