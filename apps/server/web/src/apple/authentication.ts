import { Anisette } from "./anisette";
import { decryptAppToken, decryptSession, hmac, verifyNegotiation } from "./crypto";
import { text, utf8 } from "./encoding";
import { clientParameters, grandSlamHeaders, twoFactorHeaders, xcodeAppId } from "./headers";
import { decodePlist, encodePlist } from "./plist";
import { startSrp } from "./srp";
import { relay, requireSuccess } from "./transport";
import type { AnisetteData, Session } from "./types";
import { array, bytes, integer, record, string } from "./validation";

interface Challenge {
  dsid: string;
  token: string;
  anisette: AnisetteData;
}

export type SignInResult =
  | { type: "authenticated"; session: Session }
  | { type: "two-factor"; challenge: Challenge; resume: () => Promise<SignInResult> };

class AppleRejection extends Error {
  constructor(readonly code: number) {
    super("Apple rejected authentication");
  }
}

async function request(
  anisette: AnisetteData,
  operation: string,
  parameters: Record<string, import("./plist").Plist>,
) {
  const response = await relay(
    "https://gsa.apple.com/grandslam/GsService2",
    "POST",
    grandSlamHeaders(anisette),
    encodePlist({
      Header: { Version: "1.0.1" },
      Request: { ...parameters, o: operation, cpd: clientParameters(anisette) },
    }),
  );
  requireSuccess(response.status);
  const result = record(decodePlist(response.body).Response);
  const status = record(result.Status);
  if (status.hsc === 433 || status.hsc === 434) {
    throw new Error("Apple requires anisette reprovisioning or unsupported resynchronization");
  }
  const code = integer(status.ec);
  if (code !== 0) {
    throw new AppleRejection(code);
  }
  return result;
}

export async function signIn(
  anisette: Anisette,
  username: string,
  password: string,
  verified = false,
): Promise<SignInResult> {
  if (!username.trim() || !password) {
    throw new Error("Enter an Apple account and password");
  }
  // Fresh identity data per exchange, but one unchanged snapshot across all
  // messages within that exchange. Never retry credentials or proofs blindly.
  const data = await anisette.fresh();
  const srp = startSrp();
  const initialized = await request(data, "init", {
    u: username,
    A2k: srp.publicValue,
    ps: ["s2k", "s2k_fo"],
  });
  const protocol = string(initialized.sp);
  const iterations = integer(initialized.i);
  const salt = bytes(initialized.s);
  if (
    (protocol !== "s2k" && protocol !== "s2k_fo") ||
    iterations < 1 ||
    iterations > 10_000_000 ||
    salt.length > 1024
  ) {
    throw new Error("Unsupported Apple password protocol");
  }
  const proof = await srp.challenge({
    username,
    password,
    protocol,
    iterations,
    salt,
    serverPublicValue: bytes(initialized.B),
  });
  const completed = await request(data, "complete", {
    u: username,
    c: string(initialized.c),
    M1: proof.clientProof,
  });
  const encrypted = bytes(completed.spd);
  proof.verify(bytes(completed.M2));
  await verifyNegotiation(
    proof.key,
    protocol,
    encrypted,
    completed.sc === undefined ? undefined : bytes(completed.sc),
    bytes(completed.np),
  );
  const account = decodePlist(await decryptSession(proof.key, encrypted), true);
  const dsid = string(account.adsid);
  const token = string(account.GsIdmsToken);
  const challenge = () => {
    if (verified) {
      throw new Error("Apple requested verification again; start a new sign-in");
    }
    return {
      type: "two-factor" as const,
      challenge: { dsid, token, anisette: data },
      resume: () => signIn(anisette, username, password, true),
    };
  };
  const status = record(completed.Status);
  const mode = status.au ?? completed.au;
  if (
    mode &&
    mode !== "secondaryAuth" &&
    mode !== "trustedDeviceSecondaryAuth" &&
    mode !== "trustedDevice"
  ) {
    throw new Error("This Apple verification method is not supported");
  }
  if (
    status.hsc === 409 ||
    mode === "secondaryAuth" ||
    mode === "trustedDeviceSecondaryAuth" ||
    mode === "trustedDevice"
  ) {
    return challenge();
  }
  const key = bytes(account.sk);
  const checksum = await hmac(key, utf8("apptokens"), utf8(dsid), utf8(xcodeAppId));
  let tokens: Record<string, unknown>;
  try {
    tokens = await request(data, "apptokens", {
      u: dsid,
      t: token,
      c: bytes(account.c),
      app: [xcodeAppId],
      checksum,
    });
  } catch (error) {
    if (error instanceof AppleRejection && error.code === -22421 && !verified) {
      return challenge();
    }
    throw error;
  }
  const decoded = decodePlist(await decryptAppToken(key, bytes(tokens.et)), true);
  const appToken = record(record(decoded.t)[xcodeAppId]);
  const expiry = appToken.expiry === undefined ? undefined : integer(appToken.expiry);
  if (expiry !== undefined && expiry <= Date.now()) {
    throw new Error("Apple returned an expired session");
  }
  return {
    type: "authenticated",
    session: { dsid, authToken: string(appToken.token), expiresAt: expiry },
  };
}

export async function trustedDevice(challenge: Challenge, code?: string): Promise<boolean> {
  if (code !== undefined && !/^\d{6}$/.test(code)) {
    throw new Error("Enter a six-digit verification code");
  }
  const response = await relay(
    `https://gsa.apple.com${code === undefined ? "/auth/verify/trusteddevice" : "/grandslam/GsService2/validate"}`,
    "GET",
    {
      ...twoFactorHeaders(challenge.anisette, challenge.dsid, challenge.token),
      ...(code === undefined ? {} : { "security-code": code }),
    },
  );
  if ([429, 433, 434].includes(response.status) || response.status >= 500) {
    throw new Error("Apple verification is unavailable; start a new sign-in");
  }
  const input = text(response.body).trim();
  if (input.startsWith("{") || input.includes("<plist")) {
    const payload = input.startsWith("{") ? record(JSON.parse(input)) : decodePlist(response.body);
    const status = payload.Status === undefined ? {} : record(payload.Status);
    if (status.hsc === 433 || status.hsc === 434) {
      throw new Error("Apple requires a new anisette identity");
    }
    const value = payload.ec ?? status.ec;
    const errorCode = value === undefined ? 0 : integer(value);
    if (errorCode === -21669 && code !== undefined) {
      return false;
    }
    if (
      errorCode !== 0 ||
      (payload.errors !== undefined && array(payload.errors).length) ||
      (payload.serviceErrors !== undefined && array(payload.serviceErrors).length)
    ) {
      throw new Error("Apple rejected verification; start a new sign-in");
    }
  } else if (input.startsWith("<")) {
    if (/<!ENTITY|\[\s*<!/i.test(input)) {
      throw new Error("Invalid Apple verification response");
    }
    const document = new DOMParser().parseFromString(input, "application/xml");
    if (
      document.querySelector("parsererror") ||
      (!document.querySelector("pinView") &&
        Array.from(document.querySelectorAll("alert")).some(
          (alert) =>
            !alert.hasAttribute("id") &&
            (alert.hasAttribute("title") || alert.hasAttribute("message")),
        ))
    ) {
      throw new Error("Apple rejected verification; start a new sign-in");
    }
  } else if (input) {
    throw new Error("Unrecognized Apple verification response");
  }
  return response.status === 200;
}
