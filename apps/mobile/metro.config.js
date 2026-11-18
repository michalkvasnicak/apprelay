const path = require("node:path");
const { getSentryExpoConfig } = require("@sentry/react-native/metro");

const config = getSentryExpoConfig(__dirname);
config.resolver.assetExts.push("glb");

// three's CommonJS build calls process.emitWarning, which React Native lacks.
// Resolve every `three` import to the ES module build, which also keeps one shared copy.
const threeModule = path.join(path.dirname(require.resolve("three")), "three.module.js");
const resolveRequest = config.resolver.resolveRequest;
config.resolver.resolveRequest = (context, moduleName, platform) => {
  if (moduleName === "three") {
    return { type: "sourceFile", filePath: threeModule };
  }
  return (resolveRequest ?? context.resolveRequest)(context, moduleName, platform);
};

module.exports = config;
