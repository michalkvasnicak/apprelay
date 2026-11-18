import { DeviceCredentialError, registerDevice, type DeviceRegistration } from "./config";
import { vpn } from "./native";

let cached: { publicKey: string; device: DeviceRegistration } | undefined;
let pending: Promise<DeviceRegistration> | undefined;

async function registerWithRecovery(): Promise<DeviceRegistration> {
  let { publicKey } = await vpn.getIdentity();
  if (
    cached?.publicKey === publicKey &&
    Date.parse(cached.device.expires_at) > Date.now() + 30_000
  ) {
    return cached.device;
  }

  cached = undefined;
  let device: DeviceRegistration;
  try {
    device = await registerDevice(publicKey);
  } catch (error) {
    if (!(error instanceof DeviceCredentialError)) {
      throw error;
    }
    // The server won't let us reclaim an existing public key without its token.
    // Retry once with a new key; a second rejection must surface.
    ({ publicKey } = await vpn.resetRegistration());
    device = await registerDevice(publicKey);
  }
  cached = { publicKey, device };
  return device;
}

// Both manual connection and pairing share one in-flight registration.
export function ensureRegistration(): Promise<DeviceRegistration> {
  return (pending ??= registerWithRecovery().finally(() => {
    pending = undefined;
  }));
}

export async function ensureTunnel(registration: DeviceRegistration) {
  const status = await vpn.getStatus();
  if (status === "disconnected") {
    await vpn.connect(registration);
    return "connecting" as const;
  }
  if (status === "connected" || status === "connecting" || status === "reconnecting") {
    return status;
  }
  throw new Error("Wait for the VPN to finish changing state, then try again.");
}
