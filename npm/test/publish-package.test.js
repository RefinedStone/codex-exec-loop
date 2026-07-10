import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const packageRoot = path.resolve(path.dirname(__filename), "..");
const publishScript = path.join(packageRoot, "scripts", "publish-package.mjs");
const packageName = "@refinedstone/akra";

const fakeNpm = `#!/usr/bin/env node
const fs = require("node:fs");
const path = require("node:path");
const packageName = "@refinedstone/akra";
const args = process.argv.slice(2);
fs.appendFileSync(process.env.FAKE_NPM_LOG, JSON.stringify(args) + "\\n");
if (process.env.FAKE_NPM_RUNTIME_LOG) {
  const npmConfig = Object.fromEntries(
    Object.entries(process.env)
      .filter(([name]) => name.toUpperCase().startsWith("NPM_CONFIG_"))
      .sort(([left], [right]) => left.localeCompare(right)),
  );
  const userConfig = fs.readFileSync(npmConfig.NPM_CONFIG_USERCONFIG, "utf8");
  const globalConfig = fs.readFileSync(npmConfig.NPM_CONFIG_GLOBALCONFIG, "utf8");
  const projectConfig = fs.readFileSync(path.join(process.cwd(), ".npmrc"), "utf8");
  fs.appendFileSync(
    process.env.FAKE_NPM_RUNTIME_LOG,
    JSON.stringify({
      cwd: process.cwd(),
      globalConfig,
      githubActions: process.env.GITHUB_ACTIONS,
      home: process.env.HOME,
      npmConfig,
      nodeAuthToken: process.env.NODE_AUTH_TOKEN,
      oidcToken: process.env.ACTIONS_ID_TOKEN_REQUEST_TOKEN,
      oidcUrl: process.env.ACTIONS_ID_TOKEN_REQUEST_URL,
      path: process.env.PATH,
      projectConfig,
      userConfig,
    }) + "\\n",
  );
}

function loadState() {
  return JSON.parse(fs.readFileSync(process.env.FAKE_NPM_STATE, "utf8"));
}

function saveState(state) {
  fs.writeFileSync(process.env.FAKE_NPM_STATE, JSON.stringify(state));
}

if (args[0] === "pack") {
  const destination = args[args.indexOf("--pack-destination") + 1];
  const filename = "refinedstone-akra-fixture.tgz";
  fs.writeFileSync(path.join(destination, filename), "fixture-tarball");
  console.log(JSON.stringify([{ filename, integrity: "sha512-local" }]));
  process.exit(0);
}

if (args[0] === "view") {
  const field = args[2];
  const state = loadState();
  if (field === "versions") {
    if (process.env.FAKE_NPM_MODE === "versions-view-failure") {
      console.error("npm error code ECONNRESET");
      process.exit(1);
    }
    console.log(JSON.stringify(state.versions));
    process.exit(0);
  }

  if (field.startsWith("dist-tags.")) {
    if (process.env.FAKE_NPM_MODE === "tag-view-failure") {
      console.error("npm error code ECONNRESET");
      process.exit(1);
    }
    const tag = field.slice("dist-tags.".length);
    console.log(JSON.stringify(state.tags[tag] ?? null));
    process.exit(0);
  }

  if (field !== "dist.integrity") {
    console.error("unexpected npm view field", field);
    process.exit(2);
  }
  if (process.env.FAKE_NPM_MODE === "integrity-view-failure") {
    console.error("npm error code ECONNRESET");
    process.exit(1);
  }
  if (process.env.FAKE_NPM_MODE === "existing-mismatch") {
    console.log(JSON.stringify("sha512-remote"));
    process.exit(0);
  }
  if (process.env.FAKE_NPM_MODE === "publish-conflict-mismatch" && state.published) {
    console.log(JSON.stringify("sha512-remote"));
    process.exit(0);
  }
  if (process.env.FAKE_NPM_PACKAGE_EXISTS === "true" || state.published) {
    console.log(JSON.stringify("sha512-local"));
    process.exit(0);
  }
  console.error("npm error code E404");
  process.exit(1);
}

if (args[0] === "publish") {
  const state = loadState();
  if (process.env.FAKE_NPM_MODE.startsWith("publish-conflict-")) {
    if (process.env.FAKE_NPM_MODE !== "publish-conflict-unexposed") {
      state.published = true;
      if (!state.versions.includes(process.env.FAKE_NPM_VERSION)) {
        state.versions.push(process.env.FAKE_NPM_VERSION);
      }
      state.tags["release-staging"] = process.env.FAKE_NPM_VERSION;
      saveState(state);
    }
    console.error("npm error code EPUBLISHCONFLICT");
    process.exit(1);
  }
  if (process.env.FAKE_NPM_MODE === "publish-other-failure") {
    console.error("npm error code E500 " + process.env.NODE_AUTH_TOKEN);
    process.exit(1);
  }
  state.published = true;
  if (!state.versions.includes(process.env.FAKE_NPM_VERSION)) {
    state.versions.push(process.env.FAKE_NPM_VERSION);
  }
  const tag = args[args.indexOf("--tag") + 1];
  state.tags[tag] = process.env.FAKE_NPM_VERSION;
  saveState(state);
  console.log("published");
  process.exit(0);
}

if (args[0] === "dist-tag" && args[1] === "add") {
  if (process.env.FAKE_NPM_MODE === "dist-tag-failure") {
    console.error("npm error code E403");
    process.exit(1);
  }
  const spec = args[2];
  if (!spec.startsWith(packageName + "@")) {
    console.error("unexpected dist-tag package", spec);
    process.exit(2);
  }
  const state = loadState();
  const version = spec.slice((packageName + "@").length);
  state.tags[args[3]] = version;
  if (process.env.FAKE_NPM_RACE_VERSION && !state.raceInjected) {
    state.raceInjected = true;
    if (!state.versions.includes(process.env.FAKE_NPM_RACE_VERSION)) {
      state.versions.push(process.env.FAKE_NPM_RACE_VERSION);
    }
    if (process.env.FAKE_NPM_RACE_MOVES_TAG === "true") {
      state.tags[args[3]] = process.env.FAKE_NPM_RACE_VERSION;
    }
  }
  saveState(state);
  console.log("+" + args[3] + ": " + spec);
  process.exit(0);
}

console.error("unexpected fake npm command", args[0]);
process.exit(2);
`;

