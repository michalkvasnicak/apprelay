import { NativeModules, Platform } from "react-native";

import { getApiUrl } from "../api/client";
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
  getWifiConnected(): Promise<boolean | null>;
  getIdentity(): Promise<VpnIdentity>;
  getRegistrationToken(): Promise<string | null>;
  saveRegistrationToken(token: string, publicKey: string): Promise<void>;
  resetRegistration(): Promise<VpnIdentity>;
  getStatus(): Promise<VpnStatus>;
  connect(settings: {
    host: string;
    port: number;
    publicKey: string;
    clientAddress: string;
    allowedIPs: string[];
    persistentKeepaliveSeconds: number;
    remotePairingReportURL: string;
    remotePairingAppKey: string;
  }): Promise<void>;
  disconnect(): Promise<void>;
};

const native = NativeModules.AppRelayVPN as NativeVpnModule | undefined;
export const isVpnAvailable = Platform.OS === "ios" && Boolean(native);

function requireNative(): NativeVpnModule {
  if (!isVpnAvailable || !native) {
    throw new Error(
      "VPN requires a custom native iOS build on a physical iPhone or iPad; Expo Go is not supported.",
    );
  }
  return native;
}

// The UI and registration may both ask for the identity during first launch.
let pendingIdentity: Promise<VpnIdentity> | undefined;

export const vpn = {
  getWifiConnected: () => requireNative().getWifiConnected(),
  getIdentity: () =>
    (pendingIdentity ??= requireNative()
      .getIdentity()
      .finally(() => {
        pendingIdentity = undefined;
      })),
  getRegistrationToken: () => requireNative().getRegistrationToken(),
  saveRegistrationToken: (token: string, publicKey: string) =>
    requireNative().saveRegistrationToken(token, publicKey),
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
      remotePairingReportURL: `${getApiUrl()}/v1/devices/${encodeURIComponent(registration.device_id)}/remote-pairing`,
      remotePairingAppKey: process.env.EXPO_PUBLIC_API_KEY!,
    });
  },
  disconnect: () => requireNative().disconnect(),
};
