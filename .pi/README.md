# Pi hooks

`.pi/settings.json` loads the published `@michalkvasnicak/pi-hooks` package, matching the setup in `../piup`. No sibling checkout is required at runtime.

Start Pi from the repository root and grant project trust to load the package. Run `/reload` after changing settings or hook configuration.

`.pi/pi-hooks.json` uses the runtime configuration shape: `agent_before_settle` is a top-level key, without a `hooks` wrapper or `$schema`.

Before the agent finishes, separate checks run `pnpm --filter @apprelay/mobile lint` and `pnpm --filter @apprelay/mobile typecheck` for `apps/mobile`: oxlint with React rules, autofixes, and warnings treated as errors, plus TypeScript (`tsc --noEmit`). Checks run initially and then rerun when their inputs change, including the app's sources, tool configuration, package manifests, workspace configuration, and lockfile. Add separate checks with appropriate input globs when adding apps or shared packages. The server has separate `pnpm --filter @apprelay/server lint` (Clippy) and `typecheck` (`cargo check`) checks watching Rust sources, Cargo manifests/lockfile, and workspace manifests. This is a lifecycle check, not a filesystem watcher. Checks fail fast; failures are sent back to the agent for up to five retries.

Formatting is intentionally separate: `pnpm install` installs Husky via `prepare`, and `.husky/pre-commit` runs lint-staged. It formats supported staged files with oxfmt or rustfmt and stages the changes, hiding and restoring unstaged edits to partially staged files. Formatting failures abort the commit.
