#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const REGISTRY = "https://registry.npmjs.org";
const PACKAGE_NAME = "@refinedstone/akra";
const STAGING_DIST_TAG = "release-staging";
const RECONCILE_ATTEMPTS = 8;
const RECONCILE_RETRY_DELAY_MS = process.env.NODE_ENV === "test" ? 5 : 500;
const REGISTRY_RETRY_DELAY_MS = process.env.NODE_ENV === "test" ? 5 : 2000;
const STABLE_VERSION = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/;
const PLATFORM_VERSION =
  /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-(linux-x64|darwin-arm64|win32-x64)$/;
const SEMVER_VERSION =
  /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-((?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

function parseArgs(argv) {
  const args = { expectedVersion: "", packageDir: "", tag: "" };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    switch (arg) {
      case "--package-dir":
        args.packageDir = argv[++index] ?? "";
        break;
      case "--expected-version":
        args.expectedVersion = argv[++index] ?? "";
        break;
      case "--tag":
        args.tag = argv[++index] ?? "";
        break;
      default:
        throw new Error(`unsupported option: ${arg}`);
    }
  }

  if (!args.packageDir) {
    throw new Error("--package-dir is required");
  }
  if (!args.expectedVersion) {
    throw new Error("--expected-version is required");
  }
  if (!args.tag) {
    throw new Error("--tag is required");
  }
  if (!new Set(["latest", "platform"]).has(args.tag)) {
    throw new Error("--tag must be latest or platform");
  }

  return args;
}

function readPackage(packageDir, tag, expectedVersion) {
  const packagePath = path.join(packageDir, "package.json");
  const packageJson = JSON.parse(fs.readFileSync(packagePath, "utf8"));

  if (packageJson.name !== PACKAGE_NAME) {
    throw new Error(
      `refusing to publish unexpected package: ${packageJson.name ?? "<missing>"}`,
    );
  }
  const versionPolicy = tag === "latest" ? STABLE_VERSION : PLATFORM_VERSION;
  if (
    typeof packageJson.version !== "string" ||
    !versionPolicy.test(packageJson.version)
  ) {
    throw new Error(
      `${PACKAGE_NAME}@${packageJson.version ?? "<missing>"} is not valid for the ${tag} dist-tag`,
    );
  }
  if (packageJson.version !== expectedVersion) {
    throw new Error(
      `${PACKAGE_NAME}@${packageJson.version} does not match the frozen release version ${expectedVersion}`,
    );
  }

  const lifecycleScripts = new Set([
    "dependencies",
    "install",
    "postinstall",
    "postpack",
    "postprepare",
    "postpublish",
    "preinstall",
    "prepack",
    "prepare",
    "preprepare",
    "prepublish",
    "prepublishOnly",
    "publish",
  ]);
  const scripts = packageJson.scripts;
  if (
    scripts !== undefined &&
    (scripts === null || typeof scripts !== "object" || Array.isArray(scripts))
  ) {
    throw new Error(
      `${PACKAGE_NAME}@${packageJson.version} has an invalid scripts manifest`,
    );
  }
  const unsafeScript = Object.keys(scripts ?? {}).find((name) =>
    lifecycleScripts.has(name),
  );
  if (unsafeScript) {
    throw new Error(
      `${PACKAGE_NAME}@${packageJson.version} contains forbidden npm lifecycle script ${unsafeScript}`,
    );
  }

  const publishConfig = packageJson.publishConfig;
  if (
    publishConfig !== undefined &&
    (publishConfig === null ||
      typeof publishConfig !== "object" ||
      Array.isArray(publishConfig))
  ) {
    throw new Error(
      `${PACKAGE_NAME}@${packageJson.version} has an invalid publishConfig`,
    );
  }
  const publishConfigKeys = Object.keys(publishConfig ?? {});
  const unsupportedPublishConfig = publishConfigKeys.find(
    (name) => name !== "access",
  );
  if (unsupportedPublishConfig) {
    throw new Error(
      `${PACKAGE_NAME}@${packageJson.version} contains forbidden publishConfig.${unsupportedPublishConfig}`,
    );
  }
  if (publishConfig?.access !== undefined && publishConfig.access !== "public") {
    throw new Error(
      `${PACKAGE_NAME}@${packageJson.version} must not override public npm access`,
    );
  }

  return packageJson;
}

function createNpmRuntime(tempDir) {
  const runtimeDir = path.join(tempDir, "runtime");
  const cacheDir = path.join(runtimeDir, "cache");
  const projectConfigPath = path.join(runtimeDir, ".npmrc");
  const userConfigPath = path.join(runtimeDir, "user.npmrc");
  const globalConfigPath = path.join(runtimeDir, "global.npmrc");
  fs.mkdirSync(cacheDir, { recursive: true, mode: 0o700 });
  const controlledConfig = [
    `registry=${REGISTRY}/`,
    `@refinedstone:registry=${REGISTRY}/`,
    `//registry.npmjs.org/:_authToken=\${NODE_AUTH_TOKEN}`,
    "",
  ].join("\n");
  fs.writeFileSync(projectConfigPath, controlledConfig, { mode: 0o600 });
  fs.writeFileSync(userConfigPath, controlledConfig, { mode: 0o600 });
  fs.writeFileSync(globalConfigPath, "", { mode: 0o600 });
  fs.writeFileSync(
    path.join(runtimeDir, "package.json"),
    '{"name":"akra-release-runtime","private":true}\n',
    { mode: 0o600 },
  );

  const env = {};
  for (const [name, value] of Object.entries(process.env)) {
    if (!name.toUpperCase().startsWith("NPM_CONFIG_")) {
      env[name] = value;
    }
  }
  Object.assign(env, {
    NPM_CONFIG_CACHE: cacheDir,
    NPM_CONFIG_GLOBALCONFIG: globalConfigPath,
    NPM_CONFIG_REGISTRY: REGISTRY,
    NPM_CONFIG_UPDATE_NOTIFIER: "false",
    NPM_CONFIG_USERCONFIG: userConfigPath,
  });
  return { cwd: runtimeDir, env };
}

function runNpm(args, runtime) {
  return spawnSync("npm", args, {
    cwd: runtime.cwd,
    encoding: "utf8",
    env: runtime.env,
    maxBuffer: 10 * 1024 * 1024,
  });
}

function npmFailure(result, operation) {
  if (result.error) {
    return new Error(`${operation} could not start: ${result.error.message}`);
  }
  const output = `${result.stderr ?? ""}\n${result.stdout ?? ""}`.trim();
  const token = process.env.NODE_AUTH_TOKEN;
  const redacted = token ? output.replaceAll(token, "[redacted]") : output;
  const detail = redacted ? `: ${redacted.slice(0, 2000)}` : "";
  return new Error(
    `${operation} failed with npm exit ${result.status ?? "unknown"}${detail}`,
  );
}

function immutablePublishConflict(result) {
  const output = `${result.stderr ?? ""}\n${result.stdout ?? ""}`;
  return /\b(?:E409|EPUBLISHCONFLICT)\b/.test(output);
}

function packPackage(packageDir, outputDir, runtime) {
  const result = runNpm([
    "pack",
    packageDir,
    "--json",
    "--ignore-scripts",
    "--pack-destination",
    outputDir,
  ], runtime);
  if (result.status !== 0) {
    throw npmFailure(result, "npm pack");
  }

  let records;
  try {
    records = JSON.parse(result.stdout);
  } catch {
    throw new Error("npm pack did not return valid JSON");
  }
  if (!Array.isArray(records) || records.length !== 1) {
    throw new Error("npm pack must return exactly one package record");
  }

  const [{ filename, integrity }] = records;
  if (typeof filename !== "string" || path.basename(filename) !== filename) {
    throw new Error("npm pack returned an unsafe tarball filename");
  }
  if (typeof integrity !== "string" || !integrity.startsWith("sha512-")) {
    throw new Error("npm pack did not return a sha512 integrity");
  }

  const tarballPath = path.join(outputDir, filename);
  if (!fs.existsSync(tarballPath) || !fs.statSync(tarballPath).isFile()) {
    throw new Error("npm pack did not create the reported tarball");
  }
  return { integrity, tarballPath };
}

function publishedIntegrity(spec, runtime) {
  const result = runNpm([
    "view",
    spec,
    "dist.integrity",
    "--json",
    "--registry",
    REGISTRY,
  ], runtime);
  if (result.status !== 0) {
    const output = `${result.stderr ?? ""}\n${result.stdout ?? ""}`;
    if (/\bE404\b/.test(output)) {
      return null;
    }
    throw npmFailure(result, `npm view ${spec}`);
  }

  let integrity;
  try {
    integrity = JSON.parse(result.stdout);
  } catch {
    throw new Error(`npm view ${spec} did not return valid JSON`);
  }
  if (typeof integrity !== "string" || !integrity.startsWith("sha512-")) {
    throw new Error(`npm view ${spec} did not return a sha512 dist.integrity`);
  }
  return integrity;
}

function publishedDistTag(tag, runtime) {
  const result = runNpm([
    "view",
    PACKAGE_NAME,
    `dist-tags.${tag}`,
    "--json",
    "--registry",
    REGISTRY,
  ], runtime);
  if (result.status !== 0) {
    const output = `${result.stderr ?? ""}\n${result.stdout ?? ""}`;
    if (/\bE404\b/.test(output)) {
      return null;
    }
    throw npmFailure(result, `npm view ${PACKAGE_NAME} dist-tags.${tag}`);
  }

  const output = result.stdout.trim();
  if (!output) {
    return null;
  }

  let version;
  try {
    version = JSON.parse(output);
  } catch {
    throw new Error(
      `npm view ${PACKAGE_NAME} dist-tags.${tag} did not return valid JSON`,
    );
  }
  if (version === null) {
    return null;
  }
  if (typeof version !== "string" || !version) {
    throw new Error(
      `npm view ${PACKAGE_NAME} dist-tags.${tag} did not return a version`,
    );
  }
  return version;
}

function publishedVersions(runtime) {
  const result = runNpm([
    "view",
    PACKAGE_NAME,
    "versions",
    "--json",
    "--registry",
    REGISTRY,
  ], runtime);
  if (result.status !== 0) {
    const output = `${result.stderr ?? ""}\n${result.stdout ?? ""}`;
    if (/\bE404\b/.test(output)) {
      return [];
    }
    throw npmFailure(result, `npm view ${PACKAGE_NAME} versions`);
  }

  let versions;
  try {
    versions = JSON.parse(result.stdout);
  } catch {
    throw new Error(
      `npm view ${PACKAGE_NAME} versions did not return valid JSON`,
    );
  }
  if (typeof versions === "string") {
    versions = [versions];
  }
  if (
    !Array.isArray(versions) ||
    versions.some(
      (version) => typeof version !== "string" || !SEMVER_VERSION.test(version),
    )
  ) {
    throw new Error(
      `npm view ${PACKAGE_NAME} versions did not return SemVer strings`,
    );
  }
  return versions;
}

function parseSemVer(version) {
  const match = SEMVER_VERSION.exec(version);
  if (!match) {
    throw new Error(
      `npm registry returned invalid SemVer for ${PACKAGE_NAME}@latest: ${version}`,
    );
  }
  return {
    core: match.slice(1, 4).map((part) => BigInt(part)),
    prerelease: match[4]?.split(".") ?? [],
  };
}

function compareSemVer(leftVersion, rightVersion) {
  const left = parseSemVer(leftVersion);
  const right = parseSemVer(rightVersion);

  for (let index = 0; index < left.core.length; index += 1) {
    if (left.core[index] !== right.core[index]) {
      return left.core[index] < right.core[index] ? -1 : 1;
    }
  }

  if (left.prerelease.length === 0 || right.prerelease.length === 0) {
    if (left.prerelease.length === right.prerelease.length) {
      return 0;
    }
    return left.prerelease.length === 0 ? 1 : -1;
  }

  const count = Math.max(left.prerelease.length, right.prerelease.length);
  for (let index = 0; index < count; index += 1) {
    const leftPart = left.prerelease[index];
    const rightPart = right.prerelease[index];
    if (leftPart === undefined || rightPart === undefined) {
      return leftPart === undefined ? -1 : 1;
    }
    if (leftPart === rightPart) {
      continue;
    }

    const leftIsNumeric = /^(0|[1-9][0-9]*)$/.test(leftPart);
    const rightIsNumeric = /^(0|[1-9][0-9]*)$/.test(rightPart);
    if (leftIsNumeric && rightIsNumeric) {
      return BigInt(leftPart) < BigInt(rightPart) ? -1 : 1;
    }
    if (leftIsNumeric !== rightIsNumeric) {
      return leftIsNumeric ? -1 : 1;
    }
    return leftPart < rightPart ? -1 : 1;
  }
  return 0;
}

function assertIntegrity(spec, localIntegrity, remoteIntegrity) {
  if (localIntegrity !== remoteIntegrity) {
    throw new Error(
      `integrity mismatch for immutable ${spec}; local npm pack differs from registry dist.integrity`,
    );
  }
}

function sleep(milliseconds) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
}

