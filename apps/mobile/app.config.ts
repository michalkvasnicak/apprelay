import type { ConfigContext, ExpoConfig } from "expo/config";

export default ({ config }: ConfigContext): ExpoConfig => {
  const teamId = process.env.APPLE_TEAM_ID?.trim();
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

  const scheme = process.env.EXPO_PUBLIC_APP_SCHEME;
  if (!scheme || !/^[a-z][a-z0-9+.-]*$/.test(scheme)) {
    throw new Error("Set EXPO_PUBLIC_APP_SCHEME to a lowercase URL scheme without ://.");
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

  const vpnHost = process.env.EXPO_PUBLIC_VPN_HOST?.trim();
  if (!vpnHost) {
    throw new Error("Set EXPO_PUBLIC_VPN_HOST to the direct WireGuard server hostname or IP.");
  }
  let vpnUrl: URL;
  try {
    vpnUrl = new URL(`http://${vpnHost}`);
  } catch {
    throw new Error("EXPO_PUBLIC_VPN_HOST must be a hostname or IP, not a URL.");
  }
  if (
    vpnUrl.hostname !== vpnHost.toLowerCase() ||
    vpnUrl.port ||
    vpnUrl.username ||
    vpnUrl.password ||
    vpnUrl.pathname !== "/" ||
    vpnUrl.search ||
    vpnUrl.hash
  ) {
    throw new Error("EXPO_PUBLIC_VPN_HOST must contain only a hostname or IP, without a port.");
  }
  const vpnPort = process.env.EXPO_PUBLIC_VPN_PORT ?? "51820";
  if (!/^\d{1,5}$/.test(vpnPort) || Number(vpnPort) < 1 || Number(vpnPort) > 65535) {
    throw new Error("EXPO_PUBLIC_VPN_PORT must be an integer between 1 and 65535.");
  }

  const apiKey = process.env.EXPO_PUBLIC_API_KEY;
  if (!apiKey || !/^[a-fA-F0-9]{64}$/.test(apiKey)) {
    throw new Error("Set EXPO_PUBLIC_API_KEY to the server's 64-character hexadecimal app key.");
  }

  let name = config.name ?? "AppRelay VPN";
  const simulator = process.env.APPRELAY_ENV === "simulator";
  if (simulator) {
    name += " (Simulator)";
  } else if (process.env.APPRELAY_ENV === "local") {
    name += " (Local)";
  }

  const sentryOrg = process.env.SENTRY_ORG?.trim();
  const sentryProject = process.env.SENTRY_PROJECT?.trim();
  const sentryUrl = process.env.SENTRY_URL?.trim();

  return {
    ...config,
    name,
    slug: config.slug ?? "apprelay",
    plugins: [
      ...(config.plugins ?? []).filter(
        (plugin) => !simulator || plugin !== "./plugins/with-wireguard",
      ),
      ...(sentryOrg && sentryProject && sentryUrl
        ? [
            [
              "@sentry/react-native/expo",
              { organization: sentryOrg, project: sentryProject, url: sentryUrl },
            ] satisfies [string, Record<string, string>],
          ]
        : []),
    ],
    scheme: simulator ? `${scheme}-simulator` : scheme,
    ios: {
      ...config.ios,
      bundleIdentifier: simulator ? `${bundleIdentifier}.simulator` : bundleIdentifier,
      infoPlist: {
        ...config.ios?.infoPlist,
        // Matches Apple's no-documentation assessment with France excluded from distribution.
        // Reassess before changing encryption or distribution territories.
        ITSAppUsesNonExemptEncryption: false,
      },
      ...(buildNumber ? { buildNumber } : {}),
      ...(teamId ? { appleTeamId: teamId } : {}),
    },
  };
};
