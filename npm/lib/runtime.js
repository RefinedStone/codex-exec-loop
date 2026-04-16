import { existsSync } from "node:fs";
import path from "node:path";

import { resolvePlatformConfig } from "./platform.js";

function installCommand(packageManager) {
  if (packageManager === "bun") {
    return "bun install -g @refinedstone/akra@latest";
  }

  return "npm install -g @refinedstone/akra@latest";
}

export function detectPackageManager({
  env = process.env,
  dirname = "",
} = {}) {
  const userAgent = env.npm_config_user_agent || "";
  if (/\bbun\//.test(userAgent)) {
    return "bun";
  }

  const execPath = env.npm_execpath || "";
  if (execPath.includes("bun")) {
    return "bun";
  }

  if (
    dirname.includes(".bun/install/global") ||
    dirname.includes(".bun\\install\\global")
  ) {
    return "bun";
  }

  return userAgent ? "npm" : null;
}

export function resolveBinaryPath({
  packageRoot,
  platform = process.platform,
  arch = process.arch,
  resolvePackageJson,
  existsSyncFn = existsSync,
  env = process.env,
} = {}) {
  const config = resolvePlatformConfig(platform, arch);
  if (!config) {
    throw new Error(`Unsupported platform: ${platform} (${arch})`);
  }

  const localVendorRoot = path.join(packageRoot, "vendor");
  const localBinaryPath = path.join(
    localVendorRoot,
    config.targetTriple,
    "akra",
    config.binaryName,
  );

  let vendorRoot;
  try {
    const packageJsonPath = resolvePackageJson(
      `${config.packageAlias}/package.json`,
    );
    vendorRoot = path.join(path.dirname(packageJsonPath), "vendor");
  } catch {
    if (existsSyncFn(localBinaryPath)) {
      vendorRoot = localVendorRoot;
    } else {
      throw new Error(
        `Missing optional dependency ${config.packageAlias}. Reinstall akra: ${installCommand(
          detectPackageManager({ env, dirname: packageRoot }),
        )}`,
      );
    }
  }

  const binaryPath = path.join(
    vendorRoot,
    config.targetTriple,
    "akra",
    config.binaryName,
  );
  if (!existsSyncFn(binaryPath)) {
    throw new Error(`Installed akra binary is missing at ${binaryPath}`);
  }

  return { binaryPath, config };
}
