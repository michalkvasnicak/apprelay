export type DeviceRegistration = {
  device_id: string;
  address: string;
  expires_at: string;
  lifetime_seconds: number;
  wireguard: {
    server_public_key: string;
    endpoint: string;
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
  const data: unknown = await response.json();
  if (!isRegistration(data)) {
    throw new Error("The VPN server returned an invalid device registration.");
  }
  return data;
}

function isRegistration(value: unknown): value is DeviceRegistration {
  if (!value || typeof value !== "object") {
    return false;
  }
  const data = value as Partial<DeviceRegistration>;
  const wg = data.wireguard;
  return (
    typeof data.device_id === "string" &&
    data.device_id.length > 0 &&
    typeof data.address === "string" &&
    /^.+\/32$/.test(data.address) &&
    typeof data.expires_at === "string" &&
    Date.parse(data.expires_at) > Date.now() &&
    typeof data.lifetime_seconds === "number" &&
    Number.isInteger(data.lifetime_seconds) &&
    data.lifetime_seconds > 0 &&
    !!wg &&
    typeof wg.server_public_key === "string" &&
    /^[A-Za-z0-9+/]{42}[AEIMQUYcgkosw048]=$/.test(wg.server_public_key) &&
    typeof wg.endpoint === "string" &&
    /^[^:\s]+:\d+$/.test(wg.endpoint) &&
    Array.isArray(wg.allowed_ips) &&
    wg.allowed_ips.length > 0 &&
    wg.allowed_ips.every((ip: unknown) => typeof ip === "string" && /^.+\/\d+$/.test(ip)) &&
    Number.isInteger(wg.persistent_keepalive_seconds) &&
    wg.persistent_keepalive_seconds >= 0 &&
    wg.persistent_keepalive_seconds <= 65535
  );
}
