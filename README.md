# apprelay

pnpm monorepo with a minimal Expo React Native app and a Rust server process.

## Layout

- `apps/mobile` — Expo app (`@apprelay/mobile`) with a blank white screen.
- `apps/server` — Rust HTTP server (`@apprelay/server`) and privileged host VPN agent.
- `packages/*` — workspace location for future shared packages.
- Repository root — shared tooling, Git hooks, and Pi hooks.

## Development

Use Node.js 24 and pnpm 10 (`packageManager` pins the pnpm version).

Native iOS builds require iOS 27.0 or newer, configured through `expo.ios.deploymentTarget` in `apps/mobile/app.json`. This minimum applies to native builds, not the Expo Go host app.

```sh
pnpm install
pnpm start
```

Root development commands forward to `apps/mobile`. Open the app in Expo Go, or use `pnpm ios` / `pnpm android` to launch a simulator or emulator. `pnpm web` starts the web app.

Use `pnpm --filter @apprelay/mobile <command>` to target the app directly. Add Expo dependencies with `pnpm --filter @apprelay/mobile exec expo install <package>`.

### Local infrastructure

[Local infrastructure setup](infra/README.md) runs PostgreSQL and Caddy HTTPS in Docker Compose, including Colima networking and iPhone certificate trust. The application server and VPN agent remain native on the host. Start with `cp .env.example .env`, edit the local settings, then run `docker compose up -d`.

### Rust server

Install Rust 1.85 or newer with the Clippy and rustfmt components, then run:

```sh
pnpm run server
```

The server listens on `127.0.0.1:8080` by default and serves `GET /health` with `{"status":"ok"}`. Set `APPRELAY_HTTP_ADDRESS` to override the listener. With Caddy running and its CA trusted, use `https://<configured-host>:8443/health` to verify HTTPS. Health does not require the VPN agent or PostgreSQL. `pnpm --filter @apprelay/server start:release` runs an optimized build.

## Code quality

- `pnpm lint` — run each workspace's lint script (the mobile app uses oxlint with React rules, autofixes, and warnings treated as errors).
- `pnpm typecheck` — TypeScript checking for mobile and `cargo check` for the server.
- The server's lint script runs Clippy with warnings treated as errors.
- `pnpm format` / `pnpm format:check` — oxfmt across the repository plus rustfmt for the server.
- Husky runs lint-staged before commits, formatting and re-staging supported staged files with oxfmt or rustfmt.
- Pi runs lint and typecheck automatically before settling. See [`.pi/README.md`](.pi/README.md).

No navigation, application features, or tests are included.
