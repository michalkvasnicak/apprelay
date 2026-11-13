# AppRelay mobile

## iOS WireGuard VPN

The UI provides Connect/Disconnect, system tunnel status, API-assigned device settings and a selectable device public key. This is an **iOS-only custom native build**, not Expo Go. Android/web display an unsupported-platform message; simulator builds are intentionally unsupported.

### Build configuration

Copy `.env.example` to `.env.local` in this directory. Set `APPLE_TEAM_ID` to your Apple Developer Team ID and `APPLE_BUNDLE_IDENTIFIER` to the app identifier for this build. The bundle identifier is required; there is no production fallback. `app.config.ts` reads these native build settings without an `EXPO_PUBLIC_` prefix. If `APPLE_TEAM_ID` is empty, no team is configured by the app config.

Set `EXPO_PUBLIC_VPN_API_URL` to the VPN server's HTTPS base URL, for example `https://vpn.example.com`. The app appends `/v1/devices`. Credentials, query strings and fragments are not accepted. Expo embeds this public URL in the JavaScript bundle; never put private keys in environment variables. `.env.local` is ignored by Git.

Release builds need these variables in their bundling environment too. Restart Expo after local changes; rebuild/rebundle for distributed changes, including updates. Values are not read from the phone's environment at runtime.

### Native build prerequisites

- Xcode and a physical iPhone supported by the deployment target in `app.json`.
- An Apple Developer team with Network Extension provisioning enabled for both the app and extension. Configure `APPLE_TEAM_ID` and `APPLE_BUNDLE_IDENTIFIER` in `.env.local` or the build environment before provisioning. The Expo config plugin uses these resolved settings for both targets.
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

The official WireGuard Apple sources are pinned to revision `2fec12a6e1f6e3460b6ee483aa00ad29cddadab1`. Prebuild fetches that exact revision into `.expo/wireguard-apple/` (Git and network access are required on the first run), then copies the package sources and license into generated `ios/AppRelayVPN/WireGuardKit/`. The plugin raises the package's `swift-tools-version` from 5.3 to 5.5: upstream uses `.macOS(.v12)` and `.iOS(.v15)`, which are unavailable in PackageDescription 5.3. The plugin also adds the missing `<sys/types.h>` include to `WireGuardKitC.h` so its Darwin typedefs are available during explicit Clang module compilation. Xcode uses this patched local package, and the Go build uses the same source tree. No Xcode dependency-cache edits are needed. Its upstream Go toolchain is old; upgrading/auditing the native dependencies is a separate production-readiness task.

After pulling this manifest fix with an existing generated iOS project, run from the repository root:

```sh
pnpm --filter @apprelay/mobile exec expo prebuild --platform ios --no-install
pnpm ios
```

The incremental prebuild replaces the previous remote package reference with the patched local package and refreshes the Go build script. `expo run:ios` alone does not necessarily rerun prebuild when `ios/` already exists.

### Device registration and connection

Connect posts the device's base64 WireGuard public key to `POST /v1/devices` over HTTPS and requires `201 Created`. The private key stays in the shared Keychain and is never sent to JavaScript or the API. The response supplies the device ID, IPv4 `/32` address, expiry, server key, DNS/IPv4 UDP endpoint, IPv4 allowed routes and keepalive interval. The device address is no longer manually editable.

Registration uses a TanStack Query mutation with automatic retries disabled: the API contract does not guarantee idempotency. A successful response is reused in memory until `expires_at`, including after a tunnel-start failure. Reconnecting after expiry or restarting the app registers again. There is no persistent registration cache, background renewal or client-enforced expiry disconnect; server-side peer expiry must be enforced by the VPN server. `lifetime_seconds` is validated as metadata; `expires_at` determines reuse.

Approve iOS's VPN prompt on first connection. The API and WireGuard endpoint must be reachable from the phone, and the server must actually implement peer provisioning; the Rust process in this repository is unchanged. No authentication beyond the supplied public-key contract is added.

Identity and status use TanStack Query queries. Status polls once per second in the foreground and refreshes on app activation; Connect/Disconnect use mutations. No component `useEffect` is needed. The tunnel runs independently in the extension while JavaScript is suspended.

“Tunnel active” reports iOS provider state, **not a successful WireGuard handshake**. Verify handshakes and service reachability on the server.

### Split routing

The native provider validates the API settings and applies exactly the returned IPv4 allowed routes, including after network-path changes. Subnets such as `10.77.0.0/24` are supported; default routes and IPv6 settings are deliberately rejected. The client interface uses `/32`. DNS is unchanged, and other destinations retain normal iOS routing. This is destination-based routing, not per-app routing; any app contacting an allowed destination can use the tunnel.

Native source changes require regenerating the Expo iOS project and rebuilding the app; a JavaScript-only update is insufficient.

### Validation status

No native build, signing, device connection or routing experiment has been run as part of this implementation. Repository instructions reserve checks for automation. Physical-device validation must cover consent, registered-peer handshakes, server-service access, unchanged routing/DNS for other destinations, disconnect, background/lock behavior and Wi-Fi changes before treating this as verified on-device.
