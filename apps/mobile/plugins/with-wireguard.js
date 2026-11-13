const fs = require("node:fs");
const path = require("node:path");
const { withEntitlementsPlist, withInfoPlist, withXcodeProject } = require("expo/config-plugins");

const TARGET = "AppRelayPacketTunnel";
const DIRECTORY = "AppRelayVPN";
// Pin the reviewed upstream sources, including the matching Go bridge.
const WIREGUARD_REVISION = "2fec12a6e1f6e3460b6ee483aa00ad29cddadab1";
const quote = (value) => JSON.stringify(value);

module.exports = function withWireGuard(config) {
  const identifier = config.ios?.bundleIdentifier;
  if (!identifier || !/^[A-Za-z0-9.-]+$/.test(identifier)) {
    throw new Error("WireGuard requires ios.bundleIdentifier in the Expo configuration");
  }
  const extensionIdentifier = `${identifier}.PacketTunnel`;
  const keychainGroup = `$(AppIdentifierPrefix)${identifier}.vpn`;
  const entitlements = {
    "com.apple.developer.networking.networkextension": ["packet-tunnel-provider"],
    "keychain-access-groups": [keychainGroup],
  };

  // EAS needs this before prebuild in order to provision the extension target.
  config.extra ??= {};
  config.extra.eas ??= {};
  config.extra.eas.build ??= {};
  config.extra.eas.build.experimental ??= {};
  config.extra.eas.build.experimental.ios ??= {};
  const easIos = config.extra.eas.build.experimental.ios;
  easIos.appExtensions = [
    ...(easIos.appExtensions ?? []).filter((extension) => extension.targetName !== TARGET),
    { targetName: TARGET, bundleIdentifier: extensionIdentifier, entitlements },
  ];

  config = withEntitlementsPlist(config, (mod) => {
    for (const [key, values] of Object.entries(entitlements)) {
      mod.modResults[key] = [...new Set([...(mod.modResults[key] ?? []), ...values])];
    }
    return mod;
  });
  config = withInfoPlist(config, (mod) => {
    mod.modResults.AppRelayKeychainGroup = keychainGroup;
    return mod;
  });

  return withXcodeProject(config, (mod) => {
    const project = mod.modResults;
    const iosRoot = mod.modRequest.platformProjectRoot;
    const destination = path.join(iosRoot, DIRECTORY);
    fs.mkdirSync(destination, { recursive: true });
    fs.cpSync(path.join(__dirname, "../native/vpn"), destination, { recursive: true });
    fs.writeFileSync(
      path.join(destination, "PacketTunnel.entitlements"),
      plist(`
      <key>com.apple.developer.networking.networkextension</key><array><string>packet-tunnel-provider</string></array>
      <key>keychain-access-groups</key><array><string>${keychainGroup}</string></array>`),
    );
    fs.writeFileSync(
      path.join(destination, "PacketTunnel-Info.plist"),
      plist(`
      <key>CFBundleDisplayName</key><string>AppRelay VPN</string>
      <key>CFBundleIdentifier</key><string>$(PRODUCT_BUNDLE_IDENTIFIER)</string>
      <key>CFBundleExecutable</key><string>$(EXECUTABLE_NAME)</string>
      <key>CFBundleName</key><string>$(PRODUCT_NAME)</string>
      <key>CFBundlePackageType</key><string>XPC!</string>
      <key>CFBundleShortVersionString</key><string>$(MARKETING_VERSION)</string>
      <key>CFBundleVersion</key><string>$(CURRENT_PROJECT_VERSION)</string>
      <key>AppRelayKeychainGroup</key><string>${keychainGroup}</string>
      <key>NSExtension</key><dict>
        <key>NSExtensionPointIdentifier</key><string>com.apple.networkextension.packet-tunnel</string>
        <key>NSExtensionPrincipalClass</key><string>$(PRODUCT_MODULE_NAME).PacketTunnelProvider</string>
      </dict>`),
    );

    // Source files are refreshed above on every prebuild; targets are added once.
    if (
      Object.values(project.pbxNativeTargetSection()).some(
        (target) => target?.name?.replaceAll('"', "") === TARGET,
      )
    ) {
      return mod;
    }
    project.hash.project.objects.PBXTargetDependency ??= {};
    project.hash.project.objects.PBXContainerItemProxy ??= {};
    const app = project.getFirstTarget();
    const extension = project.addTarget(TARGET, "app_extension", DIRECTORY, extensionIdentifier);
    project.addBuildPhase([], "PBXSourcesBuildPhase", "Sources", extension.uuid);
    project.addBuildPhase([], "PBXFrameworksBuildPhase", "Frameworks", extension.uuid);
    project.addBuildPhase([], "PBXResourcesBuildPhase", "Resources", extension.uuid);
    const group = project.addPbxGroup([], DIRECTORY);
    project.addToPbxGroup(group.uuid, project.getFirstProject().firstProject.mainGroup);

    for (const file of [
      "VpnSettings.swift",
      "VpnKeychain.swift",
      "AppRelayVPN.swift",
      "AppRelayVPNBridge.m",
    ]) {
      addSource(project, `${DIRECTORY}/${file}`, app.uuid, group.uuid);
    }
    for (const file of ["VpnSettings.swift", "VpnKeychain.swift", "PacketTunnelProvider.swift"]) {
      addSource(project, `${DIRECTORY}/${file}`, extension.uuid, group.uuid);
    }

    const appSettings = configurations(project, app.firstTarget).map(
      (entry) => entry.buildSettings,
    );
    const team =
      config.ios?.appleTeamId ??
      appSettings.find((entry) => entry.DEVELOPMENT_TEAM)?.DEVELOPMENT_TEAM;
    for (const settings of appSettings) {
      settings.ENABLE_BITCODE = "NO";
      settings.INFOPLIST_EXPAND_BUILD_SETTINGS = "YES";
    }
    for (const entry of configurations(project, extension.pbxNativeTarget)) {
      Object.assign(entry.buildSettings, {
        INFOPLIST_FILE: quote(`${DIRECTORY}/PacketTunnel-Info.plist`),
        CODE_SIGN_ENTITLEMENTS: quote(`${DIRECTORY}/PacketTunnel.entitlements`),
        CODE_SIGN_STYLE: "Automatic",
        SWIFT_VERSION: "5.0",
        PRODUCT_MODULE_NAME: TARGET,
        IPHONEOS_DEPLOYMENT_TARGET: config.ios?.deploymentTarget ?? "15.0",
        TARGETED_DEVICE_FAMILY: quote(config.ios?.supportsTablet ? "1,2" : "1"),
        APPLICATION_EXTENSION_API_ONLY: "YES",
        ENABLE_BITCODE: "NO",
        ENABLE_USER_SCRIPT_SANDBOXING: "NO",
        GENERATE_INFOPLIST_FILE: "NO",
        INFOPLIST_EXPAND_BUILD_SETTINGS: "YES",
        MARKETING_VERSION: quote(config.version ?? "1.0.0"),
        CURRENT_PROJECT_VERSION: quote(config.ios?.buildNumber ?? "1"),
        LIBRARY_SEARCH_PATHS: [quote("$(inherited)"), quote("$(BUILT_PRODUCTS_DIR)")],
        OTHER_LDFLAGS: [quote("$(inherited)"), quote("-lwg-go"), quote("-lresolv")],
      });
      if (team) {
        entry.buildSettings.DEVELOPMENT_TEAM = team;
      }
    }
    addWireGuardPackage(project, extension);
    const phase = project.addBuildPhase(
      [],
      "PBXShellScriptBuildPhase",
      "Build WireGuard Go bridge",
      extension.uuid,
      {
        shellPath: "/bin/sh",
        shellScript: '/bin/sh "${SRCROOT}/AppRelayVPN/build-wireguard-go.sh"',
        inputPaths: [],
        outputPaths: [],
      },
    );
    phase.buildPhase.alwaysOutOfDate = 1;
    // The bridge must exist before the extension links WireGuardKit.
    const phases = extension.pbxNativeTarget.buildPhases;
    phases.unshift(phases.pop());
    return mod;
  });
};

