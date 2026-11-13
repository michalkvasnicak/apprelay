# apprelay

pnpm monorepo with a minimal Expo React Native app, TypeScript, and a blank white screen.

## Layout

- `apps/mobile` — Expo app (`@apprelay/mobile`).
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

## Code quality

- `pnpm lint` — run each workspace's lint script (the mobile app uses oxlint with React rules, autofixes, and warnings treated as errors).
- `pnpm typecheck` — run each workspace's TypeScript checking.
- `pnpm format` / `pnpm format:check` — oxfmt across the repository.
- Husky runs lint-staged before commits, formatting and re-staging supported staged files.
- Pi runs lint and typecheck automatically before settling. See [`.pi/README.md`](.pi/README.md).

No navigation, application features, or tests are included.
