#!/usr/bin/env node
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { renderKernelProtocol } from "./kernel-schema.mts";

const root = new URL("../", import.meta.url);
const spec = JSON.parse(readFileSync(new URL("../schema/akra-kernel-spec.json", root), "utf8"));
const directory = new URL("src/shared/kernel-protocol/generated/", root);
const target = new URL("index.ts", directory);
const generated = renderKernelProtocol(spec);
if (process.argv.includes("--check")) {
  if (readFileSync(target, "utf8").replaceAll("\r\n", "\n") !== generated) {
    throw new Error("Kernel protocol types are stale. Run npm run generate:kernel-protocol.");
  }
} else {
  mkdirSync(directory, { recursive: true });
  writeFileSync(target, generated);
}
console.log(`Kernel protocol ${process.argv.includes("--check") ? "checked" : "generated"}: ${fileURLToPath(target)}`);
