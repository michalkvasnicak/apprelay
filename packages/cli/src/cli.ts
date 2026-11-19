import { createHash } from "node:crypto";
import { openAsBlob } from "node:fs";
import { stat } from "node:fs/promises";
import { basename } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { parseArgs } from "node:util";

import { createApiClient } from "./api/client.ts";

const DEFAULT_SERVER = "https://apprelay.app";
const MAX_BYTES = 100 * 1024 * 1024;
const HELP = `Usage: apprelay [--server <origin>] <file.ipa>

Upload an IPA anonymously and print its share URL to stdout.
Upload status is written to stderr. Maximum file size: 100 MiB.

Options:
  --server <origin>  API origin (default: APPRELAY_SERVER or ${DEFAULT_SERVER})
  -h, --help         Show help

Requires Node.js 24.12 or newer. No account or API key is needed.
`;

class CliError extends Error {}

function options() {
  let args;
  try {
    args = parseArgs({
      allowPositionals: true,
      options: {
        server: { type: "string" },
        help: { type: "boolean", short: "h" },
      },
    });
  } catch {
    throw new CliError("Invalid arguments. Run apprelay --help for usage.");
  }
  if (args.values.help) {
    process.stdout.write(HELP);
    return null;
  }
  if (args.positionals.length !== 1) {
    throw new CliError("Provide exactly one IPA path. Run apprelay --help for usage.");
  }
  const path = args.positionals[0]!;
  const fileName = basename(path);
  if (
    !/\.ipa$/i.test(fileName) ||
    [...fileName].length < 5 ||
    [...fileName].length > 200 ||
    /[\p{Cc}/\\]/u.test(fileName)
  ) {
    throw new CliError("Provide an .ipa file with a basename of at most 200 characters.");
  }
  let server;
  try {
    server = new URL(args.values.server ?? process.env.APPRELAY_SERVER ?? DEFAULT_SERVER);
  } catch {
    throw new CliError("Server must be an HTTP(S) origin.");
  }
  if (
    !["https:", "http:"].includes(server.protocol) ||
    server.username ||
    server.password ||
    server.pathname !== "/" ||
    server.search ||
    server.hash
  ) {
    throw new CliError("Server must be an HTTP(S) origin without credentials, path, or query.");
  }
  if (
    server.protocol === "http:" &&
    !["localhost", "127.0.0.1", "[::1]"].includes(server.hostname)
  ) {
    throw new CliError("Use HTTPS for remote servers; HTTP is allowed only on localhost.");
  }
  return { path, fileName, server: server.origin };
}

async function readIpa(path: string) {
  try {
    const info = await stat(path);
    if (!info.isFile()) {
      throw new CliError("The IPA path must point to a regular file.");
    }
    if (info.size < 1 || info.size > MAX_BYTES) {
      throw new CliError("The IPA must be nonempty and no larger than 100 MiB.");
    }
    // File-backed Blob streams the upload and detects changes after opening.
    const file = await openAsBlob(path, { type: "application/octet-stream" });
    if (file.size < 1 || file.size > MAX_BYTES) {
      throw new CliError("The IPA must be nonempty and no larger than 100 MiB.");
    }
    return file;
  } catch (error) {
    if (error instanceof CliError) {
      throw error;
    }
    throw new CliError("Cannot read the IPA. Check the path and file permissions.");
  }
}

function apiFailure(stage: string, status: number): CliError {
  const hints: Record<number, string> = {
    413: "The IPA exceeds the server's size limit.",
    429: "Rate limited; wait before trying again.",
  };
  return new CliError(`${stage} failed (HTTP ${status}). ${hints[status] ?? "Try again later."}`);
}

async function upload(
  input: NonNullable<ReturnType<typeof options>>,
  signal: AbortSignal,
): Promise<string> {
  const file = await readIpa(input.path);
  process.stderr.write("Calculating checksum…\n");
  const hash = createHash("sha256");
  for await (const chunk of file.stream()) {
    signal.throwIfAborted();
    hash.update(chunk);
  }
  const client = createApiClient(input.server, signal);
  process.stderr.write("Preparing upload…\n");
  const created = await client.POST("/v1/artifacts", {
    body: {
      file_name: input.fileName,
      size_bytes: file.size,
      sha256: hash.digest("base64"),
    },
  });
  if (!created.response.ok || !created.data) {
    throw apiFailure("Preparing upload", created.response.status);
  }
  const artifact = created.data;
  // Reject unexpected transport protocols before sending the IPA.
  const uploadUrl = new URL(artifact.upload_url);
  if (uploadUrl.protocol !== "https:") {
    throw new CliError("The server returned an insecure upload URL.");
  }
  process.stderr.write(`Uploading (${(file.size / 1024 / 1024).toFixed(1)} MiB)…\n`);
  let putStatus: number | undefined;
  try {
    const response = await fetch(uploadUrl, {
      method: "PUT",
      headers: artifact.upload_headers,
      body: file,
      redirect: "error",
      signal: AbortSignal.any([signal, AbortSignal.timeout(10 * 60_000)]),
    });
    putStatus = response.status;
    await response.body?.cancel();
  } catch {
    signal.throwIfAborted();
    // A timed-out PUT can still have succeeded. Verify before reporting failure.
  }

  process.stderr.write("Verifying upload…\n");
  for (let attempt = 0; attempt < 3; attempt++) {
    signal.throwIfAborted();
    try {
      const completed = await client.POST("/v1/artifacts/{artifact_id}/complete", {
        params: { path: { artifact_id: artifact.artifact_id } },
      });
      if (completed.response.status === 204) {
        return artifact.share_url;
      }
      const status = completed.response.status;
      if (status !== 409 && status !== 429 && status < 500) {
        throw apiFailure("Verifying upload", status);
      }
      if (attempt === 2) {
        if (putStatus !== undefined && (putStatus < 200 || putStatus >= 300)) {
          throw apiFailure("Uploading IPA", putStatus);
        }
        throw apiFailure("Verifying upload", status);
      }
    } catch (error) {
      if (error instanceof CliError) {
        throw error;
      }
      signal.throwIfAborted();
      if (attempt === 2) {
        throw new CliError("Could not confirm the upload. Check your connection and try again.");
      }
    }
    await delay(1000 * (attempt + 1), undefined, { signal });
  }
  throw new CliError("Could not confirm the upload.");
}

const controller = new AbortController();
const cancel = () => controller.abort();
process.once("SIGINT", cancel);
process.once("SIGTERM", cancel);

try {
  const input = options();
  if (input) {
    const url = await upload(input, controller.signal);
    process.stderr.write("Upload complete. Share URL:\n");
    process.stdout.write(`${url}\n`);
  }
} catch (error) {
  if (controller.signal.aborted) {
    process.stderr.write("Upload cancelled.\n");
    process.exitCode = 130;
  } else {
    // Never print raw network errors: they may contain presigned URLs or tokens.
    const message =
      error instanceof CliError
        ? error.message
        : "Upload failed. Check your connection and ensure the IPA has not changed.";
    process.stderr.write(`apprelay: ${message}\n`);
    process.exitCode = 1;
  }
} finally {
  process.removeListener("SIGINT", cancel);
  process.removeListener("SIGTERM", cancel);
}
