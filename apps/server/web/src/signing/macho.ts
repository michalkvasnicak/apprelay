import { SigningError, signingLimits } from "./types";

const thinMagic = 0xfeedfacf;
const thin32Magic = 0xfeedface;

// Inspect every architecture, not just the first slice. Unknown/fat64 formats
// fail closed; this initial signer supports little-endian iOS Mach-O only.
export function inspectMachO(bytes: Uint8Array): string[] | undefined {
  if (bytes.length < 4) {
    return undefined;
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const magic = view.getUint32(0, true);
  if (magic === thinMagic || magic === thin32Magic) {
    return inspectSlice(bytes);
  }
  const fat = view.getUint32(0, false);
  if (fat !== 0xcafebabe) {
    if ([0xcafebabf, 0xbebafeca, 0xbfbafeca, 0xfeedface, 0xfeedfacf].includes(fat)) {
      throw new SigningError("Unsupported Mach-O architecture format");
    }
    return undefined;
  }
  if (bytes.length < 8) {
    throw new SigningError("Invalid universal executable");
  }
  const count = view.getUint32(4, false);
  if (count < 1 || count > 16 || 8 + count * 20 > bytes.length) {
    throw new SigningError("Invalid universal executable");
  }
  const entitlements: string[] = [];
  let previousEnd = 8 + count * 20;
  for (let index = 0; index < count; index++) {
    const offset = view.getUint32(8 + index * 20 + 8, false);
    const size = view.getUint32(8 + index * 20 + 12, false);
    if (offset < previousEnd || size < 28 || offset + size > bytes.length) {
      throw new SigningError("Invalid universal executable slices");
    }
    entitlements.push(...inspectSlice(bytes.subarray(offset, offset + size)));
    previousEnd = offset + size;
  }
  return entitlements;
}

function inspectSlice(bytes: Uint8Array): string[] {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const magic = view.getUint32(0, true);
  if (magic !== thinMagic && magic !== thin32Magic) {
    throw new SigningError("Unsupported executable slice");
  }
  const header = magic === thinMagic ? 32 : 28;
  if (bytes.length < header) {
    throw new SigningError("Invalid executable header");
  }
  if (view.getUint32(12, true) !== 2) {
    throw new SigningError("Only application executables are supported");
  }
  const count = view.getUint32(16, true);
  const end = header + view.getUint32(20, true);
  if (end > bytes.length || count > 10_000) {
    throw new SigningError("Invalid executable commands");
  }
  let offset = header;
  const entitlements: string[] = [];
  for (let index = 0; index < count; index++) {
    if (offset + 8 > end) {
      throw new SigningError("Invalid executable command");
    }
    const command = view.getUint32(offset, true);
    const length = view.getUint32(offset + 4, true);
    if (length < 8 || offset + length > end) {
      throw new SigningError("Invalid executable command length");
    }
    if (command === 0x21 || command === 0x2c) {
      if (length < 20 || view.getUint32(offset + 16, true) !== 0) {
        throw new SigningError("FairPlay-encrypted IPAs cannot be re-signed");
      }
    }
    if (command === 0x1d) {
      if (length < 16) {
        throw new SigningError("Invalid code signature command");
      }
      const start = view.getUint32(offset + 8, true);
      const size = view.getUint32(offset + 12, true);
      if (start < end || size < 12 || start + size > bytes.length) {
        throw new SigningError("Invalid embedded code signature");
      }
      entitlements.push(...signatureEntitlements(bytes.subarray(start, start + size)));
    }
    offset += length;
  }
  if (offset !== end) {
    throw new SigningError("Invalid executable command table");
  }
  return entitlements;
}

function signatureEntitlements(bytes: Uint8Array): string[] {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const size = view.getUint32(4, false);
  const count = view.getUint32(8, false);
  if (view.getUint32(0, false) !== 0xfade0cc0 || size > bytes.length || 12 + count * 8 > size) {
    throw new SigningError("Unsupported embedded code signature");
  }
  const result: string[] = [];
  let hasDer = false;
  for (let index = 0; index < count; index++) {
    const slot = view.getUint32(12 + index * 8, false);
    const offset = view.getUint32(16 + index * 8, false);
    if (offset < 12 + count * 8 || offset + 8 > size) {
      throw new SigningError("Invalid signature blob");
    }
    const length = view.getUint32(offset + 4, false);
    if (length < 8 || offset + length > size) {
      throw new SigningError("Invalid signature blob length");
    }
    if (slot === 7) {
      hasDer = true;
    }
    if (slot === 5) {
      if (view.getUint32(offset, false) !== 0xfade7171 || length > signingLimits.plistBytes) {
        throw new SigningError("Unsupported executable entitlements");
      }
      result.push(
        new TextDecoder("utf-8", { fatal: true }).decode(
          bytes.subarray(offset + 8, offset + length),
        ),
      );
    }
  }
  if (hasDer && !result.length) {
    throw new SigningError("DER-only entitlements are not supported yet");
  }
  return result;
}
