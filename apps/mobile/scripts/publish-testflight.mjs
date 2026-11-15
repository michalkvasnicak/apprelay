import { spawnSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, relative, resolve, sep } from "node:path";

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: "inherit", ...options });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} failed (${result.signal ?? result.status}).`);
  }
}

function publish() {
  if (process.platform !== "darwin") {
    throw new Error("Local iOS publishing requires macOS and Xcode.");
  }
  const teamId = process.env.APPLE_TEAM_ID?.trim();
  if (!teamId || !/^[A-Z0-9]{10}$/.test(teamId)) {
    throw new Error("Set APPLE_TEAM_ID to your 10-character Apple team ID in .env.testflight.");
  }
  if (!process.env.APPLE_BUILD_NUMBER?.trim()) {
    throw new Error("Set APPLE_BUILD_NUMBER in .env.testflight; increase it for each new upload.");
  }

  const auth = [];
  const keyId = process.env.APPLE_API_KEY_ID?.trim();
  const issuerId = process.env.APPLE_API_ISSUER_ID?.trim();
  const keyPath = process.env.APPLE_API_KEY_PATH?.trim();
  if (keyId || issuerId || keyPath) {
    if (!keyId || !issuerId || !keyPath || !existsSync(keyPath)) {
      throw new Error(
        "Supply APPLE_API_KEY_ID, APPLE_API_ISSUER_ID and an existing APPLE_API_KEY_PATH together.",
      );
    }
    auth.push(
      "-authenticationKeyID",
      keyId,
      "-authenticationKeyIssuerID",
      issuerId,
      "-authenticationKeyPath",
      resolve(keyPath),
    );
  }

  // Fail on missing or invalid app configuration before generating or signing anything.
  run("pnpm", ["exec", "expo", "config", "--type", "public"], {
    stdio: ["inherit", "ignore", "inherit"],
  });

  const projectRoot = process.cwd();
  if (!existsSync("node_modules")) {
    throw new Error("Install dependencies with pnpm install before publishing.");
  }
  mkdirSync("dist", { recursive: true });
  const output = mkdtempSync(resolve("dist/testflight-"));
  const archive = join(output, "apprelay.xcarchive");
  const exportOptions = join(output, "ExportOptions.plist");
  writeFileSync(
    exportOptions,
    `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>method</key><string>app-store-connect</string>
  <key>destination</key><string>upload</string>
  <key>teamID</key><string>${teamId}</string>
  <key>signingStyle</key><string>automatic</string>
  <key>manageAppVersionAndBuildNumber</key><false/>
  <key>uploadSymbols</key><true/>
</dict></plist>
`,
  );

  // Generate a separate release project, leaving the local iOS project and env files untouched.
  const staging = mkdtempSync(join(tmpdir(), "apprelay-testflight-"));
  try {
    cpSync(projectRoot, staging, {
      recursive: true,
      filter(source) {
        const topLevel = relative(projectRoot, source).split(sep)[0];
        return (
          !["node_modules", "ios", "android", "dist", ".expo", ".git"].includes(topLevel) &&
          !topLevel.startsWith(".env")
        );
      },
    });
    symlinkSync(join(projectRoot, "node_modules"), join(staging, "node_modules"), "dir");
    run("pnpm", ["exec", "expo", "prebuild", "--platform", "ios", "--no-install"], {
      cwd: staging,
    });
    run("pod", ["install"], { cwd: join(staging, "ios") });

    const workspaces = readdirSync(join(staging, "ios")).filter((name) =>
      name.endsWith(".xcworkspace"),
    );
    if (workspaces.length !== 1) {
      throw new Error("Expected exactly one generated iOS workspace.");
    }
    const workspace = join(staging, "ios", workspaces[0]);
    run("xcodebuild", [
      "-workspace",
      workspace,
      "-scheme",
      basename(workspace, ".xcworkspace"),
      "-configuration",
      "Release",
      "-destination",
      "generic/platform=iOS",
      "-archivePath",
      archive,
      "-allowProvisioningUpdates",
      ...auth,
      `DEVELOPMENT_TEAM=${teamId}`,
      "CODE_SIGN_STYLE=Automatic",
      `CURRENT_PROJECT_VERSION=${process.env.APPLE_BUILD_NUMBER.trim()}`,
      "archive",
    ]);
    console.log(`Archive saved to ${archive}`);
    run("xcodebuild", [
      "-exportArchive",
      "-archivePath",
      archive,
      "-exportPath",
      join(output, "upload"),
      "-exportOptionsPlist",
      exportOptions,
      "-allowProvisioningUpdates",
      ...auth,
    ]);
  } finally {
    rmSync(staging, { recursive: true, force: true });
  }
}

try {
  publish();
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
