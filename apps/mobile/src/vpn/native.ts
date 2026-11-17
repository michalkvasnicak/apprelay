import { NativeModules, Platform } from "react-native";

import type { DeviceRegistration } from "./config";

export const vpnStatuses = [
  "disconnected",
  "connecting",
  "connected",
  "reconnecting",
  "disconnecting",
  "unknown",
] as const;

export type VpnStatus = (typeof vpnStatuses)[number];
export type VpnIdentity = { publicKey: string };

type NativeVpnModule = {
  getIdentity(): Promise<VpnIdentity>;
  resetRegistration(): Promise<VpnIdentity>;
  getStatus(): Promise<VpnStatus>;
  connect(settings: {
    host: string;
    port: number;
    publicKey: string;
    clientAddress: string;
    allowedIPs: string[];
    persistentKeepaliveSeconds: number;
  }): Promise<void>;
  preparePairing(): Promise<void>;
  disconnect(): Promise<void>;
};

const native = NativeModules.AppRelayVPN as NativeVpnModule | undefined;
export const isVpnAvailable = Platform.OS === "ios" && Boolean(native);

function requireNative(): NativeVpnModule {
  if (!isVpnAvailable || !native) {
    throw new Error(
      "VPN requires a custom native iOS build on a physical iPhone; Expo Go is not supported.",
    );
  }
  return native;
}

export const vpn = {
  getIdentity: () => requireNative().getIdentity(),
  resetRegistration: () => requireNative().resetRegistration(),
  getStatus: () => requireNative().getStatus(),
  connect: (registration: DeviceRegistration) => {
    const { wireguard } = registration;
    // The HTTPS API may be proxied; WireGuard needs the direct UDP endpoint.
    // These values are validated by app.config.ts before starting/building Expo.
    return requireNative().connect({
      host: process.env.EXPO_PUBLIC_VPN_HOST!.trim(),
      port: Number(process.env.EXPO_PUBLIC_VPN_PORT ?? "51820"),
      publicKey: wireguard.server_public_key,
      clientAddress: registration.address,
      allowedIPs: wireguard.allowed_ips,
      persistentKeepaliveSeconds: wireguard.persistent_keepalive_seconds,
    });
  },
  preparePairing: () => requireNative().preparePairing(),
  disconnect: () => requireNative().disconnect(),
};
