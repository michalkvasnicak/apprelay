import { requireProvisionableDevice } from "../device";
import { DeveloperPortal } from "./developer";
import { base64, unbase64 } from "./encoding";
import { generateIdentity, matchingCertificate, validateProfile } from "./signing-assets";
import type { StoredAccount } from "./types";
import { record, string } from "./validation";

export async function ensureCertificate(
  account: StoredAccount,
  portal: DeveloperPortal,
  save: () => Promise<void>,
  afterRevocation?: { remaining: Awaited<ReturnType<DeveloperPortal["certificates"]>> },
) {
  if (!account.teamId) {
    throw new Error("Choose a development team first");
  }
  if (account.identity && account.identity.teamId !== account.teamId) {
    throw new Error("The saved signing key belongs to a different team");
  }
  if (!account.identity) {
    account.identity = await generateIdentity(account.teamId);
    await save(); // Persist the private key BEFORE any certificate mutation.
  }
  const identity = account.identity;
  const find = (certificates: Awaited<ReturnType<DeveloperPortal["certificates"]>>) =>
    certificates.find((cert) => matchingCertificate(cert.content, identity))?.content;
  let certificate = find(
    afterRevocation?.remaining ?? (await portal.certificates(identity.teamId)),
  );
  if (!certificate) {
    // Only the explicit replacement action can authorize another request
    // after revocation has been verified and the saved key reconciled.
    if (identity.submitted && !afterRevocation) {
      throw new Error(
        "The certificate request is pending or its outcome is unknown; recheck later. No new request was sent",
      );
    }
    identity.submitted = true;
    await save(); // A crash/timeout must never trigger blind CSR replay.
    await portal.request(
      "ios/submitDevelopmentCSR",
      {
        csrContent: identity.csrPem,
        machineId: identity.machineId,
        machineName: "AppRelay browser",
      },
      identity.teamId,
    );
    certificate = find(await portal.certificates(identity.teamId));
  }
  if (!certificate) {
    throw new Error(
      "Certificate is not visible yet; recheck later. The signing key has been preserved",
    );
  }
  if (identity.certificate !== certificate) {
    account.profile = undefined;
  }
  identity.certificate = certificate;
  await save();
}

export async function provisionApp(
  account: StoredAccount,
  portal: DeveloperPortal,
  bundleId: string,
  save: () => Promise<void>,
) {
  const device = requireProvisionableDevice();
  const identity = account.identity;
  if (
    !identity?.certificate ||
    identity.teamId !== account.teamId ||
    !matchingCertificate(identity.certificate, identity)
  ) {
    throw new Error("A valid matching development certificate is required");
  }
  if (!/^[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+$/.test(bundleId) || bundleId.length > 255) {
    throw new Error("Enter an explicit bundle identifier, such as com.example.app");
  }
  const devices = (await portal.list("ios/listDevices", "devices", identity.teamId)).map(
    (entry) => ({
      udid: string(entry.deviceNumber),
      status: string(entry.status),
    }),
  );
  const registered = devices.find(
    (entry) => entry.udid.toUpperCase() === device.udid.toUpperCase(),
  );
  if (registered && registered.status !== "c") {
    throw new Error("The target device is disabled in the Apple developer account");
  }
  if (!registered) {
    const receipt = record(
      (
        await portal.request(
          "ios/addDevice",
          { name: device.name, deviceNumber: device.udid },
          identity.teamId,
        )
      ).device,
    );
    if (
      string(receipt.deviceNumber).toUpperCase() !== device.udid.toUpperCase() ||
      receipt.status !== "c"
    ) {
      throw new Error("Device registration was not confirmed");
    }
  }
  const apps = (await portal.list("ios/listAppIds", "appIds", identity.teamId)).map((entry) => ({
    id: string(entry.appIdId),
    bundleId: string(entry.identifier),
  }));
  let app = apps.find((entry) => entry.bundleId === bundleId);
  if (!app) {
    const receipt = record(
      (
        await portal.request(
          "ios/addAppId",
          { name: "AppRelay App", identifier: bundleId },
          identity.teamId,
        )
      ).appId,
    );
    app = { id: string(receipt.appIdId), bundleId: string(receipt.identifier) };
    if (app.bundleId !== bundleId) {
      throw new Error("Apple returned a different app identifier");
    }
  }
  // This endpoint can create/regenerate a profile. It is an explicit mutation,
  // never a query or an automatic retry. Rechecks reconcile device/app lists.
  const payload = await portal.request(
    "ios/downloadTeamProvisioningProfile",
    { appIdId: app.id },
    identity.teamId,
  );
  const profile = record(payload.provisioningProfile);
  const encoded = profile.encodedProfile;
  const data = encoded instanceof Uint8Array ? encoded : unbase64(string(encoded));
  const expiresAt = validateProfile(data, identity, device.udid, bundleId);
  account.profile = { bundleId, udid: device.udid, data: base64(data), expiresAt };
  await save();
}
