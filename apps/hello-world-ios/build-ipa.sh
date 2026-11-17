#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"

# No certificate, provisioning profile, dependencies, or server required.
# The result must be signed separately before it can run on an iPhone.
xcodebuild \
  -project HelloWorld.xcodeproj \
  -scheme HelloWorld \
  -configuration Release \
  -destination 'generic/platform=iOS' \
  -derivedDataPath dist/DerivedData \
  CODE_SIGNING_ALLOWED=NO \
  build

mkdir -p dist
staging="$(mktemp -d "$PWD/dist/ipa.XXXXXX")"
trap 'rm -rf "$staging"' EXIT
mkdir -p "$staging/Payload"
ditto dist/DerivedData/Build/Products/Release-iphoneos/HelloWorld.app \
  "$staging/Payload/HelloWorld.app"
(cd "$staging" && /usr/bin/zip -qr HelloWorld-unsigned.ipa Payload)
mv -f "$staging/HelloWorld-unsigned.ipa" dist/HelloWorld-unsigned.ipa

printf '\nUnsigned IPA: %s/dist/HelloWorld-unsigned.ipa\n' "$PWD"
