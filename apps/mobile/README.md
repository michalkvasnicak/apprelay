# AppRelay mobile

An iOS app for connecting to the AppRelay VPN and accessing private services.

Requires a signed native build on a physical iPhone. Expo Go, the iOS simulator, Android and web are not supported.

## Prerequisites

- Node.js 22 or newer and pnpm (Node.js 24 is recommended).
- Xcode and an iPhone supported by the deployment target in `app.json`.
- An Apple Developer team with Network Extension provisioning enabled for the app and its extension (`<app bundle identifier>.PacketTunnel`).
- A reachable AppRelay server.
- Git and internet access for the first build.
- Go **1.19.13**. With Go already installed, install the required version:

  ```sh
  go install golang.org/dl/go1.19.13@latest
  "$HOME/go/bin/go1.19.13" download
  ```

  Keep the executable in `$HOME/go/bin` or set `APPRELAY_GO_BINARY` to its absolute path in the Xcode build environment. Newer Go versions are not supported for this build.

## Configuration

From the repository root, create independent configuration files:

```sh
cp apps/mobile/.env.local.example apps/mobile/.env.local
cp apps/mobile/.env.testflight.example apps/mobile/.env.testflight
```

If you already have `.env.local`, keep it; do not overwrite it. Fill in the following values in each file:

| Variable                  | Value                                                                                                                                                                               |
| ------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `APPLE_TEAM_ID`           | Your Apple Developer Team ID.                                                                                                                                                       |
| `APPLE_BUNDLE_IDENTIFIER` | The app's bundle identifier. Required for every build.                                                                                                                              |
| `EXPO_PUBLIC_VPN_API_URL` | The server's HTTPS base URL, for example `https://vpn.example.com`. Do not include credentials, a query string or a fragment.                                                       |
| `EXPO_PUBLIC_VPN_HOST`    | Direct WireGuard hostname or IP, for example `vpn.example.com`. No scheme or port. Must resolve to the VPN server, not an HTTP proxy.                                               |
| `EXPO_PUBLIC_VPN_PORT`    | Public WireGuard UDP port, matching the server and firewall configuration. Defaults to `51820`.                                                                                     |
| `EXPO_PUBLIC_API_KEY`     | Required 64-character hexadecimal app key matching the server's `APPRELAY_API_KEY`. Sent as `X-App-Key` on API requests; embedded in the app and extractable, not a private secret. |

Choose your own reverse-DNS bundle identifiers registered with your Apple Developer team. For example, replace `com.example.apprelay.dev` for local development and `com.example.apprelay` for distribution with identifiers you own. A separate local identifier lets both apps coexist; TestFlight and any future App Store release can share the same distribution identifier.

Do not put private secrets in `EXPO_PUBLIC_` variables. Both env files are ignored by Git; only the examples are committed. The API URL and app key are validated when Expo evaluates the configuration, before building.

The package scripts explicitly load just the selected file and disable Expo's automatic dotenv loading. They replace inherited app configuration rather than falling back to shell values or `.env.local`. Restart Expo after configuration changes and rebuild distributed apps to apply them.

## Build and run

From the repository root:

```sh
pnpm install
pnpm ios
```

Select your connected iPhone and approve any signing or device-trust prompts.

After native dependencies, native configuration or native source files change, regenerate the iOS project before rebuilding:

```sh
pnpm --filter @apprelay/mobile prebuild:ios
pnpm ios
```

Configure native projects through Expo configuration and plugins; do not edit generated iOS files directly. If you change the local bundle identifier, regenerate with `pnpm --filter @apprelay/mobile prebuild:ios --clean`; this deletes and recreates the generated iOS project.

## Publish to TestFlight from your Mac

### One-time setup

1. Enroll in the Apple Developer Program and create an App Store Connect app with the bundle identifier from `.env.testflight`. Enable Network Extension provisioning for both the app and `<bundle identifier>.PacketTunnel`.
2. Fill in `.env.testflight` with your Apple team ID, distribution bundle identifier, deployed server URL and app key. Set `APPLE_BUILD_NUMBER` to a build version such as `1`; increase it for every new upload.
3. Sign in to your Apple Developer account in **Xcode → Settings → Accounts**, with permission to sign and upload the app. Alternatively, supply all three optional `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID` and `APPLE_API_KEY_PATH` settings for App Store Connect API-key authentication. The key must have permission to manage signing and upload builds; keep its private `.p8` file outside the repository.
4. Install Xcode command-line tools, CocoaPods and the Go version described above. If needed, set `APPRELAY_GO_BINARY` in `.env.testflight` to the Go executable's absolute path.

No Expo account, hosted build service or Fastlane is required.

### Build and upload

From the repository root:

```sh
pnpm publish:ios:testflight
```

This command:

1. Loads only `apps/mobile/.env.testflight` and validates the Expo configuration.
2. Copies the app into a temporary build directory, without env files or existing native projects, and reuses the installed dependencies.
3. Runs Expo prebuild and CocoaPods to generate a fresh native iOS project.
4. Creates a signed Release archive using local `xcodebuild`. The explicit build number applies to both the app and its VPN extension.
5. Uses `xcodebuild -exportArchive` with an App Store Connect upload destination to send the archive directly to Apple. Upload does not run if archiving fails.

The local `.env.local` file and generated development iOS project are untouched. The temporary release project is removed afterward. Archives and export settings are saved under a unique `apps/mobile/dist/testflight-*` directory, printed by the script.

If uploading fails after archiving, you can open the saved `apprelay.xcarchive` in Xcode Organizer and choose **Distribute App → App Store Connect**, or retry `xcodebuild -exportArchive` with that archive and its saved `ExportOptions.plist`, without rebuilding.

Uploading does not release the app on the App Store or automatically invite testers. After Apple processes the build, add it to an external TestFlight group in App Store Connect, submit it for beta review as required, and enable the group's public invite link. Each build expires after 90 days.
