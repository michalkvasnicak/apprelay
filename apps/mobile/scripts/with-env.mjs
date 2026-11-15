import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { parseEnv } from "node:util";

const [environment, command, ...args] = process.argv.slice(2);
if (!["local", "testflight"].includes(environment) || !command) {
  console.error("Usage: node scripts/with-env.mjs <local|testflight> <command> [...args]");
  process.exit(1);
}

const projectRoot = fileURLToPath(new URL("../", import.meta.url));
const envFile = new URL(`../.env.${environment}`, import.meta.url);
let values;
try {
  values = parseEnv(readFileSync(envFile, "utf8"));
} catch {
  console.error(
    `Cannot read .env.${environment}. Copy .env.${environment}.example and fill in its values.`,
  );
  process.exit(1);
}

// Do not inherit app configuration from the shell or another environment.
const env = { ...process.env };
for (const key of Object.keys(env)) {
  if (
    key.startsWith("EXPO_PUBLIC_") ||
    [
      "APPLE_TEAM_ID",
      "APPLE_BUNDLE_IDENTIFIER",
      "APPLE_BUILD_NUMBER",
      "APPLE_API_KEY_ID",
      "APPLE_API_ISSUER_ID",
      "APPLE_API_KEY_PATH",
    ].includes(key)
  ) {
    delete env[key];
  }
}
Object.assign(env, values, {
  EXPO_NO_DOTENV: "1",
  APPRELAY_ENV: environment,
  EXPO_PUBLIC_APP_SCHEME: environment === "local" ? "apprelay-dev" : "apprelay",
});

const result = spawnSync(command, args, { cwd: projectRoot, env, stdio: "inherit" });
if (result.error) {
  console.error(result.error.message);
}
process.exit(result.status ?? 1);
