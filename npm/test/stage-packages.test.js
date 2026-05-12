import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { PLATFORM_CONFIGS } from "../lib/platform.js";

const __filename = fileURLToPath(import.meta.url);
const packageRoot = path.resolve(path.dirname(__filename), "..");
const repoRoot = path.resolve(packageRoot, "..");
const stageScript = path.join(packageRoot, "scripts", "stage-npm-packages.mjs");

function writeFile(filePath, body = "") {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, body);
}

function createArchive(releaseAssetsDir, config) {
  const rootName = `akra-${config.targetTriple}`;
  const bundleRoot = path.join(releaseAssetsDir, rootName);
  fs.mkdirSync(bundleRoot, { recursive: true });
  writeFile(path.join(bundleRoot, config.binaryName), "binary");
  writeFile(
    path.join(
      bundleRoot,
      "assets",
      "app-server",
      "skills",
      "akra-planning-queue-mutation",
      "SKILL.md",
    ),
    "# skill\n",
  );
  writeFile(
    path.join(bundleRoot, "assets", "admin", "game", "node_modules", "junk.js"),
    "module.exports = true;\n",
  );
  writeFile(
    path.join(bundleRoot, "assets", "admin", "game", "dist", "akra-diorama.js"),
    "console.log('built asset');\n",
  );
  writeFile(path.join(bundleRoot, "scripts", "gh-akra.sh"), "#!/usr/bin/env bash\n");

  const archivePath = path.join(
    releaseAssetsDir,
    `codex-exec-loop-native-0.1.0-${config.targetTriple}.tar.gz`,
  );
  execFileSync("tar", ["-czf", archivePath, "-C", releaseAssetsDir, rootName]);
  fs.rmSync(bundleRoot, { recursive: true, force: true });
}

function collectRelativeFiles(rootDir) {
  const files = [];
  const visit = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const entryPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        visit(entryPath);
      } else {
        files.push(path.relative(rootDir, entryPath));
      }
    }
  };
  visit(rootDir);
  return files.sort();
}

test("stage-npm-packages copies only runtime assets into platform vendors", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "akra-stage-npm-"));
  const releaseAssetsDir = path.join(tmp, "release-assets");
  const outDir = path.join(tmp, "publish");
  fs.mkdirSync(releaseAssetsDir, { recursive: true });

  try {
    for (const config of PLATFORM_CONFIGS) {
      createArchive(releaseAssetsDir, config);
    }

    execFileSync(
      process.execPath,
      [
        stageScript,
        "--release-assets-dir",
        releaseAssetsDir,
        "--out-dir",
        outDir,
        "--version",
        "0.1.0",
      ],
      { cwd: repoRoot },
    );

    for (const config of PLATFORM_CONFIGS) {
      const vendorRoot = path.join(
        outDir,
        "platforms",
        config.packageAlias,
        "vendor",
        config.targetTriple,
        "akra",
      );
      const files = collectRelativeFiles(vendorRoot);

      assert(files.includes(config.binaryName));
      assert(files.includes("scripts/gh-akra.sh"));
      assert(
        files.includes(
          "assets/app-server/skills/akra-planning-queue-mutation/SKILL.md",
        ),
      );
      assert(!files.some((file) => file.includes("node_modules")));
      assert(!files.some((file) => file.startsWith("assets/admin/")));
    }
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});
