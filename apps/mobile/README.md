# AppRelay mobile

## iOS WireGuard VPN

The UI provides Connect/Disconnect, system tunnel status, an editable device VPN address and a selectable device public key. This is an **iOS-only custom native build**, not Expo Go. Android/web display an unsupported-platform message; simulator builds are intentionally unsupported.

### Build configuration

Copy `.env.example` to `.env.local` in this directory. Set `APPLE_TEAM_ID` to your Apple Developer Team ID. Optionally set `APPLE_BUNDLE_IDENTIFIER` to your own app identifier; otherwise it defaults to `sk.brosoft.apprelay` from `app.json`. `app.config.ts` reads these native build settings without an `EXPO_PUBLIC_` prefix. If `APPLE_TEAM_ID` is empty, no team is configured by the app config.

Also supply the Mac's Wi-Fi IPv4 address, UDP port and existing WireGuard **public** key. Expo embeds `EXPO_PUBLIC_*` values in the JavaScript bundle; they are not secrets. Never put server or device private keys in these variables. `.env.local` is ignored by Git; provide the same signing settings in your CI/build environment when building there.

Obtain the server public key on the Mac using its existing private-key file:

```sh
wg pubkey < /path/to/server.key
```

| Variable                           | Meaning                                                        |
| ---------------------------------- | -------------------------------------------------------------- |
| `EXPO_PUBLIC_WG_SERVER_HOST`       | Mac Wi-Fi IPv4 endpoint, e.g. `192.168.1.42`. Required.        |
| `EXPO_PUBLIC_WG_SERVER_PORT`       | WireGuard UDP port, e.g. `51820`. Required.                    |
| `EXPO_PUBLIC_WG_SERVER_PUBLIC_KEY` | Existing server WireGuard public key. Required.                |
| `EXPO_PUBLIC_WG_SERVER_TUNNEL_IP`  | Server IPv4 address inside WireGuard. Defaults to `10.77.0.1`. |

Release builds need these variables in their bundling environment too. Restart Expo after local changes; rebuild/rebundle for distributed changes, including updates. Values are not read from the phone's environment at runtime.

### Native build prerequisites

- Xcode and a physical iPhone supported by the deployment target in `app.json`.
- An Apple Developer team with Network Extension provisioning enabled for both the app and extension. Configure `APPLE_TEAM_ID` and, if needed, `APPLE_BUNDLE_IDENTIFIER` in `.env.local` before provisioning. The Expo config plugin uses these resolved settings for both targets.
- Both targets need the `packet-tunnel-provider` entitlement and the same keychain access group. The plugin generates these. The extension bundle identifier is `<app bundle identifier>.PacketTunnel`.
- Go **1.19.13**, matching the pinned upstream WireGuardKit bridge's runtime patch. With a host Go installation, install it once:

  ```sh
  go install golang.org/dl/go1.19.13@latest
  "$HOME/go/bin/go1.19.13" download
  ```

  The build looks in `$HOME/go/bin`, Homebrew locations and `PATH`. Alternatively supply an absolute `APPRELAY_GO_BINARY` path to the pinned compiler/wrapper in the Xcode build environment. The bridge's Go dependencies also need to be downloadable on the first build. Modern system Go versions are deliberately rejected rather than patched incorrectly.

For an initial build, run from the repository root:

```sh
pnpm ios
```

The script uses `expo run:ios --device`. Prebuild invokes `plugins/with-wireguard.js`, which adds the native bridge, the embedded packet-tunnel target, its shared keychain/signing configuration, a revision-pinned WireGuardKit Swift package, and a Go bridge build phase. No edits to generated `ios/` files are required. After changing plugin settings or bundle identifiers, regenerate with a clean Expo prebuild before building. The plugin also declares the extension to EAS for credential provisioning; EAS/custom CI must separately install the pinned Go toolchain before the Xcode build.

The official WireGuard Apple sources are pinned to revision `2fec12a6e1f6e3460b6ee483aa00ad29cddadab1`. Its upstream Go toolchain is old; upgrading/auditing the native dependencies is a separate production-readiness task.

### Register the device on the Mac

1. Open the native app. It generates a per-installation X25519 keypair and shows **only its public key**. The private key stays in the shared Keychain, accessible after first unlock, and is never sent to JavaScript or stored in VPN preference dictionaries.
2. Assign a unique device tunnel IPv4 address, for example `10.77.0.2`, and enter it in the UI. This is not the iPhone's Wi-Fi address. The address is remembered after successful VPN configuration.
3. The server agent currently has no peer-provisioning API. Register the displayed public key manually against the agent's current interface:

   ```sh
   sudo env PATH="$PATH" wg set utun6 peer '<device-public-key>' allowed-ips 10.77.0.2/32
   ```

   Replace `utun6` with the interface printed by the running agent. Repeat registration after restarting the agent; this command changes runtime state, not persistent agent configuration. Never reuse a device address for another peer.

4. Allow UDP 51820 (or your configured port) through the Mac's firewall. Both devices must have Wi-Fi reachability without client isolation. No router port forwarding is needed on the same LAN.
5. Tap **Connect** and approve iOS's VPN configuration prompt. Tap **Disconnect** to stop it. The tunnel runs in the extension independently of React Native while the app is suspended; UI status refreshes when the app returns to the foreground and while it is active.

“Tunnel active” reports iOS provider state, **not a successful WireGuard handshake**. A missing peer, wrong server public key, blocked UDP port or unreachable Mac can leave a running tunnel with no usable connection. Check handshakes on the Mac and reach a service at `10.77.0.1:<port>` to establish end-to-end connectivity. The service must listen on the tunnel address or a suitable wildcard, not just loopback.

### Single-server routing

The peer allows exactly `<server tunnel IP>/32`, normally `10.77.0.1/32`. The native provider validates the public settings itself and derives the route instead of accepting arbitrary routes from JavaScript. It also replaces WireGuardKit's generated Network Extension routes with exactly that server host route on every network-settings application, including path changes.

There are no default routes, IPv6 routes, DNS overrides, on-demand rules or `includeAllNetworks` enforcement. The client's interface address also uses `/32`. Traffic to other destinations retains normal iOS routing; the Mac's Wi-Fi endpoint carries only the encrypted transport over the normal network.

This is destination-based routing, not per-app routing: any app contacting the private server IP can use the tunnel. Bonjour discovery is not implemented.

### Validation status

No native build, signing, device connection or routing experiment has been run as part of this implementation. Repository instructions reserve checks for automation. Physical-device validation must cover consent, registered-peer handshakes, server-service access, unchanged routing/DNS for other destinations, disconnect, background/lock behavior and Wi-Fi changes before treating this as verified on-device.
