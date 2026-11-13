import type { ConfigContext, ExpoConfig } from "expo/config";

export default ({ config }: ConfigContext): ExpoConfig => {
  const teamId = process.env.APPLE_TEAM_ID?.trim();
  const bundleIdentifier = process.env.APPLE_BUNDLE_IDENTIFIER?.trim();

  return {
    ...config,
    name: config.name ?? "apprelay",
    slug: config.slug ?? "apprelay",
    ios: {
      ...config.ios,
      bundleIdentifier: bundleIdentifier || config.ios?.bundleIdentifier,
      ...(teamId ? { appleTeamId: teamId } : {}),
    },
  };
};
