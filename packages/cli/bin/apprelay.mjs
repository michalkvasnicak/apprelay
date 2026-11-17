#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { registerHooks, stripTypeScriptTypes } from "node:module";

// Native TypeScript loading excludes node_modules, including npx installations.
// Strip only our own source, leaving dependency loading untouched.
const sourceRoot = new URL("../src/", import.meta.url).href;
registerHooks({
  load(url, context, nextLoad) {
    if (url.startsWith(sourceRoot) && url.endsWith(".ts")) {
      return {
        format: "module",
        source: stripTypeScriptTypes(readFileSync(new URL(url), "utf8")),
        shortCircuit: true,
      };
    }
    return nextLoad(url, context);
  },
});

await import("../src/cli.ts");
