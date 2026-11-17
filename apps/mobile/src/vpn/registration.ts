import { registerDevice, type DeviceRegistration } from "./config";
import { vpn } from "./native";

let device: DeviceRegistration | undefined;
let pending: Promise<DeviceRegistration> | undefined;

// Both manual connection and pairing restore registration through the app’s Keychain credential.
export function ensureRegistration(): Promise<DeviceRegistration> {
  if (device && Date.parse(device.expires_at) > Date.now() + 30_000) {
    return Promise.resolve(device);
  }
  pending ??= vpn
    .getIdentity()
    .then(({ publicKey }) => registerDevice(publicKey))
    .then((registration) => {
      device = registration;
      return registration;
    })
    .finally(() => {
      pending = undefined;
    });
  return pending;
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
