import { createApiClient, getApiUrl } from "../api/client";
import type { components } from "../api/schema";

import { credentials } from "./credentials";

export type DeviceRegistration = components["schemas"]["RegistrationResponse"];

export function getRegistrationUrl(): string {
  return `${getApiUrl()}/v1/devices`;
}

export async function registerDevice(publicKey: string): Promise<DeviceRegistration> {
  const url = getRegistrationUrl();
  const scope = `${url}:${publicKey}`;
  const token = await credentials.get(scope);
  const {
    data: device,
    error,
    response,
  } = await createApiClient().POST("/v1/devices", {
    headers: token ? { Authorization: `Bearer ${token}` } : {},
    body: { wireguard_public_key: publicKey },
  });
  if (response.status !== 201 || !device) {
    const reason = error?.error;
    if (response.status === 401 && reason === "a valid app key is required") {
      throw new Error("Device registration failed (HTTP 401): the server rejected the app key.");
    }
    if (response.status === 401 && reason === "a valid device bearer token is required") {
      throw new Error(
        token
          ? "Device registration failed (HTTP 401): the server rejected the saved device token."
          : "Device registration failed (HTTP 401): no saved device token was found for this API URL and device identity.",
      );
    }
    throw new Error(`Device registration failed (HTTP ${response.status}).`);
  }
  if (!/^[A-Za-z0-9_-]{43}$/.test(device.token)) {
    throw new Error("Device registration returned an invalid bearer token.");
  }
  await credentials.save(scope, device.token);
  return device;
}