function runPublisher({
  mode,
  version,
  tag,
  expectedVersion = version,
  packageJsonExtras = {},
  poisonNpmEnvironment = false,
  currentTag,
  versions,
  raceVersion = "",
  raceMovesTag = false,
}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "akra-publish-test-"));
  const packageDir = path.join(root, "package");
  const binDir = path.join(root, "bin");
  const logPath = path.join(root, "npm.log");
  const runtimeLogPath = path.join(root, "npm-runtime.log");
  const statePath = path.join(root, "registry.json");
  const publisherCwd = path.join(root, "publisher-cwd");
  const poisonCache = path.join(root, "poison-cache");
  const poisonUserConfig = path.join(root, "poison-user.npmrc");
  const fixtureHome = path.join(root, "fixture-home");
  fs.mkdirSync(packageDir, { recursive: true });
  fs.mkdirSync(binDir, { recursive: true });
  fs.mkdirSync(publisherCwd, { recursive: true });
  fs.mkdirSync(fixtureHome, { recursive: true });
  fs.writeFileSync(
    path.join(publisherCwd, ".npmrc"),
    "@refinedstone:registry=https://registry.attacker.invalid/\\n" +
      "//registry.attacker.invalid/:_authToken=${NODE_AUTH_TOKEN}\\n",
  );
  fs.writeFileSync(
    poisonUserConfig,
    "registry=https://user-config.attacker.invalid/\\n",
  );
  fs.writeFileSync(
    path.join(packageDir, "package.json"),
    JSON.stringify({ name: packageName, version, ...packageJsonExtras }),
  );
  const npmPath = path.join(binDir, "npm");
  fs.writeFileSync(npmPath, fakeNpm);
  fs.chmodSync(npmPath, 0o755);

  const packageExists = [
    "existing-match",
    "existing-mismatch",
    "dist-tag-failure",
    "integrity-view-failure",
    "tag-view-failure",
    "versions-view-failure",
  ].includes(mode);
  const initialVersions = versions ?? (packageExists ? [version] : []);
  const initialTag =
    currentTag === undefined ? (packageExists ? version : null) : currentTag;
  fs.writeFileSync(
    statePath,
    JSON.stringify({
      published: false,
      raceInjected: false,
      tags: initialTag === null ? {} : { [tag]: initialTag },
      versions: initialVersions,
    }),
  );

  const result = spawnSync(
    process.execPath,
    [
      publishScript,
      "--package-dir",
      packageDir,
      "--expected-version",
      expectedVersion,
      "--tag",
      tag,
    ],
    {
      cwd: publisherCwd,
      encoding: "utf8",
      env: {
        ...process.env,
        NODE_ENV: "test",
        PATH: `${binDir}${path.delimiter}${process.env.PATH ?? ""}`,
        NODE_AUTH_TOKEN: "fixture-token-must-not-be-printed",
        ACTIONS_ID_TOKEN_REQUEST_TOKEN: "fixture-oidc-token",
        ACTIONS_ID_TOKEN_REQUEST_URL: "https://oidc.actions.invalid/token",
        GITHUB_ACTIONS: "true",
        HOME: fixtureHome,
        FAKE_NPM_RUNTIME_LOG: runtimeLogPath,
        FAKE_NPM_LOG: logPath,
        FAKE_NPM_MODE: mode,
        FAKE_NPM_PACKAGE_EXISTS: packageExists ? "true" : "false",
        FAKE_NPM_RACE_MOVES_TAG: raceMovesTag ? "true" : "false",
        FAKE_NPM_RACE_VERSION: raceVersion,
        FAKE_NPM_STATE: statePath,
        FAKE_NPM_VERSION: version,
        ...(poisonNpmEnvironment
          ? {
              NPM_CONFIG_CACHE: poisonCache,
              NPM_CONFIG_PROVENANCE: "false",
              NPM_CONFIG_REGISTRY: "https://env.attacker.invalid/",
              NPM_CONFIG_USERCONFIG: poisonUserConfig,
              npm_config_script_shell: path.join(root, "steal-token.sh"),
              npm_config_tag: "attacker",
            }
          : {}),
      },
    },
  );
  const calls = fs.existsSync(logPath)
    ? fs
        .readFileSync(logPath, "utf8")
        .trim()
        .split("\n")
        .filter(Boolean)
        .map((line) => JSON.parse(line))
    : [];
  const runtimeRecords = fs.existsSync(runtimeLogPath)
    ? fs
        .readFileSync(runtimeLogPath, "utf8")
        .trim()
        .split("\n")
        .filter(Boolean)
        .map((line) => JSON.parse(line))
    : [];

  return {
    calls,
    cleanup: () => fs.rmSync(root, { recursive: true, force: true }),
    result,
    runtimeRecords,
    state: JSON.parse(fs.readFileSync(statePath, "utf8")),
  };
}

