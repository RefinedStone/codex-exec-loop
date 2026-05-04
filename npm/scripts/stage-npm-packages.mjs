#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { PLATFORM_CONFIGS } from "../lib/platform.js";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const packageRoot = path.resolve(__dirname, "..");
const publishedPackageName = "@refinedstone/akra";

function parseArgs(argv) {
  const args = {
    releaseAssetsDir: "release-assets",
    outDir: "npm/dist/publish",
    version: "",
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    switch (arg) {
      case "--release-assets-dir":
        args.releaseAssetsDir = argv[++index];
        break;
      case "--out-dir":
        args.outDir = argv[++index];
        break;
      case "--version":
        args.version = argv[++index];
        break;
      default:
        throw new Error(`Unsupported option: ${arg}`);
    }
  }

  if (!args.version) {
    throw new Error("--version is required");
  }

  return args;
}

function normalizeVersion(version) {
  return version.startsWith("v") ? version.slice(1) : version;
}

function ensureDir(dirPath) {
  fs.mkdirSync(dirPath, { recursive: true });
}

function copyDir(sourceDir, destinationDir) {
  ensureDir(destinationDir);
  for (const entry of fs.readdirSync(sourceDir, { withFileTypes: true })) {
    const sourcePath = path.join(sourceDir, entry.name);
    const destinationPath = path.join(destinationDir, entry.name);
    if (entry.isDirectory()) {
      copyDir(sourcePath, destinationPath);
      continue;
    }

    fs.copyFileSync(sourcePath, destinationPath);
  }
}

function findArchive(releaseAssetsDir, targetTriple) {
  const suffix = `${targetTriple}.tar.gz`;
  const archiveName = fs
    .readdirSync(releaseAssetsDir)
    .find(
      (entry) =>
        entry.endsWith(suffix) && !entry.endsWith(".tar.gz.sha256"),
    );

  if (!archiveName) {
    throw new Error(`Missing release archive for ${targetTriple}`);
  }

  return path.join(releaseAssetsDir, archiveName);
}

function extractArchive(archivePath, outputDir) {
  execFileSync("tar", ["-xzf", archivePath, "-C", outputDir]);

  const entries = fs
    .readdirSync(outputDir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory());
  if (entries.length !== 1) {
    throw new Error(
      `Expected exactly one root directory after extracting ${archivePath}`,
    );
  }

  return path.join(outputDir, entries[0].name);
}

function writeJson(filePath, value) {
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`);
}

function stagePlatformPackage({
  archivePath,
  outDir,
  packageVersion,
  config,
}) {
  const extractDir = fs.mkdtempSync(
    path.join(os.tmpdir(), `akra-${config.packageVersionSuffix}-`),
  );

  try {
    const bundleRoot = extractArchive(archivePath, extractDir);
    const sourceBinaryPath = path.join(bundleRoot, config.binaryName);
    if (!fs.existsSync(sourceBinaryPath)) {
      throw new Error(`Bundled binary is missing: ${sourceBinaryPath}`);
    }
    const sourceAssetsPath = path.join(bundleRoot, "assets");
    if (!fs.existsSync(sourceAssetsPath)) {
      throw new Error(`Bundled runtime assets are missing: ${sourceAssetsPath}`);
    }

    const packageDir = path.join(outDir, config.packageAlias);
    const vendorDir = path.join(
      packageDir,
      "vendor",
      config.targetTriple,
      "akra",
    );

    fs.rmSync(packageDir, { recursive: true, force: true });
    ensureDir(vendorDir);

    const destinationBinaryPath = path.join(vendorDir, config.binaryName);
    fs.copyFileSync(sourceBinaryPath, destinationBinaryPath);
    if (config.os !== "win32") {
      fs.chmodSync(destinationBinaryPath, 0o755);
    }
    copyDir(sourceAssetsPath, path.join(vendorDir, "assets"));

    writeJson(path.join(packageDir, "package.json"), {
      name: publishedPackageName,
      version: `${packageVersion}-${config.packageVersionSuffix}`,
      description: `Platform binary for akra (${config.os} ${config.cpu})`,
      os: [config.os],
      cpu: [config.cpu],
      files: ["vendor"],
      repository: {
        type: "git",
        url: "git+https://github.com/RefinedStone/codex-exec-loop.git",
        directory: "npm",
      },
      homepage: "https://github.com/RefinedStone/codex-exec-loop#readme",
      bugs: {
        url: "https://github.com/RefinedStone/codex-exec-loop/issues",
      },
      engines: {
        node: ">=18",
      },
    });

    fs.writeFileSync(
      path.join(packageDir, "README.md"),
      `# akra ${config.packageVersionSuffix}\n\nPrebuilt native binary for \`${config.targetTriple}\`.\n`,
    );
  } finally {
    fs.rmSync(extractDir, { recursive: true, force: true });
  }
}

function stageMainPackage({ outDir, packageVersion }) {
  const mainDir = path.join(outDir, "main");
  fs.rmSync(mainDir, { recursive: true, force: true });
  ensureDir(mainDir);

  for (const entry of ["bin", "lib"]) {
    copyDir(path.join(packageRoot, entry), path.join(mainDir, entry));
  }
  fs.copyFileSync(
    path.join(packageRoot, "README.md"),
    path.join(mainDir, "README.md"),
  );

  const basePackageJson = JSON.parse(
    fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"),
  );
  basePackageJson.version = packageVersion;
  basePackageJson.optionalDependencies = Object.fromEntries(
    PLATFORM_CONFIGS.map((config) => [
      config.packageAlias,
      `npm:${publishedPackageName}@${packageVersion}-${config.packageVersionSuffix}`,
    ]),
  );

  writeJson(path.join(mainDir, "package.json"), basePackageJson);
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const packageVersion = normalizeVersion(args.version);
  const releaseAssetsDir = path.resolve(process.cwd(), args.releaseAssetsDir);
  const outDir = path.resolve(process.cwd(), args.outDir);

  fs.rmSync(outDir, { recursive: true, force: true });
  ensureDir(outDir);

  for (const config of PLATFORM_CONFIGS) {
    const archivePath = findArchive(releaseAssetsDir, config.targetTriple);
    stagePlatformPackage({
      archivePath,
      outDir: path.join(outDir, "platforms"),
      packageVersion,
      config,
    });
  }

  stageMainPackage({ outDir, packageVersion });

  console.log(`main_package_dir=${path.join(outDir, "main")}`);
  for (const config of PLATFORM_CONFIGS) {
    console.log(
      `platform_package_dir_${config.packageVersionSuffix}=${path.join(
        outDir,
        "platforms",
        config.packageAlias,
      )}`,
    );
  }
}

main();
