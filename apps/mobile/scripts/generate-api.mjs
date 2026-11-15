import { spawnSync } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import openapiTS, { astToString } from "openapi-typescript";
import { format } from "oxfmt";

const output = new URL("../src/api/schema.d.ts", import.meta.url);
const server = fileURLToPath(new URL("../../server/", import.meta.url));
const result = spawnSync("cargo", ["run", "--quiet", "--bin", "export-openapi"], {
  cwd: server,
  encoding: "utf8",
  maxBuffer: 10 * 1024 * 1024,
  stdio: ["ignore", "pipe", "inherit"],
});
if (result.error) {
  throw result.error;
}
if (result.status !== 0) {
  process.exit(result.status ?? 1);
}

const schema = JSON.parse(result.stdout);
const generated = [
  "// Generated from the Rust API. Do not edit; run pnpm api:generate.",
  astToString(await openapiTS(schema)),
].join("\n");
const formatConfig = JSON.parse(
  await readFile(new URL("../../../.oxfmtrc.json", import.meta.url), "utf8"),
);
const { code: source, errors } = await format(fileURLToPath(output), generated, formatConfig);
if (errors.length > 0) {
  throw new Error("Could not format generated API types");
}

if (process.argv.includes("--check")) {
  const existing = await readFile(output, "utf8").catch((error) => {
    if (error.code === "ENOENT") {
      return null;
    }
    throw error;
  });
  if (existing !== source) {
    console.error("API types are out of sync. Run pnpm api:generate and commit the result.");
    process.exit(1);
  }
} else {
  await mkdir(new URL(".", output), { recursive: true });
  await writeFile(output, source);
  console.log("Updated apps/mobile/src/api/schema.d.ts");
}
