#!/usr/bin/env node

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { PLATFORM_CONFIGS } from "../lib/platform.js";
import {
  cleanupVerifiedReleaseAssets,
  extractVerifiedArchive,
  verifyReleaseAssets,
} from "./verify-native-release-assets.mjs";

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
    if (!entry.isFile() || entry.isSymbolicLink()) {
      throw new Error(`Refusing unsafe runtime asset: ${sourcePath}`);
    }
    fs.copyFileSync(sourcePath, destinationPath, fs.constants.COPYFILE_EXCL);
  }
}

function copyRuntimeAssets(bundleRoot, vendorDir) {
  const runtimeSkillAssets = path.join(
    bundleRoot,
    "assets",
    "app-server",
    "skills",
  );
  if (!fs.existsSync(runtimeSkillAssets)) {
    throw new Error(
      `Bundled runtime skill assets are missing: ${runtimeSkillAssets}`,
    );
  }
  copyDir(
    runtimeSkillAssets,
    path.join(vendorDir, "assets", "app-server", "skills"),
  );
  const notices = path.join(bundleRoot, "THIRD_PARTY_NOTICES");
  if (!fs.existsSync(notices) || !fs.lstatSync(notices).isDirectory()) {
    throw new Error(`Bundled third-party notices are missing: ${notices}`);
  }
  const fontNotice = path.join(notices, "Galmuri-OFL.txt");
  if (!fs.existsSync(fontNotice)) {
    throw new Error(`Required third-party notice is missing: ${fontNotice}`);
  }
  const fontNoticeStat = fs.lstatSync(fontNotice);
  if (
    !fontNoticeStat.isFile() ||
    fontNoticeStat.isSymbolicLink() ||
    fontNoticeStat.nlink !== 1 ||
    fontNoticeStat.size <= 0
  ) {
    throw new Error(
      `Required third-party notice must be a nonempty single-link regular file: ${fontNotice}`,
    );
  }
  copyDir(notices, path.join(vendorDir, "THIRD_PARTY_NOTICES"));
}

function writeJson(filePath, value) {
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`);
}

function stagePlatformPackage({
  verifiedArchive,
  outDir,
  packageVersion,
  config,
}) {
  const extractDir = fs.mkdtempSync(
    path.join(os.tmpdir(), `akra-${config.packageVersionSuffix}-`),
  );

  try {
    const bundleRoot = extractVerifiedArchive(verifiedArchive, extractDir);
    const sourceBinaryPath = path.join(bundleRoot, config.binaryName);
    if (
      !fs.existsSync(sourceBinaryPath) ||
      !fs.lstatSync(sourceBinaryPath).isFile()
    ) {
      throw new Error(`Bundled binary is missing: ${sourceBinaryPath}`);
    }
    const sourceScriptsPath = path.join(bundleRoot, "scripts");
    if (
      !fs.existsSync(sourceScriptsPath) ||
      !fs.lstatSync(sourceScriptsPath).isDirectory()
    ) {
      throw new Error(`Bundled runtime scripts are missing: ${sourceScriptsPath}`);
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
    fs.copyFileSync(
      sourceBinaryPath,
      destinationBinaryPath,
      fs.constants.COPYFILE_EXCL,
    );
    if (config.os !== "win32") {
      fs.chmodSync(destinationBinaryPath, 0o755);
    }
    copyRuntimeAssets(bundleRoot, vendorDir);
    copyDir(sourceScriptsPath, path.join(vendorDir, "scripts"));

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

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const packageVersion = normalizeVersion(args.version);
  const releaseAssetsDir = path.resolve(process.cwd(), args.releaseAssetsDir);
  const outDir = path.resolve(process.cwd(), args.outDir);
  const verifiedArchives = await verifyReleaseAssets(releaseAssetsDir, args.version);

  try {
    fs.rmSync(outDir, { recursive: true, force: true });
    ensureDir(outDir);

    for (const config of PLATFORM_CONFIGS) {
      stagePlatformPackage({
        verifiedArchive: verifiedArchives.get(config.targetTriple),
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
  } catch (error) {
    fs.rmSync(outDir, { recursive: true, force: true });
    throw error;
  } finally {
    cleanupVerifiedReleaseAssets(verifiedArchives);
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
