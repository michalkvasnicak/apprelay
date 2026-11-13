export type VpnServerConfig = {
  endpoint: {
    host: string;
    port: number;
  };
  publicKey: string;
  tunnelAddress: string;
  allowedIPs: readonly [`${string}/32`];
};

// Call before configuring the native tunnel, not during module initialization.
export function getVpnServerConfig(): VpnServerConfig {
  // Expo requires direct process.env.EXPO_PUBLIC_* references for inlining.
  const host = requireIPv4(process.env.EXPO_PUBLIC_WG_SERVER_HOST, "EXPO_PUBLIC_WG_SERVER_HOST");
  const portValue = process.env.EXPO_PUBLIC_WG_SERVER_PORT?.trim();
  const port = Number(portValue);
  if (
    !portValue ||
    !/^\d+$/.test(portValue) ||
    !Number.isInteger(port) ||
    port < 1 ||
    port > 65535
  ) {
    throw new Error("EXPO_PUBLIC_WG_SERVER_PORT must be an integer from 1 to 65535");
  }

  const publicKey = process.env.EXPO_PUBLIC_WG_SERVER_PUBLIC_KEY?.trim();
  if (!publicKey || !/^[A-Za-z0-9+/]{42}[AEIMQUYcgkosw048]=$/.test(publicKey)) {
    throw new Error(
      "EXPO_PUBLIC_WG_SERVER_PUBLIC_KEY must be a base64-encoded 32-byte WireGuard key",
    );
  }

  const tunnelAddress = requireIPv4(
    process.env.EXPO_PUBLIC_WG_SERVER_TUNNEL_IP ?? "10.77.0.1",
    "EXPO_PUBLIC_WG_SERVER_TUNNEL_IP",
  );
  if (host === tunnelAddress) {
    throw new Error("The WireGuard endpoint must differ from the server's tunnel address");
  }

  return {
    endpoint: { host, port },
    publicKey,
    tunnelAddress,
    // Intentionally one host only: never a subnet or a default route.
    allowedIPs: [`${tunnelAddress}/32`],
  } satisfies VpnServerConfig;
}

function requireIPv4(value: string | undefined, name: string): string {
  const address = value?.trim();
  const octets = address?.split(".");
  if (
    !address ||
    octets?.length !== 4 ||
    !octets.every((octet) => /^(0|[1-9]\d{0,2})$/.test(octet) && Number(octet) <= 255)
  ) {
    throw new Error(`${name} must be an IPv4 address without a port or CIDR prefix`);
  }
  if (Number(octets[0]) === 0 || Number(octets[0]) === 127 || Number(octets[0]) >= 224) {
    throw new Error(`${name} must be a unicast address reachable from the iPhone`);
  }
  return address;
}
