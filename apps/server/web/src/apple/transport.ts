import { base64, unbase64 } from "./encoding";
import { integer, record, string } from "./validation";

const maximumBody = 2_000_000;

export async function relay(
  url: string,
  method: "GET" | "POST",
  headers: Record<string, string>,
  body: Uint8Array = new Uint8Array(),
  signal?: AbortSignal,
) {
  if (body.length > maximumBody) {
    throw new Error("Apple request exceeds the size limit");
  }
  const timeout = AbortSignal.timeout(80_000);
  const response = await fetch("/v1/apple/proxy", {
    method: "POST",
    credentials: "omit",
    cache: "no-store",
    redirect: "error",
    signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ url, method, headers, body: base64(body) }),
  });
  if (!response.ok) {
    throw new Error("Apple relay is unavailable or rejected the request");
  }
  // Bound streaming allocation before decoding the envelope.
  const reader = response.body?.getReader();
  if (!reader) {
    throw new Error("Empty relay response");
  }
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    while (true) {
      const chunk = await reader.read();
      if (chunk.done) {
        break;
      }
      size += chunk.value.length;
      if (size > 2_850_000) {
        await reader.cancel();
        throw new Error("Relay response exceeds the size limit");
      }
      chunks.push(chunk.value);
    }
  } finally {
    reader.releaseLock();
  }
  const envelope = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    envelope.set(chunk, offset);
    offset += chunk.length;
  }
  const payload = record(JSON.parse(new TextDecoder().decode(envelope)));
  const status = integer(payload.status);
  if (status < 200 || status > 599) {
    throw new Error("Invalid upstream status");
  }
  const result = unbase64(typeof payload.body === "string" ? payload.body : string(payload.body));
  if (result.length > maximumBody) {
    throw new Error("Apple response exceeds the size limit");
  }
  const responseHeaders = record(payload.headers);
  return { status, headers: responseHeaders, body: result };
}

export function requireSuccess(status: number) {
  if (status < 200 || status >= 300) {
    throw new Error("Apple rejected the request");
  }
}
