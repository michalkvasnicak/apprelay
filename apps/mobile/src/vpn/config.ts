import { credentials } from "./credentials";

export type DeviceRegistration = {
  device_id: string;
  token: string;
  address: string;
  expires_at: string;
  lifetime_seconds: number;
  wireguard: {
    server_public_key: string;
    allowed_ips: string[];
    persistent_keepalive_seconds: number;
  };
};

export function getRegistrationUrl(): string {
  // Expo requires a direct reference for build-time inlining.
  // Required environment variables are validated in app.config.ts.
  const url = new URL(process.env.EXPO_PUBLIC_VPN_API_URL!.trim());
  return `${url.toString().replace(/\/$/, "")}/v1/devices`;
}

export function getAppKeyHeaders(): { "X-App-Key": string } {
  // Expo requires a direct reference for build-time inlining. This is not a secret.
  return { "X-App-Key": process.env.EXPO_PUBLIC_API_KEY! };
}

export async function registerDevice(publicKey: string): Promise<DeviceRegistration> {
  const url = getRegistrationUrl();
  const scope = `${url}:${publicKey}`;
  const token = await credentials.get(scope);
  const response = await fetch(url, {
    method: "POST",
    headers: {
      ...getAppKeyHeaders(),
      "Content-Type": "application/json",
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
    },
    body: JSON.stringify({ wireguard_public_key: publicKey }),
  });
  if (response.status !== 201) {
    throw new Error(`Device registration failed (HTTP ${response.status}).`);
  }
  const device = (await response.json()) as DeviceRegistration;
  if (!/^[A-Za-z0-9_-]{43}$/.test(device.token)) {
    throw new Error("Device registration returned an invalid bearer token.");
  }
  await credentials.save(scope, device.token);
  return device;
}
