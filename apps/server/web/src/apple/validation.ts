export function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("Invalid protocol response");
  }
  return value as Record<string, unknown>;
}

export function string(value: unknown): string {
  if (typeof value !== "string" || !value || value.length > 2_000_000) {
    throw new Error("Invalid protocol string");
  }
  return value;
}

export function integer(value: unknown): number {
  const result = typeof value === "string" && /^-?\d+$/.test(value) ? Number(value) : value;
  if (typeof result !== "number" || !Number.isSafeInteger(result)) {
    throw new Error("Invalid protocol integer");
  }
  return result;
}

export function array(value: unknown): unknown[] {
  if (!Array.isArray(value)) {
    throw new Error("Invalid protocol list");
  }
  return value;
}

export function bytes(value: unknown): Uint8Array {
  if (!(value instanceof Uint8Array) || !value.length) {
    throw new Error("Invalid protocol bytes");
  }
  return value;
}
