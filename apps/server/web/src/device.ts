import type { components } from "./api/schema";

export type PairedDevice = components["schemas"]["PairedDevice"];

let targetDevice: PairedDevice | undefined;

export function selectTargetDevice(device: PairedDevice) {
  // Until hardware identity retrieval is implemented, the server supplies the
  // remote-pairing identifier as the provisional provisioning UDID.
  const expiresAt = Date.parse(device.expires_at);
  if (!device.udid.trim() || !Number.isFinite(expiresAt) || expiresAt <= Date.now()) {
    throw new Error("Pair your iPhone again before provisioning");
  }
  targetDevice = { ...device };
}

export function clearTargetDevice() {
  targetDevice = undefined;
}

export function requireProvisionableDevice() {
  if (!targetDevice || Date.parse(targetDevice.expires_at) <= Date.now()) {
    throw new Error("Pair your iPhone again before provisioning");
  }
  return targetDevice;
}