function commandNames(fixture) {
  return fixture.calls.map((args) => args[0]);
}

function distTagAdds(fixture) {
  return fixture.calls.filter((args) => args[0] === "dist-tag");
}

test("existing npm versions require matching immutable integrity before reconciliation", () => {
  const fixture = runPublisher({
    mode: "existing-match",
    version: "1.2.3",
    tag: "latest",
  });
  try {
    assert.equal(fixture.result.status, 0, fixture.result.stderr);
    assert.match(
      fixture.result.stdout,
      /registry integrity matches local npm pack/,
    );
    assert.match(fixture.result.stdout, /latest converged to 1\.2\.3/);
    assert.equal(fixture.calls[0][0], "pack");
    assert.equal(fixture.calls[1][2], "dist.integrity");
    assert.deepEqual(distTagAdds(fixture), []);
    assert.doesNotMatch(
      fixture.result.stdout + fixture.result.stderr,
      /fixture-token/,
    );
  } finally {
    fixture.cleanup();
  }
});

test("existing npm versions fail closed when registry integrity differs", () => {
  const fixture = runPublisher({
    mode: "existing-mismatch",
    version: "1.2.3",
    tag: "latest",
  });
  try {
    assert.notEqual(fixture.result.status, 0);
    assert.match(fixture.result.stderr, /integrity mismatch for immutable/);
    assert.deepEqual(commandNames(fixture), ["pack", "view"]);
    assert.deepEqual(distTagAdds(fixture), []);
  } finally {
    fixture.cleanup();
  }
});

test("registry lookup failures never fall through to a dist-tag mutation", () => {
  for (const [mode, expectedCalls] of [
    ["integrity-view-failure", ["pack", "view"]],
    ["versions-view-failure", ["pack", "view", "view"]],
    ["tag-view-failure", ["pack", "view", "view", "view"]],
  ]) {
    const fixture = runPublisher({ mode, version: "1.2.3", tag: "latest" });
    try {
      assert.notEqual(fixture.result.status, 0, mode);
      assert.match(fixture.result.stderr, /npm view .* failed with npm exit 1/);
      assert.deepEqual(commandNames(fixture), expectedCalls);
      assert.deepEqual(distTagAdds(fixture), []);
    } finally {
      fixture.cleanup();
    }
  }
});

