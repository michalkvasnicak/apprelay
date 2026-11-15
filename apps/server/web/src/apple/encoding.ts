export const utf8 = (value: string) => new TextEncoder().encode(value);
export const text = (value: Uint8Array) => new TextDecoder("utf-8", { fatal: true }).decode(value);
export const binary = (value: Uint8Array) =>
  Array.from(value, (byte) => String.fromCharCode(byte)).join("");
export const base64 = (value: Uint8Array) => btoa(binary(value));

export function unbase64(value: string): Uint8Array {
  const normalized = value.replace(/\s/g, "");
  if (
    normalized.length > 2_800_000 ||
    !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(normalized)
  ) {
    throw new Error("Invalid base64");
  }
  return Uint8Array.from(atob(normalized), (character) => character.charCodeAt(0));
}
