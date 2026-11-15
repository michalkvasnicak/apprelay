export const signingLimits = {
  inputBytes: 100 * 1024 * 1024,
  expandedBytes: 256 * 1024 * 1024,
  entryBytes: 100 * 1024 * 1024,
  entries: 10_000,
  plistBytes: 2_000_000,
} as const;

export class SigningError extends Error {}

export interface Inspection {
  bundleId: string;
  executable: string;
  entitlements: string[];
}

export interface SigningAssets {
  privateKeyPem: string;
  certificate: string;
  profile: Uint8Array;
  entitlementDocument: Uint8Array;
}

export type WorkerRequest =
  | { type: "inspect"; file: File }
  | { type: "sign"; assets: SigningAssets };

export type WorkerResponse =
  | { type: "progress"; message: string }
  | { type: "inspected"; inspection: Inspection }
  | { type: "signed"; bytes: ArrayBuffer }
  | { type: "error"; message: string };
