import { base64, text, unbase64, utf8 } from "../apple/encoding";
import type { StoredAccount } from "../apple/types";
import { integer, record, string } from "../apple/validation";

const storageKey = "apprelay.apple-vault.v1";
const context = utf8("apprelay.apple-vault.v1");
const maximumSize = 512_000;

function decodeAccount(value: unknown): StoredAccount {
  const saved = record(value);
  const session = record(saved.session);
  const anisette = record(saved.anisette);
  const account: StoredAccount = {
    session: {
      dsid: string(session.dsid),
      authToken: string(session.authToken),
      expiresAt: session.expiresAt === undefined ? undefined : integer(session.expiresAt),
    },
    anisette: {
      identifier: string(anisette.identifier),
      adiPb: string(anisette.adiPb),
      userAgent: string(anisette.userAgent),
      routingInfo: anisette.routingInfo === undefined ? undefined : integer(anisette.routingInfo),
    },
    teamId: saved.teamId === undefined ? undefined : string(saved.teamId),
  };
  if (saved.identity !== undefined) {
    const identity = record(saved.identity);
    if (typeof identity.submitted !== "boolean") {
      throw new Error("Invalid signing identity");
    }
    account.identity = {
      teamId: string(identity.teamId),
      machineId: string(identity.machineId),
      csrPem: string(identity.csrPem),
      privateKeyPem: string(identity.privateKeyPem),
      submitted: identity.submitted,
      certificate: identity.certificate === undefined ? undefined : string(identity.certificate),
    };
  }
  if (saved.profile !== undefined) {
    const profile = record(saved.profile);
    account.profile = {
      bundleId: string(profile.bundleId),
      udid: string(profile.udid),
      data: string(profile.data),
      expiresAt: integer(profile.expiresAt),
    };
  }
  return account;
}

async function derive(passphrase: string, salt: Uint8Array) {
  const input = await crypto.subtle.importKey("raw", utf8(passphrase), "PBKDF2", false, [
    "deriveKey",
  ]);
  return crypto.subtle.deriveKey(
    { name: "PBKDF2", hash: "SHA-256", salt: new Uint8Array(salt), iterations: 310_000 },
    input,
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt", "decrypt"],
  );
}

// The encryption key exists only while unlocked. Neither the passphrase nor
// Apple passwords/codes are stored. Same-origin scripts can access unlocked data.
export class LocalVault {
  private key?: CryptoKey;
  private salt?: Uint8Array;

  exists() {
    return localStorage.getItem(storageKey) !== null;
  }

  get unlocked() {
    return this.key !== undefined;
  }

  async create(passphrase: string, account: StoredAccount) {
    if (this.exists() || passphrase.length < 12) {
      throw new Error("Unlock the existing vault or use a passphrase of at least 12 characters");
    }
    const salt = crypto.getRandomValues(new Uint8Array(16));
    const key = await derive(passphrase, salt);
    await this.write(key, salt, account);
    this.key = key;
    this.salt = salt;
  }

  async unlock(passphrase: string): Promise<StoredAccount> {
    const raw = localStorage.getItem(storageKey);
    if (!raw || raw.length > maximumSize) {
      throw new Error("Vault unavailable");
    }
    const envelope = record(JSON.parse(raw));
    if (envelope.version !== 1) {
      throw new Error("Unsupported vault version");
    }
    const salt = unbase64(string(envelope.salt));
    const iv = unbase64(string(envelope.iv));
    if (salt.length !== 16 || iv.length !== 12) {
      throw new Error("Invalid vault framing");
    }
    const key = await derive(passphrase, salt);
    const plaintext = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: new Uint8Array(iv), additionalData: context },
      key,
      new Uint8Array(unbase64(string(envelope.data))),
    );
    const account = decodeAccount(JSON.parse(text(new Uint8Array(plaintext))));
    this.key = key;
    this.salt = salt;
    return account;
  }

  async save(account: StoredAccount) {
    if (!this.key || !this.salt) {
      throw new Error("Unlock local storage first");
    }
    await this.write(this.key, this.salt, account);
  }

  private async write(key: CryptoKey, salt: Uint8Array, account: StoredAccount) {
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const plaintext = JSON.stringify(account);
    if (plaintext.length > maximumSize / 2) {
      throw new Error("Vault exceeds the storage limit");
    }
    const encrypted = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv, additionalData: context },
      key,
      utf8(plaintext),
    );
    localStorage.setItem(
      storageKey,
      JSON.stringify({
        version: 1,
        salt: base64(salt),
        iv: base64(iv),
        data: base64(new Uint8Array(encrypted)),
      }),
    );
  }

  lock() {
    this.key = undefined;
    this.salt = undefined;
  }

  forget() {
    localStorage.removeItem(storageKey);
    this.lock();
  }
}
