import forge from "node-forge";
const randomBytes = (size: number) => crypto.getRandomValues(new Uint8Array(size));
import { equal, hex, passwordKey, sha256 } from "./crypto";
import { utf8 } from "./encoding";

/** RFC 5054 2048-bit group element width in bytes. */
const groupBytes = 256;
/** Padded hexadecimal width of a group element. */
const groupHexDigits = 512;
/** GrandSlam omits the username before this password-hash separator. */
const passwordSeparator = ":";
/** Fresh 256-bit ephemeral SRP secret width. */
const ephemeralSecretBytes = 32;

/** RFC 5054 Appendix A 2048-bit modulus; arithmetic uses node-forge jsbn. */
const modulus = new forge.jsbn.BigInteger(
  "AC6BDB41324A9A9BF166DE5E1389582FAF72B6651987EE07FC3192943DB56050" +
    "A37329CBB4A099ED8193E0757767A13DD52312AB4B03310DCD7F48A9DA04FD50" +
    "E8083969EDB767B0CF6095179A163AB3661A05FBD5FAAAE82918A9962F0B93B8" +
    "55F97993EC975EEAA80D740ADBF4FF747359D041D5C33EA71D281E446B14773B" +
    "CA97B43A23FB801676BD207A436C6481F1D2B9078717461A5B9D32E688F87748" +
    "544523B524B0D57D5EA77A2775D2ECFA032CFBDBF52FB3786160279004E57AE6" +
    "AF874E7303CE53299CCC041C7BC308D82A5698F3A8D0C38271AE35F8E9DBFBB6" +
    "94B5C803D89F7AE435DE236D525F54759B65E372FCD68EF20FA7111F9E4AFF73",
  16,
);
/** RFC 5054 group generator. */
const generator = new forge.jsbn.BigInteger("2", 10);
const number = (bytes: Uint8Array) => new forge.jsbn.BigInteger(hex(bytes) || "0", 16);
function padded(value: forge.jsbn.BigInteger): Uint8Array {
  const digits = value.toString(16).padStart(groupHexDigits, "0");
  if (digits.length !== groupHexDigits || digits.startsWith("-")) {
    throw new Error();
  }
  return Uint8Array.from({ length: groupBytes }, (_, index) =>
    parseInt(digits.slice(index * 2, index * 2 + 2), 16),
  );
}

/** One ephemeral SRP exchange. Never reuse after any failed/verified exchange. */
export function startSrp() {
  let secret = number(randomBytes(ephemeralSecretBytes));
  while (secret.signum() === 0) {
    secret = number(randomBytes(ephemeralSecretBytes));
  }
  const publicValue = padded(generator.modPow(secret, modulus));
  return {
    publicValue,
    async challenge(input: {
      readonly username: string;
      readonly password: string;
      readonly salt: Uint8Array;
      readonly serverPublicValue: Uint8Array;
      readonly iterations: number;
      readonly protocol: "s2k" | "s2k_fo";
    }) {
      const server = number(input.serverPublicValue);
      if (
        input.serverPublicValue.length !== groupBytes ||
        server.signum() <= 0 ||
        server.compareTo(modulus) >= 0
      ) {
        throw new Error();
      }
      const stretched = await passwordKey(
        input.password,
        input.salt,
        input.iterations,
        input.protocol,
      );
      // GrandSlam omits username in x, but includes H(username) in M1.
      const x = number(await sha256(input.salt, await sha256(utf8(passwordSeparator), stretched)));
      const multiplier = number(await sha256(padded(modulus), padded(generator)));
      const scrambling = number(await sha256(publicValue, input.serverPublicValue));
      if (scrambling.signum() === 0) {
        throw new Error();
      }
      const base = server.subtract(multiplier.multiply(generator.modPow(x, modulus))).mod(modulus);
      if (base.signum() === 0) {
        throw new Error();
      }
      const shared = base.modPow(secret.add(scrambling.multiply(x)), modulus);
      const key = await sha256(padded(shared));
      const groupHash = await sha256(padded(modulus));
      const generatorHash = await sha256(padded(generator));
      const xor = groupHash.map((value, index) => value ^ generatorHash[index]);
      const clientProof = await sha256(
        xor,
        await sha256(utf8(input.username)),
        input.salt,
        publicValue,
        input.serverPublicValue,
        key,
      );
      const serverProof = await sha256(publicValue, clientProof, key);
      return {
        key,
        clientProof,
        verify(proof: Uint8Array) {
          if (!equal(serverProof, proof)) {
            throw new Error();
          }
        },
      };
    },
  };
}
