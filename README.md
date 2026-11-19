<p align="center">
  <img src="apps/server/web/public/assets/logo.svg" alt="AppRelay logo" width="96" height="96" />
</p>

# AppRelay

AppRelay lets you share iOS development builds with testers and install them on a paired iPhone without Xcode. Upload an `.ipa` with the CLI to get a share link, or open an IPA directly in the browser to sign and install it on your own iPhone.

[Website](https://apprelay.app/) · [Sign an IPA](https://apprelay.app/sign) · [Get AppRelay VPN on TestFlight](https://testflight.apple.com/join/yW4QnZEg)

## Share a build

With Node.js 24.12 or newer, run:

```sh
npx @brosoft/apprelay ./MyApp.ipa
```

Send the returned link to your testers. Share links expire after **7 days**. Each tester signs the build for their own iPhone before installing it.

See the [CLI documentation](packages/cli/README.md) for installation and custom server options.

## Sign and install on your iPhone

1. On an iPhone running **iOS 27 or later**, enable **Developer Mode** under **Settings → Privacy & Security**, then restart and confirm the setting.
2. Install [AppRelay VPN from TestFlight](https://testflight.apple.com/join/yW4QnZEg). Connect the VPN and keep the iPhone on Wi-Fi during pairing and installation. The TestFlight invite is for the VPN companion, not the build you’re installing.
3. Open the build’s share link, or open [the signing page](https://apprelay.app/sign) and choose a local IPA. Follow the browser prompts to pair your iPhone.
4. Sign in with your Apple Account to sign the IPA in the browser, then install it on the paired iPhone over the VPN. Free and paid accounts both work, but Apple applies stricter provisioning limits to free accounts.

**Supported builds:** IPAs up to **100 MiB compressed** and **256 MiB unpacked**. Apps with extensions, embedded frameworks, custom capabilities, or encrypted binaries aren’t supported.

## Repository layout

| Component                                        | What it does                                                                                                                     |
| ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------- |
| [iOS companion](apps/mobile/README.md)           | Expo and React Native app in TypeScript, with a native Swift WireGuard tunnel. Connects the iPhone for pairing and installation. |
| [Backend](apps/server/README.md)                 | Rust and Axum API, background workers, and a host VPN agent for sharing, pairing, and installation.                              |
| [Web frontend](apps/server/web/)                 | React, TypeScript, Vite, and Tailwind CSS for the homepage and browser-based signing workspace.                                  |
| [Upload CLI](packages/cli/README.md)             | TypeScript CLI published as `@brosoft/apprelay`. Uploads an IPA and prints its share URL.                                        |
| [Sample iOS app](apps/hello-world-ios/README.md) | Standalone SwiftUI app you can build into a sample IPA with Xcode.                                                               |
| [Local infrastructure](infra/README.md)          | Docker Compose setup for PostgreSQL, Redis, Temporal, and Caddy.                                                                 |

## Local development

Use **Node.js 24.12+**, the **pnpm 10** version pinned in `package.json`, and **Rust 1.92+** with Clippy and rustfmt.

```sh
pnpm install
```

- **Backend:** Follow the [local infrastructure guide](infra/README.md) and [server setup](apps/server/README.md) for configuration and VPN prerequisites. Run `pnpm server` to start the API. The native listener defaults to `http://127.0.0.1:8080`, with `/health` for health checks and `/openapi.json` for the API contract.
- **Browser:** Copy `apps/server/web/.env.example` to `apps/server/web/.env.local`. Set `VITE_APPRELAY_APP_SCHEME` to match the mobile app’s `EXPO_PUBLIC_APP_SCHEME` (`apprelay-dev` in the local examples). Run `pnpm web:dev` for Vite at `http://localhost:5173`; it proxies API requests to the local Rust server. `pnpm web:build` outputs to `apps/server/web/dist`, which Axum can serve. Use `APPRELAY_WEB_DIR` to override that path.
- **iPhone:** Follow the [mobile setup guide](apps/mobile/README.md) for environment files, native build prerequisites, and TestFlight publishing. Use `pnpm ios` to build for a physical iPhone. The VPN and pairing workflow requires a signed native build; Expo Go and the simulator are not supported. Xcode is needed to build the companion app locally, not to use the hosted service.
- **CLI:** Run `pnpm cli ./MyApp.ipa` from the checkout. To target a different backend, see the [CLI server configuration](packages/cli/README.md#server-configuration).

### API contracts and checks

Rust request and response types and `#[utoipa::path]` annotations define the OpenAPI contract. After changing it, run `pnpm api:generate` and commit the generated TypeScript schemas for the mobile app, web frontend, and CLI with the Rust changes. No running backend is needed for generation.

- `pnpm api:check` checks for stale API types and is included in `pnpm typecheck`.
- `pnpm lint` runs workspace linters, including Clippy for Rust.
- `pnpm format` and `pnpm format:check` write or verify formatting.

## License

[MIT](LICENSE)
