import { base64, unbase64, utf8, text } from "./encoding";
import { record } from "./validation";

/** Maximum XML input size accepted by the strict parser. */
const maximumPlistCharacters = 2_000_000;
/** Maximum nesting depth for decoded plist nodes. */
const maximumPlistDepth = 64;
/** XML 1.0 controls other than tab, line feed and carriage return. */
// eslint-disable-next-line no-control-regex
const forbiddenXmlControls = /[\u0000-\u0008\u000b\u000c\u000e-\u001f]/;
/** Reject custom entity declarations and internal DTD subsets. */
const entityDeclarations = /<!ENTITY|\[\s*<!/i;
/** Signed decimal integer grammar used by plist integers. */
const integerPattern = /^-?\d+$/;
/** XML declaration, Apple public DTD and root opening frame. */
const plistHeader =
  '<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0">';
/** Closing XML plist frame. */
const plistFooter = "</plist>";
/** DOM parser mode for strict XML plist input. */
const xmlMediaType = "application/xml";

const invalid = () => new Error("Invalid plist");
const reject = (_reason: string): never => {
  throw invalid();
};

export type Plist =
  | string
  | number
  | boolean
  | Date
  | Uint8Array
  | readonly Plist[]
  | { readonly [key: string]: Plist };

function escape(value: string): string {
  // Reject control characters forbidden in XML 1.0, allowing tab, LF, and CR.
  if (forbiddenXmlControls.test(value)) {
    throw invalid();
  }
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}
function encode(value: Plist): string {
  if (typeof value === "string") {
    return `<string>${escape(value)}</string>`;
  }
  if (typeof value === "boolean") {
    return value ? "<true/>" : "<false/>";
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      throw invalid();
    }
    return Number.isInteger(value) ? `<integer>${value}</integer>` : `<real>${value}</real>`;
  }
  if (value instanceof Date) {
    return `<date>${value.toISOString()}</date>`;
  }
  if (value instanceof Uint8Array) {
    return `<data>${base64(value)}</data>`;
  }
  if (Array.isArray(value)) {
    return `<array>${value.map(encode).join("")}</array>`;
  }
  return `<dict>${Object.entries(value)
    .map(([key, item]) => `<key>${escape(key)}</key>${encode(item)}`)
    .join("")}</dict>`;
}
export function encodePlist(value: Plist): Uint8Array {
  return utf8(`${plistHeader}${encode(value)}${plistFooter}`);
}

export function parsePlist(input: string, allowBareDictionary = false): Record<string, unknown> {
  if (input.length > maximumPlistCharacters) {
    reject("size");
  }
  if (entityDeclarations.test(input)) {
    reject("entities");
  }
  const document = new DOMParser().parseFromString(input, xmlMediaType);
  const root = document.documentElement;
  if (document.querySelector("parsererror")) {
    reject("xml-syntax");
  }
  let valueRoot: Element;
  if (allowBareDictionary && root?.tagName === "dict") {
    valueRoot = root;
  } else {
    if (!root || root.tagName !== "plist") {
      reject(root?.tagName === "dict" ? "root-dict-without-plist" : "root-not-plist");
    }
    if (root.children.length !== 1) {
      reject("root-child-count");
    }
    valueRoot = root.children[0];
  }
  function read(node: Element, depth: number): unknown {
    if (depth > maximumPlistDepth) {
      reject("depth");
    }
    const children = Array.from(node.children);
    if (node.tagName === "dict") {
      if (children.length % 2) {
        reject("dict-pairs");
      }
      const result: Record<string, unknown> = Object.create(null);
      for (let i = 0; i < children.length; i += 2) {
        const key = children[i];
        if (key.tagName !== "key") {
          reject("dict-key");
        }
        if (key.children.length) {
          reject("dict-key-children");
        }
        if (Object.hasOwn(result, key.textContent ?? "")) {
          reject("dict-duplicate-key");
        }
        result[key.textContent ?? ""] = read(children[i + 1], depth + 1);
      }
      return result;
    }
    if (node.tagName === "array") {
      return children.map((child) => read(child, depth + 1));
    }
    if (children.length) {
      reject("scalar-children");
    }
    const value = node.textContent ?? "";
    switch (node.tagName) {
      case "string":
        return value;
      case "data":
        try {
          return unbase64(value);
        } catch {
          return reject("data-base64");
        }
      case "true":
        if (value.trim()) {
          reject("boolean-text");
        }
        return true;
      case "false":
        if (value.trim()) {
          reject("boolean-text");
        }
        return false;
      case "integer": {
        if (!integerPattern.test(value.trim())) {
          reject("integer-syntax");
        }
        const number = Number(value);
        if (!Number.isSafeInteger(number)) {
          reject("integer-range");
        }
        return number;
      }
      case "real": {
        if (!value.trim() || !Number.isFinite(Number(value))) {
          reject("real-value");
        }
        return Number(value);
      }
      case "date": {
        const date = new Date(value);
        if (!Number.isFinite(date.getTime())) {
          reject("date-value");
        }
        return date;
      }
      default:
        return reject("unsupported-tag");
    }
  }
  const value = read(valueRoot, 0);
  return record(value);
}

export function decodePlist(body: Uint8Array, allowBareDictionary = false) {
  return parsePlist(text(body), allowBareDictionary);
}
