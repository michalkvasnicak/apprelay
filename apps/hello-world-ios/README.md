# Hello World iOS

Standalone SwiftUI example for iPhone and iPad (iOS 16+). No Expo, npm,
CocoaPods, VPN entitlements, server, or third-party dependencies.

Requires macOS with full Xcode installed and its first-launch setup completed.
If command-line tools point elsewhere, select Xcode:

```sh
sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
```

## Create an unsigned IPA

From the repository root:

```sh
bash apps/hello-world-ios/build-ipa.sh
```

Output: **`apps/hello-world-ios/dist/HelloWorld-unsigned.ipa`**

This is a real device build packaged with `Payload/HelloWorld.app`, not a
simulator build. No Apple account is required to create it. It is **unsigned**
and cannot be installed or launched until signed with a valid certificate and
device-compatible provisioning profile. Use it as input to a separate IPA
signing tool; AppRelay's signing frontend does not yet implement IPA signing.

The default bundle identifier is `com.example.apprelay.helloworld`. Change it in
the target's Build Settings if your signing workflow requires another identifier.

## Export a signed IPA with Xcode

1. Open `apps/hello-world-ios/HelloWorld.xcodeproj`.
2. Select the **HelloWorld** target → **Signing & Capabilities**.
3. Choose your Apple Developer team and replace the bundle identifier with one
   you own. Leave automatic signing enabled.
4. Select **Any iOS Device (arm64)** (not a simulator), then **Product → Archive**.
5. In Organizer select **Distribute App → Custom**, choose **Release Testing**
   (Ad Hoc) or **Development/Debugging**, as offered by your Xcode version,
   and export the IPA. Register the intended devices with your team when prompted.

Exporting a distributable IPA requires appropriate Apple Developer Program
membership and provisioning. A free Personal Team can run the app directly on
a connected device in Xcode but is not a substitute for distribution signing.

This minimal example has no app icon and is intended for local testing and
signing experiments, not App Store submission.
