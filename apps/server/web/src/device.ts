let targetDevice: { name: string; udid: string } | undefined;

export function selectTargetDevice(udid: string) {
  const value = udid.trim();
  if (!/^(?:[a-fA-F0-9]{40}|[a-fA-F0-9]{8}-[a-fA-F0-9]{16})$/.test(value)) {
    throw new Error("Enter a valid iPhone hardware UDID");
  }
  targetDevice = { name: "Target iPhone", udid: value };
}

export function requireProvisionableDevice() {
  if (!targetDevice) {
    throw new Error("Confirm the target iPhone UDID before provisioning");
  }
  return targetDevice;
}
