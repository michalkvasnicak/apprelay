import createClient from "openapi-fetch";

import type { paths } from "../api/schema";
import { base64 } from "../apple/encoding";
import { signingLimits } from "../signing/types";

export class ArtifactError extends Error {}

const client = createClient<paths>({ baseUrl: window.location.origin });

async function readBytes(response: Response, size: number) {
  if (!response.body) {
    throw new ArtifactError("The IPA download was empty. Try again.");
  }
  const bytes = new Uint8Array(size);
  const reader = response.body.getReader();
  let offset = 0;
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) {
        break;
      }
      if (offset + value.byteLength > size) {
        throw new ArtifactError("The downloaded IPA does not match its declared size.");
      }
      bytes.set(value, offset);
      offset += value.byteLength;
    }
    if (offset !== size) {
      throw new ArtifactError("The IPA download was incomplete. Try again.");
    }
    return bytes;
  } catch (error) {
    await reader.cancel().catch(() => {});
    throw error;
  } finally {
    reader.releaseLock();
  }
}

export async function downloadArtifact(token: string, signal: AbortSignal): Promise<File> {
  if (!/^[A-Za-z0-9_-]{43}$/.test(token)) {
    throw new ArtifactError("This share link is invalid.");
  }
  try {
    const { data, response } = await client.GET("/v1/artifacts/{token}", {
      params: { path: { token } },
      signal,
      cache: "no-store",
      credentials: "omit",
      referrerPolicy: "no-referrer",
      redirect: "error",
    });
    if (response.status === 404) {
      throw new ArtifactError("This app is missing, expired, or has not finished uploading.");
    }
    if (response.status === 429) {
      throw new ArtifactError("Too many requests. Wait a moment before trying again.");
    }
    if (!response.ok || !data) {
      throw new ArtifactError("This app is temporarily unavailable. Try again later.");
    }
    if (
      !Number.isSafeInteger(data.size_bytes) ||
      data.size_bytes < 1 ||
      data.size_bytes > signingLimits.inputBytes ||
      typeof data.file_name !== "string" ||
      !/\.ipa$/i.test(data.file_name)
    ) {
      throw new ArtifactError("This app is not a supported IPA of up to 100 MiB.");
    }
    if (data.expires_at != null && !(Date.parse(data.expires_at) > Date.now())) {
      throw new ArtifactError("This share link has expired.");
    }
    const downloadUrl = new URL(data.download_url);
    if (downloadUrl.protocol !== "https:") {
      throw new ArtifactError("This app has an invalid download URL.");
    }
    const download = await fetch(downloadUrl, {
      signal,
      credentials: "omit",
      referrerPolicy: "no-referrer",
      redirect: "error",
      cache: "no-store",
    });
    if (!download.ok) {
      await download.body?.cancel();
      throw new ArtifactError("Could not download this app. Try again to request a fresh link.");
    }
    const bytes = await readBytes(download, data.size_bytes);
    const checksum = base64(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)));
    if (checksum !== data.sha256) {
      throw new ArtifactError("The IPA checksum did not match. Do not sign this download.");
    }
    signal.throwIfAborted();
    return new File([bytes], data.file_name, { type: "application/octet-stream" });
  } catch (error) {
    if (error instanceof ArtifactError) {
      throw error;
    }
    // Network errors can contain a share token or presigned URL.
    throw new ArtifactError("Could not load the app. Check your connection and try again.");
  }
}
