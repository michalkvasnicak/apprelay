import createClient from "openapi-fetch";

import type { paths } from "./schema";

export function getApiUrl(): string {
  // Expo requires direct references for build-time inlining.
  // Required environment variables are validated in app.config.ts.
  return new URL(process.env.EXPO_PUBLIC_VPN_API_URL!.trim()).toString().replace(/\/$/, "");
}

export function createApiClient() {
  return createClient<paths>({
    baseUrl: getApiUrl(),
    headers: { "X-App-Key": process.env.EXPO_PUBLIC_API_KEY! },
    // URL-encode path parameters, including untrusted persisted device IDs.
    pathSerializer: (pathname, pathParams) =>
      pathname.replace(/\{([^}]+)\}/g, (_, key: string) =>
        encodeURIComponent(String(pathParams[key])),
      ),
  });
}
