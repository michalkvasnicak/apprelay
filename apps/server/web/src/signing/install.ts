import { api } from "../api/client";
import type { components } from "../api/schema";
import { base64 } from "../apple/encoding";
import { SigningError, signingLimits } from "./types";

function installError(status: number, fallback: string) {
  if (status === 401 || status === 404) {
    return new SigningError("Install authorization expired. Pair your iPhone again.");
  }
  if (status === 429) {
    return new SigningError("Too many install requests. Wait a moment before retrying.");
  }
  return new SigningError(fallback);
}

function putSignedIpa(
  file: File,
  upload: components["schemas"]["InstallUpload"],
  onProgress: (percent: number) => void,
) {
  const url = new URL(upload.upload_url);
  if (url.protocol !== "https:") {
    throw new SigningError("The install server returned an invalid upload URL.");
  }

  const remaining = Date.parse(upload.upload_expires_at) - Date.now();
  if (!Number.isFinite(remaining) || remaining <= 0) {
    throw new SigningError("The upload window expired. Try again or pair your iPhone again.");
  }

  return new Promise<void>((resolve, reject) => {
    const request = new XMLHttpRequest();
    const failed = () => reject(new SigningError("Could not upload the signed IPA. Try again."));
    request.upload.onprogress = (event) => {
      onProgress(Math.min(100, Math.floor((event.loaded / file.size) * 100)));
    };
    request.onload = () => {
      if (request.status >= 200 && request.status < 300) {
        onProgress(100);
        resolve();
      } else {
        failed();
      }
    };
    request.onerror = failed;
    request.ontimeout = failed;
    request.onabort = failed;
    request.open("PUT", url.href);
    request.timeout = Math.min(remaining, 15 * 60_000);
    for (const [name, value] of Object.entries(upload.upload_headers)) {
      // The browser sets Content-Length from the exact File body.
      if (name.toLowerCase() !== "content-length") {
        request.setRequestHeader(name, value);
      }
    }
    request.send(file);
  });
}

export async function uploadSignedIpa(
  file: File,
  headers: HeadersInit,
  onProgress: (percent: number) => void,
) {
  if (file.size < 1 || file.size > signingLimits.inputBytes) {
    throw new SigningError("The signed IPA must be no larger than 100 MiB to install.");
  }
  const sha256 = base64(
    new Uint8Array(await crypto.subtle.digest("SHA-256", await file.arrayBuffer())),
  );
  const { data, response } = await api.POST("/v1/install-jobs", {
    body: { size_bytes: file.size, sha256 },
    headers,
    redirect: "error",
    signal: AbortSignal.timeout(20_000),
  });
  if (response.status !== 201 || !data) {
    throw installError(response.status, "Could not prepare the install upload. Try again.");
  }
  onProgress(0);
  await putSignedIpa(file, data, onProgress);
  return data.job_id;
}

export async function queueInstall(jobId: string, headers: HeadersInit) {
  const { response } = await api.POST("/v1/install-jobs/{job_id}/complete", {
    params: { path: { job_id: jobId } },
    headers,
    parseAs: "text",
    redirect: "error",
    signal: AbortSignal.timeout(20_000),
  });
  if (response.status !== 202) {
    throw installError(
      response.status,
      "Could not confirm that the install was queued. Try again.",
    );
  }
}
