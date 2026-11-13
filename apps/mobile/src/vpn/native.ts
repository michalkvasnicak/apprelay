import { NativeModules, Platform } from "react-native";

import type { VpnServerConfig } from "./config";

export const vpnStatuses = [
  "disconnected",
  "connecting",
  "connected",
  "reconnecting",
  "disconnecting",
  "unknown",
] as const;

export type VpnStatus = (typeof vpnStatuses)[number];
export type VpnIdentity = { publicKey: string; clientAddress: string };

type NativeVpnModule = {
  getIdentity(): Promise<VpnIdentity>;
  getStatus(): Promise<VpnStatus>;
  connect(settings: {
    host: string;
    port: number;
    publicKey: string;
    serverAddress: string;
    clientAddress: string;
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
  connect: (server: VpnServerConfig, clientAddress: string) =>
    requireNative().connect({
      host: server.endpoint.host,
      port: server.endpoint.port,
      publicKey: server.publicKey,
      serverAddress: server.tunnelAddress,
      clientAddress: clientAddress.trim(),
    }),
  disconnect: () => requireNative().disconnect(),
};