function plist(body) {
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>${body}</dict></plist>\n`;
}

function configurations(project, target) {
  return project
    .pbxXCConfigurationList()
    [target.buildConfigurationList].buildConfigurations.map(
      ({ value }) => project.pbxXCBuildConfigurationSection()[value],
    );
}

function addSource(project, file, target, group) {
  const references = project.pbxFileReferenceSection();
  let reference = Object.keys(references).find(
    (key) => !key.endsWith("_comment") && references[key].path?.replaceAll('"', "") === file,
  );
  if (!reference) {
    reference = project.addFile(file, group).fileRef;
  }
  const uuid = project.generateUuid();
  project.pbxBuildFileSection()[uuid] = {
    isa: "PBXBuildFile",
    fileRef: reference,
    fileRef_comment: path.basename(file),
  };
  project.pbxBuildFileSection()[`${uuid}_comment`] = `${path.basename(file)} in Sources`;
  project
    .pbxSourcesBuildPhaseObj(target)
    .files.push({ value: uuid, comment: `${path.basename(file)} in Sources` });
}

function addWireGuardPackage(project, extension) {
  const objects = project.hash.project.objects;
  const packageID = project.generateUuid();
  objects.XCRemoteSwiftPackageReference ??= {};
  objects.XCRemoteSwiftPackageReference[packageID] = {
    isa: "XCRemoteSwiftPackageReference",
    repositoryURL: quote("https://git.zx2c4.com/wireguard-apple"),
    requirement: { kind: "revision", revision: quote(WIREGUARD_REVISION) },
  };
  const root = project.getFirstProject().firstProject;
  root.packageReferences ??= [];
  root.packageReferences.push({ value: packageID, comment: "WireGuardKit" });
  const productID = project.generateUuid();
  objects.XCSwiftPackageProductDependency ??= {};
  objects.XCSwiftPackageProductDependency[productID] = {
    isa: "XCSwiftPackageProductDependency",
    package: packageID,
    productName: "WireGuardKit",
  };
  extension.pbxNativeTarget.packageProductDependencies = [
    { value: productID, comment: "WireGuardKit" },
  ];
  const buildID = project.generateUuid();
  project.pbxBuildFileSection()[buildID] = {
    isa: "PBXBuildFile",
    productRef: productID,
    productRef_comment: "WireGuardKit",
  };
  project.pbxBuildFileSection()[`${buildID}_comment`] = "WireGuardKit in Frameworks";
  project
    .pbxFrameworksBuildPhaseObj(extension.uuid)
    .files.push({ value: buildID, comment: "WireGuardKit in Frameworks" });
}
