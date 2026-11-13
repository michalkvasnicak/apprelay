# Repository Guidelines

## Code conventions

- Prefer clear names, simple control flow, and focused functions. Follow existing package conventions.
- Preserve precise TypeScript literal types with `as const` and `satisfies`; derive unions rather than widening fixed values.
- Keep changes focused on requested behavior; do not add speculative features or abstractions.
- Do not run lint, formatting, typechecking, builds, or tests manually; checks run automatically before the agent finishes.
- Add tests only when the user explicitly specifies what to test. Test observable behavior or meaningful contracts, not tautologies.

## Project structure

- `apps/mobile` (`@apprelay/mobile`): Expo React Native client in strict TypeScript. Place mobile UI and client-side behavior here; `App.tsx` is the root component.
- `apps/server` (`@apprelay/server`): Rust backend. Place server-side behavior here; `src/main.rs` is the entry point. Currently an idle process without network listeners.
- `packages/*`: Shared packages. Extract code here only when it is genuinely shared.
- Repository root: Shared workspace configuration, code-quality tooling, and Git hooks. Keep application logic inside its app or package.
- `.pi/pi-hooks.json`: Automatic agent checks. Add appropriate checks and input globs when adding apps or packages.
- Configure generated iOS and Android projects through `apps/mobile/app.json` and Expo config plugins; do not edit generated native directories directly.
