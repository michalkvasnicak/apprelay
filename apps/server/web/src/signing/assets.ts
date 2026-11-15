import { unbase64 } from "../apple/encoding";
import { encodePlist, type Plist } from "../apple/plist";
import { matchingCertificate, profileDocument, validateProfile } from "../apple/signing-assets";
import type { StoredAccount } from "../apple/types";
import { record } from "../apple/validation";
import { requireProvisionableDevice } from "../device";
import { validateBasicEntitlements } from "./entitlements";
import { SigningError, type SigningAssets } from "./types";

export function signingAssets(account: StoredAccount, bundleId: string): SigningAssets {
  const device = requireProvisionableDevice();
  const { identity, profile } = account;
  if (
    !identity?.certificate ||
    identity.teamId !== account.teamId ||
    !matchingCertificate(identity.certificate, identity)
  ) {
    throw new SigningError("Prepare a valid matching development certificate first");
  }
  if (
    !profile ||
    profile.bundleId !== bundleId ||
    profile.udid.toUpperCase() !== device.udid.toUpperCase()
  ) {
    throw new SigningError("Prepare a profile for this app and the supplied device first");
  }
  const data = unbase64(profile.data);
  validateProfile(data, identity, device.udid, bundleId);
  const entitlements = { ...record(profileDocument(data).Entitlements) };
  const applicationId = entitlements["application-identifier"];
  if (typeof applicationId !== "string") {
    throw new SigningError("The profile has no explicit application identity");
  }
  const prefix = applicationId.slice(0, -(bundleId.length + 1));
  const groups = entitlements["keychain-access-groups"];
  // Resolve only the standard profile wildcard to this app's default group.
  // Custom/shared groups and all unsupported capabilities still fail closed.
  if (Array.isArray(groups) && groups.length === 1 && groups[0] === prefix + ".*") {
    entitlements["keychain-access-groups"] = [applicationId];
  }
  validateBasicEntitlements(entitlements, bundleId);
  return {
    privateKeyPem: identity.privateKeyPem,
    certificate: identity.certificate,
    profile: data,
    // The published signer has no entitlement setter: its third argument only
    // extracts Entitlements from profile-shaped XML. Supply our validated,
    // resolved dictionary there; embed the ORIGINAL CMS profile in the IPA.
    entitlementDocument: encodePlist({ Entitlements: entitlements as Record<string, Plist> }),
  };
}
