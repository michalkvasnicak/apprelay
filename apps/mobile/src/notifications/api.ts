import { getRegistrationUrl } from "../vpn/config";

export async function syncPushToken(
  deviceId: string,
  token: string | null,
  signal: AbortSignal,
): Promise<void> {
  const response = await fetch(
    `${getRegistrationUrl()}/${encodeURIComponent(deviceId)}/push-token`,
    {
      method: token === null ? "DELETE" : "PUT",
      ...(token === null
        ? {}
        : {
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ token }),
          }),
      signal,
    },
  );
  if (response.status !== 204) {
    throw new Error(`Push token sync failed (HTTP ${response.status}).`);
  }
}
