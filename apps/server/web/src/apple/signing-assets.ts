import forge from "node-forge";

import { base64, binary, unbase64 } from "./encoding";
import { decodePlist } from "./plist";
import type { SigningIdentity } from "./types";
import { array, bytes, record, string } from "./validation";

export async function generateIdentity(teamId: string): Promise<SigningIdentity> {
  // Native browser crypto generates the key asynchronously. Only a public CSR
  // leaves the browser; forge handles PEM/ASN.1 and CSR serialization locally.
  const pair = await crypto.subtle.generateKey(
    {
      name: "RSASSA-PKCS1-v1_5",
      modulusLength: 2048,
      publicExponent: new Uint8Array([1, 0, 1]),
      hash: "SHA-256",
    },
    true,
    ["sign", "verify"],
  );
  const encoded = base64(new Uint8Array(await crypto.subtle.exportKey("pkcs8", pair.privateKey)));
  const privateKeyPem = `-----BEGIN PRIVATE KEY-----\n${encoded.match(/.{1,64}/g)?.join("\n")}\n-----END PRIVATE KEY-----`;
  const key = forge.pki.privateKeyFromPem(privateKeyPem);
  const csr = forge.pki.createCertificationRequest();
  csr.publicKey = forge.pki.rsa.setPublicKey(key.n, key.e);
  csr.setSubject([{ name: "commonName", value: "AppRelay browser signing" }]);
  csr.sign(key, forge.md.sha256.create());
  return {
    teamId,
    machineId: crypto.randomUUID().toUpperCase(),
    privateKeyPem,
    csrPem: forge.pki.certificationRequestToPem(csr),
    submitted: false,
  };
}

export function matchingCertificate(content: string, identity: SigningIdentity): boolean {
  const cert = forge.pki.certificateFromAsn1(forge.asn1.fromDer(binary(unbase64(content))));
  const key = forge.pki.privateKeyFromPem(identity.privateKeyPem);
  const publicKey = cert.publicKey;
  return (
    "n" in publicKey &&
    "e" in publicKey &&
    publicKey.n.compareTo(key.n) === 0 &&
    publicKey.e.compareTo(key.e) === 0 &&
    cert.subject.getField("OU")?.value === identity.teamId &&
    cert.validity.notBefore.getTime() <= Date.now() &&
    cert.validity.notAfter.getTime() > Date.now()
  );
}

export function certificateExpiry(content: string): number {
  return forge.pki
    .certificateFromAsn1(forge.asn1.fromDer(binary(unbase64(content))))
    .validity.notAfter.getTime();
}

function children(node: forge.asn1.Asn1): forge.asn1.Asn1[] {
  if (!node || !Array.isArray(node.value)) {
    throw new Error("Invalid profile CMS structure");
  }
  return node.value;
}

function octets(node: forge.asn1.Asn1): string {
  if (!node || node.type !== forge.asn1.Type.OCTETSTRING) {
    throw new Error("Invalid profile CMS content");
  }
  return typeof node.value === "string" ? node.value : children(node).map(octets).join("");
}

// Extraction only; this does not authenticate the CMS signature.
export function profileDocument(data: Uint8Array): Record<string, unknown> {
  const cms = children(forge.asn1.fromDer(binary(data)));
  if (
    typeof cms[0].value !== "string" ||
    forge.asn1.derToOid(cms[0].value) !== "1.2.840.113549.1.7.2"
  ) {
    throw new Error("Expected CMS SignedData");
  }
  const signed = children(children(cms[1])[0]);
  const content = children(signed[2]);
  if (
    typeof content[0].value !== "string" ||
    forge.asn1.derToOid(content[0].value) !== "1.2.840.113549.1.7.1"
  ) {
    throw new Error("Expected CMS data");
  }
  return decodePlist(
    Uint8Array.from(octets(children(content[1])[0]), (character) => character.charCodeAt(0)),
  );
}

// Structural/binding checks only; this is NOT CMS signature or chain validation.
export function validateProfile(
  data: Uint8Array,
  identity: SigningIdentity,
  udid: string,
  bundleId: string,
): number {
  const profile = profileDocument(data);
  const entitlements = record(profile.Entitlements);
  const expires = profile.ExpirationDate;
  const created = profile.CreationDate;
  if (
    !(expires instanceof Date) ||
    !(created instanceof Date) ||
    expires.getTime() <= Date.now() ||
    created.getTime() > Date.now() ||
    !array(profile.TeamIdentifier).map(string).includes(identity.teamId) ||
    !array(profile.ProvisionedDevices)
      .map(string)
      .some((device) => device.toUpperCase() === udid.toUpperCase()) ||
    !array(profile.DeveloperCertificates).some(
      (value) => base64(bytes(value)) === identity.certificate,
    ) ||
    !array(profile.ApplicationIdentifierPrefix)
      .map(string)
      .some((prefix) => entitlements["application-identifier"] === `${prefix}.${bundleId}`) ||
    entitlements["com.apple.developer.team-identifier"] !== identity.teamId ||
    entitlements["get-task-allow"] !== true
  ) {
    throw new Error("Profile does not match the requested app, device, team, or certificate");
  }
  return expires.getTime();
}
