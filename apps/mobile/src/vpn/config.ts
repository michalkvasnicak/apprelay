import { createApiClient } from "../api/client";
import type { components } from "../api/schema";

import { credentials } from "./credentials";

export type DeviceRegistration = components["schemas"]["RegistrationResponse"];

export class DeviceCredentialError extends Error {}

export async function registerDevice(publicKey: string): Promise<DeviceRegistration> {
  const token = await credentials.get();
  const {
    data: device,
    error,
    response,
  } = await createApiClient().POST("/v1/devices", {
    headers: token ? { Authorization: `Bearer ${token}` } : {},
    body: { wireguard_public_key: publicKey },
  });
  if (response.status !== 201 || !device) {
    if (response.status === 401 && error?.code === "app_key_required") {
      throw new Error("Device registration failed (HTTP 401): the server rejected the app key.");
    }
    if (response.status === 401 && error?.code === "device_token_required") {
      throw new DeviceCredentialError(
        token
          ? "Device registration failed (HTTP 401): the server rejected the saved device token."
          : "Device registration failed (HTTP 401): no saved device token was found for this app.",
      );
    }
    throw new Error(`Device registration failed (HTTP ${response.status}).`);
  }
  if (!/^[A-Za-z0-9_-]{43}$/.test(device.token)) {
    throw new Error("Device registration returned an invalid bearer token.");
  }
  await credentials.save(device.token);
  return device;
}
