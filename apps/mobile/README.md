# AppRelay mobile

An iOS app for connecting to the AppRelay VPN and accessing private services.

Requires a signed native build on a physical iPhone. Expo Go, the iOS simulator, Android and web are not supported.

## Prerequisites

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

Copy `apps/mobile/.env.example` to `apps/mobile/.env.local` and set:

| Variable                  | Value                                                                                                                                                                               |
| ------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `APPLE_TEAM_ID`           | Your Apple Developer Team ID.                                                                                                                                                       |
| `APPLE_BUNDLE_IDENTIFIER` | The app's bundle identifier. Required for every build.                                                                                                                              |
| `EXPO_PUBLIC_VPN_API_URL` | The server's HTTPS base URL, for example `https://vpn.example.com`. Do not include credentials, a query string or a fragment.                                                       |
| `EXPO_PUBLIC_API_KEY`     | Required 64-character hexadecimal app key matching the server's `APPRELAY_API_KEY`. Sent as `X-App-Key` on API requests; embedded in the app and extractable, not a private secret. |

Do not put private secrets in `EXPO_PUBLIC_` variables. Keep `.env.local` out of version control.

Provide these variables in the release build environment as well. Restart Expo after configuration changes and rebuild distributed apps to apply them.

## Build and run

From the repository root:

```sh
pnpm install
pnpm ios
```

Select your connected iPhone and approve any signing or device-trust prompts.

After native dependencies, native configuration or native source files change, regenerate the iOS project before rebuilding:

```sh
pnpm --filter @apprelay/mobile exec expo prebuild --platform ios --no-install
pnpm ios
```

Configure native projects through Expo configuration and plugins; do not edit generated iOS files directly.
