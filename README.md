# apprelay

pnpm monorepo with a minimal Expo React Native app and a Rust server process.

## Layout

- `apps/mobile` — Expo app (`@apprelay/mobile`) with a blank white screen.
- `apps/server` — Rust process (`@apprelay/server`), currently idle until Ctrl+C.
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

### Rust server

Install Rust 1.85 or newer with the Clippy and rustfmt components, then run:

```sh
pnpm server
```

The process waits asynchronously for Ctrl+C and exits cleanly. It does not listen on any ports or implement HTTP, VPN, or device functionality yet. `pnpm --filter @apprelay/server start:release` runs an optimized build.

## Code quality

- `pnpm lint` — run each workspace's lint script (the mobile app uses oxlint with React rules, autofixes, and warnings treated as errors).
- `pnpm typecheck` — TypeScript checking for mobile and `cargo check` for the server.
- The server's lint script runs Clippy with warnings treated as errors.
- `pnpm format` / `pnpm format:check` — oxfmt across the repository plus rustfmt for the server.
- Husky runs lint-staged before commits, formatting and re-staging supported staged files with oxfmt or rustfmt.
- Pi runs lint and typecheck automatically before settling. See [`.pi/README.md`](.pi/README.md).

No navigation, application features, or tests are included.
