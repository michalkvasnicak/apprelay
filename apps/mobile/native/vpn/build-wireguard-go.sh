#!/bin/sh
set -eu

# Upstream's runtime patch targets Go 1.19. Use its pinned toolchain rather
# than silently trying to patch an arbitrary system Go installation.
if [ "${PLATFORM_NAME:-}" != "iphoneos" ]; then
  echo "error: AppRelay WireGuard requires a physical iOS device build, not a simulator." >&2
  exit 1
fi
export PATH="${HOME}/go/bin:/opt/homebrew/bin:/usr/local/bin:${PATH}"
go_binary="$(command -v "${APPRELAY_GO_BINARY:-go1.19.13}" || true)"
if [ -z "$go_binary" ]; then
  echo "error: Install the WireGuard toolchain: go install golang.org/dl/go1.19.13@latest && go1.19.13 download" >&2
  exit 1
fi
case "$("$go_binary" version)" in
  *" go1.19.13 "*) ;;
  *) echo "error: The pinned WireGuard bridge requires Go 1.19.13." >&2; exit 1 ;;
esac

bridge="${BUILD_DIR%Build/*}SourcePackages/checkouts/wireguard-apple/Sources/WireGuardKitGo"
if [ ! -f "$bridge/Makefile" ]; then
  echo "error: Resolve the WireGuardKit Swift package in Xcode before building." >&2
  exit 1
fi
# The upstream Makefile invokes 'go'. Keep the selected Go version local to
# this build and leave the developer's default toolchain unchanged.
shim="${DERIVED_FILE_DIR}/apprelay-go-bin"
mkdir -p "$shim"
ln -sf "$go_binary" "$shim/go"
export PATH="$shim:$PATH"
exec /usr/bin/make -C "$bridge" build \
  "CONFIGURATION_TEMP_DIR=${TARGET_TEMP_DIR}" \
  "CONFIGURATION_BUILD_DIR=${BUILT_PRODUCTS_DIR}" \
  "PLATFORM_NAME=${PLATFORM_NAME}" \
  "SDKROOT=${SDKROOT}" \
  "ARCHS=${ARCHS}" \
  "DEPLOYMENT_TARGET_CLANG_FLAG_NAME=miphoneos-version-min" \
  "DEPLOYMENT_TARGET_CLANG_ENV_NAME=IPHONEOS_DEPLOYMENT_TARGET" \
  "IPHONEOS_DEPLOYMENT_TARGET=${IPHONEOS_DEPLOYMENT_TARGET}"