test("stable main publication uses a staging tag before reconciling latest", () => {
  const fixture = runPublisher({
    mode: "missing-publish",
    version: "1.2.3",
    tag: "latest",
  });
  try {
    assert.equal(fixture.result.status, 0, fixture.result.stderr);
    const publish = fixture.calls.find((args) => args[0] === "publish");
    assert(publish);
    assert.deepEqual(
      publish.slice(publish.indexOf("--tag"), publish.indexOf("--tag") + 2),
      ["--tag", "release-staging"],
    );
    assert.equal(fixture.state.tags.latest, "1.2.3");
    assert.equal(fixture.state.tags["release-staging"], "1.2.3");
    assert.deepEqual(distTagAdds(fixture)[0].slice(1, 4), [
      "add",
      `${packageName}@1.2.3`,
      "latest",
    ]);
  } finally {
    fixture.cleanup();
  }
});

test("same-version publish conflicts recover only when registry integrity matches", () => {
  const fixture = runPublisher({
    mode: "publish-conflict-match",
    version: "1.2.3",
    tag: "latest",
  });
  try {
    assert.equal(fixture.result.status, 0, fixture.result.stderr);
    assert.match(fixture.result.stdout, /Verified concurrent publication of/);
    assert.equal(
      fixture.calls.filter((args) => args[0] === "publish").length,
      1,
    );
    assert.equal(fixture.state.tags.latest, "1.2.3");
    assert.equal(fixture.state.tags["release-staging"], "1.2.3");
  } finally {
    fixture.cleanup();
  }
});

test("same-version publish conflicts reject different or unexposed registry bytes", () => {
  for (const [mode, expectedError] of [
    ["publish-conflict-mismatch", /integrity mismatch for immutable/],
    ["publish-conflict-unexposed", /registry did not expose dist\.integrity/],
  ]) {
    const fixture = runPublisher({ mode, version: "1.2.3", tag: "latest" });
    try {
      assert.notEqual(fixture.result.status, 0, mode);
      assert.match(fixture.result.stderr, expectedError);
      assert.deepEqual(distTagAdds(fixture), []);
    } finally {
      fixture.cleanup();
    }
  }
});

test("non-conflict publish failures stay fail-closed and redact the token", () => {
  const fixture = runPublisher({
    mode: "publish-other-failure",
    version: "1.2.3",
    tag: "latest",
  });
  try {
    assert.notEqual(fixture.result.status, 0);
    assert.match(
      fixture.result.stderr,
      /npm publish .* failed with npm exit 1/,
    );
    assert.match(fixture.result.stderr, /\[redacted\]/);
    assert.doesNotMatch(
      fixture.result.stderr,
      /fixture-token-must-not-be-printed/,
    );
    assert.deepEqual(distTagAdds(fixture), []);
  } finally {
    fixture.cleanup();
  }
});

test("platform publication uses staging and converges the platform tag deterministically", () => {
  const fixture = runPublisher({
    mode: "missing-publish",
    version: "1.2.3-linux-x64",
    tag: "platform",
  });
  try {
    assert.equal(fixture.result.status, 0, fixture.result.stderr);
    const publish = fixture.calls.find((args) => args[0] === "publish");
    assert(publish);
    assert.deepEqual(
      publish.slice(publish.indexOf("--tag"), publish.indexOf("--tag") + 2),
      ["--tag", "release-staging"],
    );
    assert.equal(fixture.state.tags.platform, "1.2.3-linux-x64");
  } finally {
    fixture.cleanup();
  }
});

test("an older existing release preserves a higher latest tag and succeeds", () => {
  const fixture = runPublisher({
    mode: "existing-match",
    version: "1.2.3",
    tag: "latest",
    currentTag: "1.2.4",
    versions: ["1.2.3", "1.2.4", "1.2.4-linux-x64"],
  });
  try {
    assert.equal(fixture.result.status, 0, fixture.result.stderr);
    assert.match(fixture.result.stdout, /latest converged to 1\.2\.4/);
    assert.equal(fixture.state.tags.latest, "1.2.4");
    assert.deepEqual(distTagAdds(fixture), []);
  } finally {
    fixture.cleanup();
  }
});

