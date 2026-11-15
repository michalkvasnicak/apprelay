import type { ConfigContext, ExpoConfig } from "expo/config";

export default ({ config }: ConfigContext): ExpoConfig => {
  const teamId = process.env.APPLE_TEAM_ID?.trim();
  const encryptionComplianceCode = process.env.APPLE_ENCRYPTION_EXPORT_COMPLIANCE_CODE?.trim();
  const buildNumber = process.env.APPLE_BUILD_NUMBER?.trim();
  if (buildNumber && !/^[1-9]\d{0,3}(?:\.\d{1,2}){0,2}$/.test(buildNumber)) {
    throw new Error("APPLE_BUILD_NUMBER must be an Apple build version, for example 1 or 1.0.1.");
  }
  const bundleIdentifier = process.env.APPLE_BUNDLE_IDENTIFIER?.trim();
  if (!bundleIdentifier || !/^[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+$/.test(bundleIdentifier)) {
    throw new Error(
      "Set APPLE_BUNDLE_IDENTIFIER to an explicit reverse-DNS app identifier for this build (for example com.example.apprelay.dev)",
    );
  }

  const apiUrl = process.env.EXPO_PUBLIC_VPN_API_URL?.trim();
  if (!apiUrl) {
    throw new Error("Set EXPO_PUBLIC_VPN_API_URL to the VPN server's HTTPS base URL.");
  }
  let url: URL;
  try {
    url = new URL(apiUrl);
  } catch {
    throw new Error("EXPO_PUBLIC_VPN_API_URL must be a valid HTTPS base URL.");
  }
  if (url.protocol !== "https:" || url.username || url.password || url.search || url.hash) {
    throw new Error(
      "EXPO_PUBLIC_VPN_API_URL must be an HTTPS base URL without credentials, query or fragment.",
    );
  }

  const apiKey = process.env.EXPO_PUBLIC_API_KEY;
  if (!apiKey || !/^[a-fA-F0-9]{64}$/.test(apiKey)) {
    throw new Error("Set EXPO_PUBLIC_API_KEY to the server's 64-character hexadecimal app key.");
  }

  return {
    ...config,
    name: config.name ?? "apprelay",
    slug: config.slug ?? "apprelay",
    ios: {
      ...config.ios,
      bundleIdentifier,
      infoPlist: {
        ...config.ios?.infoPlist,
        // WireGuard bundles encryption rather than relying only on Apple's OS encryption.
        ITSAppUsesNonExemptEncryption: true,
        ...(encryptionComplianceCode
          ? { ITSEncryptionExportComplianceCode: encryptionComplianceCode }
          : {}),
        CFBundleDisplayName:
          process.env.APPRELAY_ENV === "local" ? "AppRelay VPN (Local)" : "AppRelay VPN",
      },
      ...(buildNumber ? { buildNumber } : {}),
      ...(teamId ? { appleTeamId: teamId } : {}),
    },
  };
};
