import { spawn } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { parseEnv } from "node:util";

const projectRoot = fileURLToPath(new URL("../", import.meta.url));
const envFile = new URL("../.env.testflight", import.meta.url);

async function publish() {
  const original = readFileSync(envFile, "utf8");
  const current = parseEnv(original).APPLE_BUILD_NUMBER;
  if (!current || !/^\d+$/.test(current)) {
    throw new Error("Set APPLE_BUILD_NUMBER to a non-negative integer in .env.testflight.");
  }

  const assignments = [
    ...original.matchAll(/^[\t ]*(?:export[\t ]+)?APPLE_BUILD_NUMBER[\t ]*=[^\r\n]*/gm),
  ];
  if (assignments.length !== 1) {
    throw new Error("Expected exactly one APPLE_BUILD_NUMBER assignment in .env.testflight.");
  }
  const assignment = assignments[0];
  const next = String(BigInt(current) + 1n);
  const updatedAssignment = assignment[0].replace(
    /^(.*?=[\t ]*)(?:"\d+"|'\d+'|\d+)([\t ]*(?:#.*)?)$/,
    (_match, prefix, suffix) => `${prefix}${next}${suffix}`,
  );
  if (updatedAssignment === assignment[0]) {
    throw new Error("APPLE_BUILD_NUMBER must be a single-line integer in .env.testflight.");
  }
  const updated =
    original.slice(0, assignment.index) +
    updatedAssignment +
    original.slice(assignment.index + assignment[0].length);

  let succeeded = false;
  writeFileSync(envFile, updated);
  try {
    console.log(`Publishing with APPLE_BUILD_NUMBER=${next} (previously ${current}).`);
    const child = spawn(
      process.execPath,
      ["scripts/with-env.mjs", "testflight", "node", "scripts/publish-testflight.mjs"],
      { cwd: projectRoot, stdio: "inherit", detached: process.platform !== "win32" },
    );
    let interrupted = false;
    const forwardSignal = (signal) => {
      interrupted = true;
      try {
        if (process.platform === "win32") {
          child.kill(signal);
        } else if (child.pid) {
          process.kill(-child.pid, signal);
        }
      } catch (error) {
        if (error.code !== "ESRCH") {
          throw error;
        }
      }
    };
    const onInterrupt = () => forwardSignal("SIGINT");
    const onTerminate = () => forwardSignal("SIGTERM");
    process.on("SIGINT", onInterrupt);
    process.on("SIGTERM", onTerminate);
    try {
      const code = await new Promise((resolve, reject) => {
        child.once("error", reject);
        child.once("close", (status) => resolve(status));
      });
      succeeded = code === 0 && !interrupted;
      process.exitCode = succeeded ? 0 : code || 1;
    } finally {
      process.off("SIGINT", onInterrupt);
      process.off("SIGTERM", onTerminate);
    }
  } finally {
    if (!succeeded) {
      writeFileSync(envFile, original);
      console.error(`Publishing failed; restored APPLE_BUILD_NUMBER=${current}.`);
    }
  }
}

try {
  await publish();
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