function verifyPublishedWithRetry(spec, localIntegrity, runtime) {
  for (let attempt = 1; attempt <= 5; attempt += 1) {
    const remoteIntegrity = publishedIntegrity(spec, runtime);
    if (remoteIntegrity !== null) {
      assertIntegrity(spec, localIntegrity, remoteIntegrity);
      return;
    }
    if (attempt < 5) {
      sleep(REGISTRY_RETRY_DELAY_MS);
    }
  }
  throw new Error(
    `npm registry did not expose dist.integrity for ${spec} after publish`,
  );
}

function addDistTag(spec, tag, runtime) {
  const result = runNpm(
    ["dist-tag", "add", spec, tag, "--registry", REGISTRY],
    runtime,
  );
  if (result.status !== 0) {
    throw npmFailure(result, `npm dist-tag add ${spec} ${tag}`);
  }
}

function tagPolicyPattern(tag) {
  return tag === "latest" ? STABLE_VERSION : PLATFORM_VERSION;
}

function tagSnapshot(tag, requestedVersion, runtime) {
  const policyPattern = tagPolicyPattern(tag);
  const versions = publishedVersions(runtime);
  const candidates = versions.filter((version) => policyPattern.test(version));
  const highestVersion = candidates.reduce(
    (highest, version) =>
      highest === null || compareSemVer(version, highest) > 0
        ? version
        : highest,
    null,
  );
  return {
    currentVersion: publishedDistTag(tag, runtime),
    highestVersion,
    requestedVisible: versions.includes(requestedVersion),
  };
}

