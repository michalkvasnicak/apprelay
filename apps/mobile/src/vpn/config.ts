export type DeviceRegistration = {
  device_id: string;
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
  const value = process.env.EXPO_PUBLIC_VPN_API_URL?.trim();
  if (!value) {
    throw new Error("Set EXPO_PUBLIC_VPN_API_URL to the VPN server's HTTPS base URL.");
  }
  const url = new URL(value);
  if (url.protocol !== "https:" || url.username || url.password || url.search || url.hash) {
    throw new Error(
      "EXPO_PUBLIC_VPN_API_URL must be an HTTPS base URL without credentials, query or fragment.",
    );
  }
  return `${url.toString().replace(/\/$/, "")}/v1/devices`;
}

export async function registerDevice(publicKey: string): Promise<DeviceRegistration> {
  const response = await fetch(getRegistrationUrl(), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ wireguard_public_key: publicKey }),
  });
  if (response.status !== 201) {
    throw new Error(`Device registration failed (HTTP ${response.status}).`);
  }
  return (await response.json()) as DeviceRegistration;
}
