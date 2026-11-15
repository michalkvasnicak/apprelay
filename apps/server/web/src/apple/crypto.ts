import { utf8 } from "./encoding";

/** Hash used by SRP, password stretching and transcript authentication. */
const sha256Algorithm = "SHA-256";
/** Keyed hash algorithm for protocol derivations. */
const hmacAlgorithm = "HMAC";
/** Password stretching algorithm negotiated by GrandSlam. */
const pbkdf2Algorithm = "PBKDF2";
/** Session encryption with WebCrypto PKCS#7 handling. */
const aesCbcAlgorithm = "AES-CBC";
/** Authenticated app-token envelope encryption. */
const aesGcmAlgorithm = "AES-GCM";
/** PBKDF2 password key width in bits. */
const derivedKeyBits = 256;
/** HMAC domain separator for session AES key derivation. */
const sessionKeyLabel = "extra data key:";
/** HMAC domain separator for session CBC IV derivation. */
const sessionIvLabel = "extra data iv:";
/** HMAC domain separator for the negotiated protocol proof. */
const negotiationKeyLabel = "HMAC key:";
/** App-token authenticated envelope prefix. */
const appTokenMagic = "XYZ";

/** AES session and app-token IV width in bytes. */
const ivBytes = 16;
/** Authenticated app-token prefix width in bytes. */
const appTokenPrefixBytes = 3;
/** GCM authentication tag width in bits. */
const gcmTagBits = 128;
/** Ciphertext begins after the prefix and IV. */
const appTokenCiphertextOffset = appTokenPrefixBytes + ivBytes;
/** Smallest envelope includes framing plus a complete GCM tag. */
const minimumAppTokenBytes = appTokenCiphertextOffset + gcmTagBits / 8;
/** Little-endian uint32 framing for negotiated transcript fields. */
const transcriptLengthBytes = 4;
/** Ordered protocol offer and delimiters authenticated in the transcript. */
const negotiationOffer = "s2k,s2k_fo||";
/** Separator between authenticated negotiation transcript fields. */
const negotiationSeparator = "|";

export function concat(...parts: readonly Uint8Array[]): Uint8Array {
  const output = new Uint8Array(parts.reduce((size, part) => size + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    output.set(part, offset);
    offset += part.length;
  }
  return output;
}
/** Own ArrayBuffer avoids Buffer/subarray offsets and SharedArrayBuffer inputs. */
const buffer = (bytes: Uint8Array): ArrayBuffer => new Uint8Array(bytes).buffer;
export async function sha256(...parts: readonly Uint8Array[]): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.digest(sha256Algorithm, buffer(concat(...parts))));
}
export async function hmac(key: Uint8Array, ...parts: readonly Uint8Array[]): Promise<Uint8Array> {
  const imported = await crypto.subtle.importKey(
    "raw",
    buffer(key),
    { name: hmacAlgorithm, hash: sha256Algorithm },
    false,
    ["sign"],
  );
  return new Uint8Array(
    await crypto.subtle.sign(hmacAlgorithm, imported, buffer(concat(...parts))),
  );
}
export function equal(left: Uint8Array, right: Uint8Array): boolean {
  if (left.length !== right.length) {
    return false;
  }
  let difference = 0;
  for (let i = 0; i < left.length; i++) {
    difference |= left[i] ^ right[i];
  }
  return difference === 0;
}
export function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}
export async function passwordKey(
  password: string,
  salt: Uint8Array,
  iterations: number,
  protocol: "s2k" | "s2k_fo",
): Promise<Uint8Array> {
  const digest = await sha256(utf8(password));
  const input = protocol === "s2k" ? digest : utf8(hex(digest));
  const imported = await crypto.subtle.importKey("raw", buffer(input), pbkdf2Algorithm, false, [
    "deriveBits",
  ]);
  return new Uint8Array(
    await crypto.subtle.deriveBits(
      { name: pbkdf2Algorithm, hash: sha256Algorithm, salt: buffer(salt), iterations },
      imported,
      derivedKeyBits,
    ),
  );
}
export async function decryptSession(key: Uint8Array, ciphertext: Uint8Array): Promise<Uint8Array> {
  const aesKey = await hmac(key, utf8(sessionKeyLabel));
  const iv = (await hmac(key, utf8(sessionIvLabel))).slice(0, ivBytes);
  const imported = await crypto.subtle.importKey("raw", buffer(aesKey), aesCbcAlgorithm, false, [
    "decrypt",
  ]);
  return new Uint8Array(
    await crypto.subtle.decrypt(
      { name: aesCbcAlgorithm, iv: buffer(iv) },
      imported,
      buffer(ciphertext),
    ),
  );
}
export async function decryptAppToken(key: Uint8Array, envelope: Uint8Array): Promise<Uint8Array> {
  if (
    envelope.length < minimumAppTokenBytes ||
    !equal(envelope.slice(0, appTokenPrefixBytes), utf8(appTokenMagic))
  ) {
    throw new Error();
  }
  const imported = await crypto.subtle.importKey("raw", buffer(key), aesGcmAlgorithm, false, [
    "decrypt",
  ]);
  return new Uint8Array(
    await crypto.subtle.decrypt(
      {
        name: aesGcmAlgorithm,
        iv: buffer(envelope.slice(appTokenPrefixBytes, appTokenCiphertextOffset)),
        additionalData: buffer(envelope.slice(0, appTokenPrefixBytes)),
        tagLength: gcmTagBits,
      },
      imported,
      buffer(envelope.slice(appTokenCiphertextOffset)),
    ),
  );
}

/** Apple's negotiation transcript uses little-endian uint32 data lengths. */
export function lengthPrefixed(value: Uint8Array): Uint8Array {
  const length = new Uint8Array(transcriptLengthBytes);
  new DataView(length.buffer).setUint32(0, value.length, true);
  return concat(length, value);
}
export async function verifyNegotiation(
  key: Uint8Array,
  protocol: string,
  spd: Uint8Array,
  sc: Uint8Array | undefined,
  proof: Uint8Array,
): Promise<void> {
  const transcript = concat(
    utf8(`${negotiationOffer}${protocol}${negotiationSeparator}`),
    lengthPrefixed(spd),
    utf8(negotiationSeparator),
    sc ? lengthPrefixed(sc) : new Uint8Array(),
    utf8(negotiationSeparator),
  );
  const expected = await hmac(await hmac(key, utf8(negotiationKeyLabel)), await sha256(transcript));
  if (!equal(expected, proof)) {
    throw new Error();
  }
}