function snapshotConverged(snapshot) {
  return (
    snapshot.requestedVisible &&
    snapshot.highestVersion !== null &&
    snapshot.currentVersion === snapshot.highestVersion
  );
}

function snapshotWouldDowngrade(snapshot, tag) {
  return (
    snapshot.currentVersion !== null &&
    snapshot.highestVersion !== null &&
    tagPolicyPattern(tag).test(snapshot.currentVersion) &&
    compareSemVer(snapshot.currentVersion, snapshot.highestVersion) > 0
  );
}

function reconcileDistTag(tag, requestedVersion, runtime) {
  let lastSnapshot = null;

  for (let attempt = 1; attempt <= RECONCILE_ATTEMPTS; attempt += 1) {
    const before = tagSnapshot(tag, requestedVersion, runtime);
    lastSnapshot = before;
    if (
      before.requestedVisible &&
      before.highestVersion !== null &&
      !snapshotWouldDowngrade(before, tag) &&
      before.currentVersion !== before.highestVersion
    ) {
      addDistTag(`${PACKAGE_NAME}@${before.highestVersion}`, tag, runtime);
    }

    const after = tagSnapshot(tag, requestedVersion, runtime);
    lastSnapshot = after;
    if (snapshotConverged(after)) {
      return after.highestVersion;
    }

    if (
      attempt < RECONCILE_ATTEMPTS &&
      (!after.requestedVisible ||
        after.highestVersion === null ||
        snapshotWouldDowngrade(after, tag))
    ) {
      sleep(RECONCILE_RETRY_DELAY_MS);
    }
  }

  throw new Error(
    `${PACKAGE_NAME}@${tag} did not converge after ${RECONCILE_ATTEMPTS} attempts; ` +
      `requested=${requestedVersion}, highest=${lastSnapshot?.highestVersion ?? "<missing>"}, ` +
      `current=${lastSnapshot?.currentVersion ?? "<missing>"}`,
  );
}

