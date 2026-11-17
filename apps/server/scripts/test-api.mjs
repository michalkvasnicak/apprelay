import { spawn, spawnSync } from "node:child_process";
import { randomBytes, randomUUID } from "node:crypto";
import { setTimeout } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const cwd = fileURLToPath(new URL("../", import.meta.url));
const container = `apprelay-api-test-${randomUUID()}`;
const redisContainer = `${container}-redis`;
const password = randomUUID();
const containers = [];
let child;
let interrupted;

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

async function waitUntilReady(name, command) {
  const deadline = Date.now() + 60_000;
  while (true) {
    try {
      await run("docker", ["exec", name, ...command], { capture: true });
      return;
    } catch (error) {
      if (interrupted || Date.now() >= deadline) {
        throw error;
      }
      await setTimeout(250);
    }
  }
}

async function publishedPort(name, containerPort) {
  const binding = await run("docker", ["port", name, `${containerPort}/tcp`], { capture: true });
  const port = /^127\.0\.0\.1:(\d+)$/.exec(binding)?.[1];
  if (!port) {
    throw new Error(`Unexpected port binding for ${name}: ${binding}`);
  }
  return port;
}

try {
  await run("docker", ["info"], { capture: true });
  console.log(`Starting disposable PostgreSQL: ${container}`);
  containers.push(container);
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

  await waitUntilReady(container, [
    "pg_isready",
    "-h",
    "127.0.0.1",
    "-U",
    "apprelay_test",
    "-d",
    "apprelay_test",
  ]);

  console.log(`Starting disposable Redis: ${redisContainer}`);
  containers.push(redisContainer);
  await run(
    "docker",
    [
      "run",
      "--detach",
      "--rm",
      "--name",
      redisContainer,
      "--publish",
      "127.0.0.1::6379",
      "--tmpfs",
      "/data:rw",
      "redis:7-alpine",
      "redis-server",
      "--save",
      "",
      "--appendonly",
      "no",
    ],
    { capture: true },
  );
  await waitUntilReady(redisContainer, ["redis-cli", "ping"]);

  const port = await publishedPort(container, 5432);
  const redisPort = await publishedPort(redisContainer, 6379);
  const databaseUrl = `postgres://apprelay_test:${password}@127.0.0.1:${port}/apprelay_test`;
  await run("cargo", ["test", "--tests", "--features", "test-support", ...process.argv.slice(2)], {
    env: {
      ...process.env,
      DATABASE_URL: databaseUrl,
      REDIS_URL: `redis://127.0.0.1:${redisPort}/0`,
      APPRELAY_API_KEY: randomBytes(32).toString("hex"),
      APPRELAY_APNS_ENVIRONMENT: "sandbox",
      PAIRING_ENCRYPTION_KEY: randomBytes(32).toString("base64"),
      APPRELAY_TRUSTED_PROXY_CIDRS: "",
    },
  });
} catch (error) {
  console.error(error.message);
  process.exitCode = interrupted === "SIGINT" ? 130 : interrupted === "SIGTERM" ? 143 : 1;
} finally {
  if (interrupted) {
    process.exitCode = interrupted === "SIGINT" ? 130 : 143;
  }
  for (const name of containers.reverse()) {
    console.log(`Removing disposable container: ${name}`);
    const result = spawnSync("docker", ["rm", "--force", "--volumes", name], {
      encoding: "utf8",
      timeout: 30_000,
    });
    if (result.error || result.status !== 0) {
      console.error(result.error?.message ?? result.stderr);
      console.error(`If it still exists, remove it with: docker rm -fv ${name}`);
      process.exitCode ||= 1;
    }
  }
}
