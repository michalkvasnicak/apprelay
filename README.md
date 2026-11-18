# apprelay

pnpm monorepo with a minimal Expo React Native app and a Rust server process.

## Layout

- `apps/mobile` — Expo app (`@apprelay/mobile`) with a blank white screen.
- `apps/server` — Rust HTTP server (`@apprelay/server`) and privileged host VPN agent.
- `apps/server/web` — React + TypeScript + Vite signing frontend (`@apprelay/web`), styled with Tailwind CSS.
- `packages/cli` — TypeScript IPA upload CLI (`@brosoft/apprelay`), requiring Node.js 24.12+.
- Repository root — shared tooling, Git hooks, and Pi hooks.

## CLI

```sh
npx @brosoft/apprelay ./MyApp.ipa
# From the repository, before npm publication:
pnpm cli ./MyApp.ipa
```

CLI configuration and usage: [packages/cli/README.md](packages/cli/README.md).

## Development

Use Node.js 24 and pnpm 10 (`packageManager` pins the pnpm version).

Native iOS builds require iOS 27.0 or newer, configured through `expo.ios.deploymentTarget` in `apps/mobile/app.json`. This minimum applies to native builds, not the Expo Go host app.

```sh
pnpm install
cp apps/mobile/.env.local.example apps/mobile/.env.local
# Fill in apps/mobile/.env.local before starting.
pnpm start
```

Root development commands forward to `apps/mobile`. Open the app in Expo Go, or use `pnpm ios` / `pnpm android` to launch a simulator or emulator. `pnpm web` starts the Expo web app.

For separate local/TestFlight configuration and local iOS publishing with `pnpm publish:ios:testflight`, see [the mobile README](apps/mobile/README.md).

Use `pnpm --filter @apprelay/mobile <command>` to target the app directly. Add Expo dependencies with `pnpm --filter @apprelay/mobile exec expo install <package>`.

### Browser signing frontend

Copy `apps/server/web/.env.example` to `apps/server/web/.env.local` and set
`VITE_APPRELAY_APP_SCHEME` to the same value as `EXPO_PUBLIC_APP_SCHEME` in your
mobile env file (`apprelay-dev` in the local mobile example). Then run
`pnpm web:dev` to open the signing preview at `http://localhost:5173`.
Vite proxies API requests to the Rust server at `127.0.0.1:8080`; start it separately with `pnpm server`.
Apple authentication, trusted-device 2FA, and basic certificate/profile preparation are experimental and not live-verified. IPA signing is not integrated yet.
Optional sessions and signing assets use a passphrase-encrypted localStorage vault. Apple passwords and verification codes are never saved.

`pnpm web:build` produces `apps/server/web/dist`, which Axum serves at `/`.
Set `APPRELAY_WEB_DIR` to override the asset directory. The production Docker image builds and includes these assets; its build context is the repository root.
API documentation is available at `/openapi.json`.

### Local infrastructure

[Local infrastructure setup](infra/README.md) runs PostgreSQL and Caddy HTTPS in Docker Compose, including Colima networking and iPhone certificate trust. The application server and VPN agent remain native on the host. Start with `cp .env.example .env`, edit the local settings, then run `docker compose up -d`.

### Rust server

Install Rust 1.85 or newer with the Clippy and rustfmt components, then run:

```sh
pnpm run server
```

The server listens on `127.0.0.1:8080` by default and serves `GET /health` with `{"status":"ok"}`. Set `APPRELAY_HTTP_ADDRESS` to override the listener. With Caddy running and its CA trusted, use `https://<configured-host>:8443/health` to verify HTTPS. Health does not require the VPN agent or PostgreSQL. `pnpm --filter @apprelay/server start:release` runs an optimized build.

## Synchronized API types

Rust request/response structs and `#[utoipa::path]` annotations are the source of truth.
The server serves the OpenAPI contract at `GET /openapi.json` (no authentication required).

After changing an API request, response, or endpoint:

```sh
pnpm api:generate
```

This exports the specification from Rust and regenerates
`apps/mobile/src/api/schema.d.ts`. It requires the Rust toolchain, but no running
server, database, Redis, or VPN agent. Commit the regenerated file with the Rust changes.

The mobile app uses `openapi-fetch` through `apps/mobile/src/api/client.ts` for
typed paths, parameters, request bodies, and responses. Types provide compile-time
checks, not runtime response validation.

For new endpoints, derive `utoipa::ToSchema` on their payload types, add
`#[utoipa::path]` to the handler, register it in
`apps/server/src/http/openapi.rs`, and wire it into the Axum router.
Document authentication and status codes alongside the endpoint.

`pnpm api:check` fails if generated types are stale without modifying files.
It also runs as part of `pnpm typecheck` and the automatic Pi checks.

## Code quality

- `pnpm lint` — run each workspace's lint script (the mobile app uses oxlint with React rules, autofixes, and warnings treated as errors).
- `pnpm typecheck` — TypeScript checking for mobile and `cargo check` for the server.
- The server's lint script runs Clippy with warnings treated as errors.
- `pnpm format` / `pnpm format:check` — oxfmt across the repository plus rustfmt for the server.
- Husky runs lint-staged before commits, formatting and re-staging supported staged files with oxfmt or rustfmt.
- Pi runs lint and typecheck automatically before settling. See [`.pi/README.md`](.pi/README.md).

The signing frontend is experimental. IPA signing, capability/extension handling, and installation are not yet implemented.