test("publishing a missing older release never moves latest backward", () => {
  const fixture = runPublisher({
    mode: "missing-publish",
    version: "1.2.3",
    tag: "latest",
    currentTag: "1.2.4",
    versions: ["1.2.4"],
  });
  try {
    assert.equal(fixture.result.status, 0, fixture.result.stderr);
    assert.equal(fixture.state.tags.latest, "1.2.4");
    assert(fixture.state.versions.includes("1.2.3"));
    assert.deepEqual(distTagAdds(fixture), []);
  } finally {
    fixture.cleanup();
  }
});

test("an older workflow converges to a newer version that appears after its mutation", () => {
  const fixture = runPublisher({
    mode: "existing-match",
    version: "1.3.0",
    tag: "latest",
    currentTag: "1.2.0",
    versions: ["1.2.0", "1.3.0"],
    raceVersion: "1.4.0",
  });
  try {
    assert.equal(fixture.result.status, 0, fixture.result.stderr);
    assert.equal(fixture.state.tags.latest, "1.4.0");
    assert.deepEqual(
      distTagAdds(fixture).map((args) => args[2]),
      [`${packageName}@1.3.0`, `${packageName}@1.4.0`],
    );
  } finally {
    fixture.cleanup();
  }
});

test("a newer workflow remains highest when an older version appears during reconciliation", () => {
  const fixture = runPublisher({
    mode: "existing-match",
    version: "1.4.0",
    tag: "latest",
    currentTag: "1.2.0",
    versions: ["1.2.0", "1.4.0"],
    raceVersion: "1.3.0",
    raceMovesTag: true,
  });
  try {
    assert.equal(fixture.result.status, 0, fixture.result.stderr);
    assert.equal(fixture.state.tags.latest, "1.4.0");
    assert.deepEqual(
      distTagAdds(fixture).map((args) => args[2]),
      [`${packageName}@1.4.0`, `${packageName}@1.4.0`],
    );
  } finally {
    fixture.cleanup();
  }
});

test("platform reconciliation uses the highest full platform version, not the requested alias", () => {
  const fixture = runPublisher({
    mode: "existing-match",
    version: "1.2.3-linux-x64",
    tag: "platform",
    currentTag: "1.2.3-darwin-arm64",
    versions: [
      "1.2.3-darwin-arm64",
      "1.2.3-linux-x64",
      "1.2.3-win32-x64",
      "1.2.3",
    ],
  });
  try {
    assert.equal(fixture.result.status, 0, fixture.result.stderr);
    assert.equal(fixture.state.tags.platform, "1.2.3-win32-x64");
    assert.deepEqual(distTagAdds(fixture)[0].slice(1, 4), [
      "add",
      `${packageName}@1.2.3-win32-x64`,
      "platform",
    ]);
  } finally {
    fixture.cleanup();
  }
});

test("dist-tag reconciliation failures are fail-closed", () => {
  const fixture = runPublisher({
    mode: "dist-tag-failure",
    version: "1.2.3",
    tag: "latest",
    currentTag: "1.2.2",
    versions: ["1.2.2", "1.2.3"],
  });
  try {
    assert.notEqual(fixture.result.status, 0);
    assert.match(
      fixture.result.stderr,
      /npm dist-tag add .* failed with npm exit 1/,
    );
    assert.equal(distTagAdds(fixture).length, 1);
  } finally {
    fixture.cleanup();
  }
});

test("reconciliation is bounded when the requested version never reaches the versions index", () => {
  const fixture = runPublisher({
    mode: "existing-match",
    version: "1.3.0",
    tag: "latest",
    currentTag: "1.2.0",
    versions: ["1.2.0"],
  });
  try {
    assert.notEqual(fixture.result.status, 0);
    assert.match(fixture.result.stderr, /did not converge after 8 attempts/);
    assert.equal(
      fixture.calls.filter(
        (args) => args[0] === "view" && args[2] === "versions",
      ).length,
      16,
    );
    assert.deepEqual(distTagAdds(fixture), []);
    assert.equal(fixture.state.tags.latest, "1.2.0");
  } finally {
    fixture.cleanup();
  }
});

