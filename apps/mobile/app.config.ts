import type { ConfigContext, ExpoConfig } from "expo/config";

export default ({ config }: ConfigContext): ExpoConfig => {
  const teamId = process.env.APPLE_TEAM_ID?.trim();
  const bundleIdentifier = process.env.APPLE_BUNDLE_IDENTIFIER?.trim();
  if (!bundleIdentifier || !/^[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+$/.test(bundleIdentifier)) {
    throw new Error(
      "Set APPLE_BUNDLE_IDENTIFIER to an explicit reverse-DNS app identifier for this build (for example sk.brosoft.apprelay.dev)",
    );
  }

  return {
    ...config,
    name: config.name ?? "apprelay",
    slug: config.slug ?? "apprelay",
    ios: {
      ...config.ios,
      bundleIdentifier,
      ...(teamId ? { appleTeamId: teamId } : {}),
    },
  };
};
