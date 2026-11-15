import { parsePlist } from "../apple/plist";
import { SigningError } from "./types";

const basicKeys = [
  "application-identifier",
  "com.apple.developer.team-identifier",
  "get-task-allow",
  "keychain-access-groups",
] as const;

// No capability rewriting in this first version. Only the ordinary app-specific
// identity/keychain values may change to those in the new development profile.
export function validateBasicEntitlements(value: Record<string, unknown>, bundleId: string) {
  for (const [key, item] of Object.entries(value)) {
    if (!basicKeys.some((allowed) => allowed === key)) {
      throw new SigningError("This app or profile requires capabilities not supported yet");
    }
    if (key === "get-task-allow") {
      if (typeof item !== "boolean") {
        throw new SigningError("Invalid development entitlement");
      }
    } else if (key === "keychain-access-groups") {
      if (
        !Array.isArray(item) ||
        item.length !== 1 ||
        typeof item[0] !== "string" ||
        !isAppIdentity(item[0], bundleId)
      ) {
        throw new SigningError("Shared or custom keychain groups are not supported yet");
      }
    } else if (
      typeof item !== "string" ||
      (key === "application-identifier" && !isAppIdentity(item, bundleId))
    ) {
      throw new SigningError("Unsupported app identity entitlement");
    }
  }
}

function isAppIdentity(value: string, bundleId: string) {
  return new RegExp("^[A-Za-z0-9]+\\." + bundleId.replaceAll(".", "\\.") + "$").test(value);
}

export function validateInspectedEntitlements(entitlements: string[], bundleId: string) {
  for (const xml of entitlements) {
    try {
      validateBasicEntitlements(parsePlist(xml), bundleId);
    } catch (error) {
      if (error instanceof SigningError) {
        throw error;
      }
      throw new SigningError("The app has malformed or unsupported entitlements");
    }
  }
}
