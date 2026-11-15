import { createApiClient } from "../api/client";

export async function syncPushToken(
  deviceId: string,
  bearerToken: string,
  token: string | null,
  signal: AbortSignal,
): Promise<void> {
  const api = createApiClient();
  const options = {
    params: { path: { device_id: deviceId } },
    headers: { Authorization: `Bearer ${bearerToken}` },
    signal,
  };
  const { response } =
    token === null
      ? await api.DELETE("/v1/devices/{device_id}/push-token", options)
      : await api.PUT("/v1/devices/{device_id}/push-token", {
          ...options,
          body: { token },
        });
  if (response.status !== 204) {
    throw new Error(`Push token sync failed (HTTP ${response.status}).`);
  }
}
