import { spawn, spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { setTimeout } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const cwd = fileURLToPath(new URL("../", import.meta.url));
const container = `apprelay-api-test-${randomUUID()}`;
const password = randomUUID();
let child;
let interrupted;
let containerRequested = false;

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    interrupted = signal;
    child?.kill("SIGTERM");
  });
}

async function run(command, args, { capture = false, env = process.env } = {}) {
  if (interrupted) {
    throw new Error(`Interrupted by ${interrupted}`);
  }
  return new Promise((resolve, reject) => {
    child = spawn(command, args, {
      cwd,
      env,
      stdio: ["ignore", capture ? "pipe" : "inherit", "inherit"],
    });
    let output = "";
    child.stdout?.setEncoding("utf8").on("data", (data) => {
      output += data;
    });
    child.once("error", (error) => {
      child = undefined;
      reject(error);
    });
    child.once("close", (code, signal) => {
      child = undefined;
      if (code !== 0) {
        reject(new Error(`${command} failed (${signal ?? code})`));
      } else {
        resolve(output.trim());
      }
    });
  });
}

try {
  await run("docker", ["info"], { capture: true });
  console.log(`Starting disposable PostgreSQL: ${container}`);
  containerRequested = true;
  await run(
    "docker",
    [
      "run",
      "--detach",
      "--rm",
      "--name",
      container,
      "--publish",
      "127.0.0.1::5432",
      "--tmpfs",
      "/var/lib/postgresql:rw",
      "--env",
      "POSTGRES_USER=apprelay_test",
      "--env",
      `POSTGRES_PASSWORD=${password}`,
      "--env",
      "POSTGRES_DB=apprelay_test",
      "postgres:18-alpine",
    ],
    { capture: true },
  );

  const deadline = Date.now() + 60_000;
  while (true) {
    try {
      await run(
        "docker",
        [
          "exec",
          container,
          "pg_isready",
          "-h",
          "127.0.0.1",
          "-U",
          "apprelay_test",
          "-d",
          "apprelay_test",
        ],
        { capture: true },
      );
      break;
    } catch (error) {
      if (interrupted || Date.now() >= deadline) {
        throw error;
      }
      await setTimeout(250);
    }
  }

  const binding = await run("docker", ["port", container, "5432/tcp"], { capture: true });
  const port = /^127\.0\.0\.1:(\d+)$/.exec(binding)?.[1];
  if (!port) {
    throw new Error(`Unexpected PostgreSQL port binding: ${binding}`);
  }
  const databaseUrl = `postgres://apprelay_test:${password}@127.0.0.1:${port}/apprelay_test`;
  await run("cargo", ["test", "--tests", ...process.argv.slice(2)], {
    env: { ...process.env, DATABASE_URL: databaseUrl },
  });
} catch (error) {
  console.error(error.message);
  process.exitCode = interrupted === "SIGINT" ? 130 : interrupted === "SIGTERM" ? 143 : 1;
} finally {
  if (interrupted) {
    process.exitCode = interrupted === "SIGINT" ? 130 : 143;
  }
  if (containerRequested) {
    console.log(`Removing disposable PostgreSQL: ${container}`);
    const result = spawnSync("docker", ["rm", "--force", "--volumes", container], {
      encoding: "utf8",
      timeout: 30_000,
    });
    if (result.error || result.status !== 0) {
      console.error(result.error?.message ?? result.stderr);
      console.error(`If it still exists, remove it with: docker rm -fv ${container}`);
      process.exitCode ||= 1;
    }
  }
}
