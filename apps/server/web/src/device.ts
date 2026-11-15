const suppliedUdid = import.meta.env.VITE_DEVICE_UDID as string | undefined;

// Replace this boundary with the trusted device context when it becomes available.
export const targetDevice = {
  name: "Target iPhone",
  udid: suppliedUdid?.trim() || "00008110-0000000000000000",
  isPlaceholder: !suppliedUdid?.trim(),
} as const;

export function requireProvisionableDevice() {
  if (
    targetDevice.isPlaceholder ||
    !/^(?:[a-fA-F0-9]{40}|[a-fA-F0-9]{8}-[a-fA-F0-9]{16})$/.test(targetDevice.udid)
  ) {
    throw new Error("A real hardware UDID must be supplied before provisioning");
  }
  return targetDevice;
}
