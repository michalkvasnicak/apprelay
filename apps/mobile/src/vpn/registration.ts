import { DeviceCredentialError, registerDevice, type DeviceRegistration } from "./config";
import { vpn } from "./native";

let device: DeviceRegistration | undefined;
let pending: Promise<DeviceRegistration> | undefined;

async function registerWithRecovery(): Promise<DeviceRegistration> {
  const identity = await vpn.getIdentity();
  try {
    return await registerDevice(identity.publicKey);
  } catch (error) {
    if (!(error instanceof DeviceCredentialError)) {
      throw error;
    }
    // Only an explicit device-credential rejection permits identity replacement.
    device = undefined;
    const replacement = await vpn.resetRegistration();
    // Retry once; a second rejection must surface instead of creating a reset loop.
    return registerDevice(replacement.publicKey);
  }
}

// Both manual connection and pairing restore registration through the app’s Keychain credential.
export function ensureRegistration(): Promise<DeviceRegistration> {
  if (device && Date.parse(device.expires_at) > Date.now() + 30_000) {
    return Promise.resolve(device);
  }
  pending ??= registerWithRecovery()
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
