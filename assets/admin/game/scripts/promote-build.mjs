import { copyFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

copyFileSync(
  resolve(packageRoot, "dist/akra-diorama.js"),
  resolve(packageRoot, "akra-diorama.js")
);
