import createClient from "openapi-fetch";

import type { paths } from "./schema.d.ts";

export function createApiClient(baseUrl: string, signal: AbortSignal) {
  return createClient<paths>({
    baseUrl,
    fetch: (request) =>
      fetch(request, {
        signal: AbortSignal.any([signal, AbortSignal.timeout(30_000)]),
        redirect: "error",
      }),
    pathSerializer: (pathname, pathParams) =>
      pathname.replace(/\{([^}]+)\}/g, (_, key: string) =>
        encodeURIComponent(String(pathParams[key])),
      ),
  });
}
