import { NativeModules, Platform } from "react-native";

import { getRegistrationUrl, type DeviceRegistration } from "./config";

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
  getStatus(): Promise<VpnStatus>;
  connect(settings: {
    host: string;
    port: number;
    publicKey: string;
    clientAddress: string;
    allowedIPs: string[];
    persistentKeepaliveSeconds: number;
  }): Promise<void>;
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
  getStatus: () => requireNative().getStatus(),
  connect: (registration: DeviceRegistration) => {
    const { wireguard } = registration;
    const host = new URL(getRegistrationUrl()).hostname;
    return requireNative().connect({
      host,
      port: 51820,
      publicKey: wireguard.server_public_key,
      clientAddress: registration.address,
      allowedIPs: wireguard.allowed_ips,
      persistentKeepaliveSeconds: wireguard.persistent_keepalive_seconds,
    });
  },
  disconnect: () => requireNative().disconnect(),
};
