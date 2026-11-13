const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const REPOSITORY = "https://git.zx2c4.com/wireguard-apple";
const REVISION = "2fec12a6e1f6e3460b6ee483aa00ad29cddadab1";

// Materialize a local package so the manifest can be patched BEFORE SwiftPM
// parses it. Do not modify Xcode's global SwiftPM cache or generated files by hand.
module.exports = function prepareWireGuardPackage(projectRoot, destination) {
  const checkout = path.join(projectRoot, ".expo", "wireguard-apple", REVISION);
  const git = (args) =>
    execFileSync("git", ["-C", checkout, ...args], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });

  fs.mkdirSync(checkout, { recursive: true });
  try {
    if (!fs.existsSync(path.join(checkout, ".git"))) {
      git(["init"]);
    }
    if (!fs.existsSync(path.join(checkout, "Package.swift"))) {
      git(["fetch", "--depth=1", REPOSITORY, REVISION]);
      git(["checkout", "--detach", "FETCH_HEAD"]);
    }
    if (git(["rev-parse", "HEAD"]).trim() !== REVISION) {
      throw new Error("Unexpected WireGuard source revision");
    }
  } catch (error) {
    throw new Error(
      `Unable to prepare WireGuardKit. Git and network access are required on the first prebuild. ${error.message}`,
    );
  }

  for (const relative of [
    "Package.swift",
    "COPYING",
    "Sources/WireGuardKit",
    "Sources/WireGuardKitC",
    "Sources/WireGuardKitGo",
  ]) {
    const target = path.join(destination, relative);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.cpSync(path.join(checkout, relative), target, { recursive: true });
  }

  const manifestPath = path.join(destination, "Package.swift");
  const manifest = fs.readFileSync(manifestPath, "utf8");
  if (!manifest.startsWith("// swift-tools-version:5.3\n")) {
    throw new Error("Unexpected pinned WireGuardKit manifest; review the compatibility patch");
  }
  // .macOS(.v12) and .iOS(.v15) require PackageDescription 5.5, not 5.3.
  fs.writeFileSync(
    manifestPath,
    manifest.replace("// swift-tools-version:5.3", "// swift-tools-version:5.5"),
  );

  // Explicit Clang modules require the header to import its Darwin typedefs
  // itself instead of relying on transitive imports from a Swift compilation.
  const headerPath = path.join(destination, "Sources/WireGuardKitC/WireGuardKitC.h");
  const header = fs.readFileSync(headerPath, "utf8");
  const include = '#include "key.h"';
  if (header.split(include).length !== 2) {
    throw new Error("Unexpected pinned WireGuardKit C header; review the compatibility patch");
  }
  fs.writeFileSync(headerPath, header.replace(include, `#include <sys/types.h>\n\n${include}`));
};