function publishPackage(packageDir, tag, expectedVersion) {
  const packageJson = readPackage(packageDir, tag, expectedVersion);
  const spec = `${packageJson.name}@${packageJson.version}`;
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "akra-npm-pack-"));

  try {
    const runtime = createNpmRuntime(tempDir);
    const { integrity: localIntegrity, tarballPath } = packPackage(
      packageDir,
      tempDir,
      runtime,
    );
    const remoteIntegrity = publishedIntegrity(spec, runtime);
    if (remoteIntegrity !== null) {
      assertIntegrity(spec, localIntegrity, remoteIntegrity);
      const resolvedVersion = reconcileDistTag(
        tag,
        packageJson.version,
        runtime,
      );
      console.log(
        `Verified existing ${spec}; registry integrity matches local npm pack and ` +
          `${PACKAGE_NAME}@${tag} converged to ${resolvedVersion}.`,
      );
      return;
    }

    const result = runNpm([
      "publish",
      tarballPath,
      "--ignore-scripts",
      "--provenance",
      "--access",
      "public",
      "--tag",
      STAGING_DIST_TAG,
      "--registry",
      REGISTRY,
    ], runtime);
    let recoveredConcurrentPublish = false;
    if (result.status !== 0) {
      if (!immutablePublishConflict(result)) {
        throw npmFailure(result, `npm publish ${spec}`);
      }
      verifyPublishedWithRetry(spec, localIntegrity, runtime);
      recoveredConcurrentPublish = true;
    } else {
      verifyPublishedWithRetry(spec, localIntegrity, runtime);
    }

    const resolvedVersion = reconcileDistTag(tag, packageJson.version, runtime);
    const publicationStatus = recoveredConcurrentPublish
      ? `Verified concurrent publication of ${spec}`
      : `Published and verified ${spec} through the ${STAGING_DIST_TAG} dist-tag`;
    console.log(
      `${publicationStatus}; ${PACKAGE_NAME}@${tag} converged to ${resolvedVersion}.`,
    );
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}

try {
  const args = parseArgs(process.argv.slice(2));
  publishPackage(
    path.resolve(args.packageDir),
    args.tag,
    args.expectedVersion,
  );
} catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  console.error(`publish-package: ${message}`);
  process.exitCode = 1;
}