test("latest rejects prerelease versions before invoking npm", () => {
  const fixture = runPublisher({
    mode: "existing-match",
    version: "1.2.3-rc.1",
    tag: "latest",
  });
  try {
    assert.notEqual(fixture.result.status, 0);
    assert.match(fixture.result.stderr, /not valid for the latest dist-tag/);
    assert.deepEqual(fixture.calls, []);
  } finally {
    fixture.cleanup();
  }
});

test("publication requires the staged package to match the frozen release version", () => {
  const fixture = runPublisher({
    mode: "missing-publish",
    version: "1.2.4",
    expectedVersion: "1.2.3",
    tag: "latest",
  });
  try {
    assert.notEqual(fixture.result.status, 0);
    assert.match(fixture.result.stderr, /does not match the frozen release version 1\.2\.3/);
    assert.deepEqual(fixture.calls, []);
  } finally {
    fixture.cleanup();
  }
});

test("publication rejects install-time npm lifecycle scripts before packing", () => {
  const fixture = runPublisher({
    mode: "missing-publish",
    version: "1.2.3",
    tag: "latest",
    packageJsonExtras: { scripts: { preinstall: "node steal-token.js" } },
  });
  try {
    assert.notEqual(fixture.result.status, 0);
    assert.match(fixture.result.stderr, /forbidden npm lifecycle script preinstall/);
    assert.deepEqual(fixture.calls, []);
  } finally {
    fixture.cleanup();
  }
});

test("npm subprocesses ignore project, user, and inherited npm configuration", () => {
  const fixture = runPublisher({
    mode: "existing-match",
    version: "1.2.3",
    tag: "latest",
    poisonNpmEnvironment: true,
  });
  try {
    assert.equal(fixture.result.status, 0, fixture.result.stderr);
    assert(fixture.runtimeRecords.length > 0);
    for (const record of fixture.runtimeRecords) {
      assert.match(record.cwd, /akra-npm-pack-.*[/\\]runtime$/);
      assert.deepEqual(Object.keys(record.npmConfig).sort(), [
        "NPM_CONFIG_CACHE",
        "NPM_CONFIG_GLOBALCONFIG",
        "NPM_CONFIG_REGISTRY",
        "NPM_CONFIG_UPDATE_NOTIFIER",
        "NPM_CONFIG_USERCONFIG",
      ]);
      assert.equal(record.npmConfig.NPM_CONFIG_REGISTRY, "https://registry.npmjs.org");
      assert.match(record.npmConfig.NPM_CONFIG_CACHE, /akra-npm-pack-/);
      assert.match(record.npmConfig.NPM_CONFIG_USERCONFIG, /akra-npm-pack-/);
      assert.equal(record.globalConfig, "");
      assert.match(
        record.userConfig,
        /@refinedstone:registry=https:\/\/registry\.npmjs\.org\//,
      );
      assert.equal(record.projectConfig, record.userConfig);
      assert.match(
        record.userConfig,
        /\/\/registry\.npmjs\.org\/:_authToken=\$\{NODE_AUTH_TOKEN\}/,
      );
      assert.doesNotMatch(record.userConfig, /fixture-token-must-not-be-printed/);
      assert.doesNotMatch(record.userConfig, /attacker\.invalid/);
      assert.equal(record.nodeAuthToken, "fixture-token-must-not-be-printed");
      assert.equal(record.oidcToken, "fixture-oidc-token");
      assert.equal(record.oidcUrl, "https://oidc.actions.invalid/token");
      assert.equal(record.githubActions, "true");
      assert.match(record.home, /fixture-home$/);
      assert(record.path.includes(path.delimiter));
    }
  } finally {
    fixture.cleanup();
  }
});

test("publication rejects registry, tag, access, and unknown publishConfig overrides", () => {
  for (const [publishConfig, expectedError] of [
    [
      { registry: "https://registry.attacker.invalid/" },
      /forbidden publishConfig\.registry/,
    ],
    [{ tag: "attacker" }, /forbidden publishConfig\.tag/],
    [{ access: "restricted" }, /must not override public npm access/],
    [{ access: "public", provenance: false }, /forbidden publishConfig\.provenance/],
  ]) {
    const fixture = runPublisher({
      mode: "missing-publish",
      version: "1.2.3",
      tag: "latest",
      packageJsonExtras: { publishConfig },
    });
    try {
      assert.notEqual(fixture.result.status, 0);
      assert.match(fixture.result.stderr, expectedError);
      assert.deepEqual(fixture.calls, []);
    } finally {
      fixture.cleanup();
    }
  }
});
