import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";

import { PLATFORM_CONFIGS } from "../lib/platform.js";
import {
  cleanupVerifiedReleaseAssets,
  extractVerifiedArchive,
  findAndVerifyArchive,
  verifyExtractedBundle,
} from "../scripts/verify-native-release-assets.mjs";

const __filename = fileURLToPath(import.meta.url);
const packageRoot = path.resolve(path.dirname(__filename), "..");
const repoRoot = path.resolve(packageRoot, "..");
const stageScript = path.join(packageRoot, "scripts", "stage-npm-packages.mjs");

function writeFile(filePath, body = "") {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, body);
}

function writeChecksum(archivePath) {
  const digest = createHash("sha256").update(fs.readFileSync(archivePath)).digest("hex");
  fs.writeFileSync(
    `${archivePath}.sha256`,
    `${digest}  ${path.basename(archivePath)}\n`,
  );
}

function createArchive(
  releaseAssetsDir,
  config,
  {
    noticeBody = "fixture font license\n",
    beforeManifest = null,
    afterManifest = null,
  } = {},
) {
  const rootName = `codex-exec-loop-native-0.1.0-${config.targetTriple}`;
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
  const launcherName = config.os === "win32" ? "akra.cmd" : "akra";
  writeFile(
    path.join(bundleRoot, launcherName),
    config.os === "win32" ? "@echo off\r\n" : "#!/usr/bin/env bash\n",
  );
  const notices = path.join(bundleRoot, "THIRD_PARTY_NOTICES");
  fs.mkdirSync(notices, { recursive: true });
  if (noticeBody !== null) {
    writeFile(path.join(notices, "Galmuri-OFL.txt"), noticeBody);
  }
  writeFile(
    path.join(bundleRoot, "VERSION.txt"),
    [
      "name=codex-exec-loop-native",
      "version=0.1.0",
      "release_tag=v0.1.0",
      `target=${config.targetTriple}`,
      "profile=release",
      `binary=${config.binaryName}`,
      `launcher=${launcherName}`,
      "",
    ].join("\n"),
  );
  beforeManifest?.(bundleRoot);

  const manifestPath = path.join(bundleRoot, "SHA256SUMS.txt");
  const manifest = collectRelativeFiles(bundleRoot)
    .filter((relativePath) => relativePath !== "SHA256SUMS.txt")
    .map((relativePath) => {
      const portablePath = relativePath.split(path.sep).join("/");
      const digest = createHash("sha256")
        .update(fs.readFileSync(path.join(bundleRoot, relativePath)))
        .digest("hex");
      return `${digest}  ${portablePath}`;
    });
  writeFile(manifestPath, `${manifest.join("\n")}\n`);
  afterManifest?.(bundleRoot, manifestPath);

  const archivePath = path.join(
    releaseAssetsDir,
    `codex-exec-loop-native-0.1.0-${config.targetTriple}.tar.gz`,
  );
  execFileSync("tar", [
    "--format=ustar",
    "-czf",
    archivePath,
    "-C",
    releaseAssetsDir,
    rootName,
  ]);
  writeChecksum(archivePath);
  fs.rmSync(bundleRoot, { recursive: true, force: true });
  return archivePath;
}

function writeTarOctal(header, start, length, value) {
  const body = value.toString(8).padStart(length - 1, "0");
  header.write(body, start, length - 1, "ascii");
  header[start + length - 1] = 0;
}

function tarHeader(name, size, type) {
  const header = Buffer.alloc(512);
  header.write(name, 0, 100, "ascii");
  writeTarOctal(header, 100, 8, type === "5" ? 0o755 : 0o644);
  writeTarOctal(header, 108, 8, 0);
  writeTarOctal(header, 116, 8, 0);
  writeTarOctal(header, 124, 12, size);
  writeTarOctal(header, 136, 12, 0);
  header.fill(0x20, 148, 156);
  header[156] = type.charCodeAt(0);
  header.write("ustar", 257, 5, "ascii");
  header[262] = 0;
  header.write("00", 263, 2, "ascii");
  let checksum = 0;
  for (const byte of header) checksum += byte;
  header.write(checksum.toString(8).padStart(6, "0"), 148, 6, "ascii");
  header[154] = 0;
  header[155] = 0x20;
  return header;
}

function runStage(releaseAssetsDir, outDir) {
  return execFileSync(
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
    { cwd: repoRoot, stdio: "pipe" },
  );
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

function rewriteVersionMetadata(bundleRoot, mutate) {
  const metadataPath = path.join(bundleRoot, "VERSION.txt");
  fs.writeFileSync(metadataPath, mutate(fs.readFileSync(metadataPath, "utf8")));
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

    runStage(releaseAssetsDir, outDir);

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
      assert(files.includes("THIRD_PARTY_NOTICES/Galmuri-OFL.txt"));
      assert(
        files.includes(
          "assets/app-server/skills/akra-planning-queue-mutation/SKILL.md",
        ),
      );
      assert(!files.some((file) => file.includes("node_modules")));
      assert(!files.some((file) => file.startsWith("assets/admin/")));

      const packageJson = JSON.parse(
        fs.readFileSync(
          path.join(outDir, "platforms", config.packageAlias, "package.json"),
          "utf8",
        ),
      );
      assert.equal(packageJson.name, "@refinedstone/akra");
      assert.equal(packageJson.version, `0.1.0-${config.packageVersionSuffix}`);
      assert.deepEqual(packageJson.os, [config.os]);
      assert.deepEqual(packageJson.cpu, [config.cpu]);
    }

    const mainPackageJson = JSON.parse(
      fs.readFileSync(path.join(outDir, "main", "package.json"), "utf8"),
    );
    assert.equal(mainPackageJson.name, "@refinedstone/akra");
    assert.equal(mainPackageJson.version, "0.1.0");
    assert.deepEqual(
      mainPackageJson.optionalDependencies,
      Object.fromEntries(
        PLATFORM_CONFIGS.map((config) => [
          config.packageAlias,
          `npm:@refinedstone/akra@0.1.0-${config.packageVersionSuffix}`,
        ]),
      ),
    );
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test("stage-npm-packages rejects a downloaded archive with a mismatched checksum", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "akra-stage-checksum-"));
  const releaseAssetsDir = path.join(tmp, "release-assets");
  const outDir = path.join(tmp, "publish");
  fs.mkdirSync(releaseAssetsDir, { recursive: true });

  try {
    const archives = PLATFORM_CONFIGS.map((config) =>
      createArchive(releaseAssetsDir, config),
    );
    fs.appendFileSync(archives[0], "tampered");

    assert.throws(
      () => runStage(releaseAssetsDir, outDir),
      /Release archive checksum mismatch/,
    );
    assert(!fs.existsSync(outDir));
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test("archive preflight enforces the exact VERSION.txt release contract", async (t) => {
  const config = PLATFORM_CONFIGS[0];
  const cases = [
    ["name", (body) => body.replace(/^name=.*$/m, "name=other-product")],
    ["version", (body) => body.replace(/^version=.*$/m, "version=9.9.9")],
    [
      "release tag",
      (body) => body.replace(/^release_tag=.*$/m, "release_tag=v9.9.9"),
    ],
    [
      "platform-swapped target",
      (body) =>
        body.replace(
          /^target=.*$/m,
          `target=${PLATFORM_CONFIGS[1].targetTriple}`,
        ),
    ],
    ["profile", (body) => body.replace(/^profile=.*$/m, "profile=debug")],
    [
      "binary",
      (body) => body.replace(/^binary=.*$/m, "binary=codex-exec-loop-native.exe"),
    ],
    ["launcher", (body) => body.replace(/^launcher=.*$/m, "launcher=akra.cmd")],
    ["missing key", (body) => body.replace(/^profile=.*\n/m, "")],
    ["unexpected key", (body) => `${body}channel=stable\n`],
    ["duplicate key", (body) => `${body}version=0.1.0\n`],
  ];

  for (const [label, mutate] of cases) {
    await t.test(label, async () => {
      const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "akra-version-contract-"));
      try {
        createArchive(tmp, config, {
          beforeManifest(bundleRoot) {
            rewriteVersionMetadata(bundleRoot, mutate);
          },
        });
        await assert.rejects(
          findAndVerifyArchive(tmp, "0.1.0", config.targetTriple),
          /Release VERSION\.txt/,
        );
      } finally {
        fs.rmSync(tmp, { recursive: true, force: true });
      }
    });
  }
});

test("archive preflight enforces exact and complete SHA256SUMS.txt records", async (t) => {
  const config = PLATFORM_CONFIGS[0];
  const cases = [
    ["empty manifest", () => "", /must be nonempty/],
    [
      "incomplete manifest",
      (body) => `${body.trimEnd().split("\n").slice(1).join("\n")}\n`,
      /cover every regular bundle file exactly once/,
    ],
    [
      "duplicate path",
      (body) => `${body}${body.split("\n")[0]}\n`,
      /repeats a path/,
    ],
    [
      "traversal path",
      (body) => `${body}${"0".repeat(64)}  ..\/escape\n`,
      /unsafe path/,
    ],
    [
      "invalid grammar",
      (body) => body.replace(/^([0-9a-f]{64})  /, "$1 "),
      /invalid grammar/,
    ],
    [
      "uppercase digest",
      (body) => `${body.slice(0, 64).toUpperCase()}${body.slice(64)}`,
      /invalid grammar/,
    ],
    [
      "missing final newline",
      (body) => body.trimEnd(),
      /canonical LF-terminated text/,
    ],
    [
      "CRLF records",
      (body) => body.replaceAll("\n", "\r\n"),
      /canonical LF-terminated text/,
    ],
    [
      "manifest self-reference",
      (body) => `${body}${"0".repeat(64)}  SHA256SUMS.txt\n`,
      /must not include itself/,
    ],
    [
      "nonexistent extra file",
      (body) => `${body}${"0".repeat(64)}  absent.txt\n`,
      /cover every regular bundle file exactly once/,
    ],
    [
      "digest mismatch",
      (body) => body.replace(/^[0-9a-f]/, body[0] === "0" ? "1" : "0"),
      /checksum mismatch/,
    ],
  ];

  for (const [label, mutate, expectedError] of cases) {
    await t.test(label, async () => {
      const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "akra-manifest-contract-"));
      try {
        createArchive(tmp, config, {
          afterManifest(_bundleRoot, manifestPath) {
            const body = fs.readFileSync(manifestPath, "utf8");
            fs.writeFileSync(manifestPath, mutate(body));
          },
        });
        await assert.rejects(
          findAndVerifyArchive(tmp, "0.1.0", config.targetTriple),
          expectedError,
        );
      } finally {
        fs.rmSync(tmp, { recursive: true, force: true });
      }
    });
  }
});

test("archive preflight requires the target binary and launcher", async (t) => {
  const config = PLATFORM_CONFIGS[0];
  for (const requiredPath of [config.binaryName, "akra"]) {
    await t.test(requiredPath, async () => {
      const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "akra-required-payload-"));
      try {
        createArchive(tmp, config, {
          beforeManifest(bundleRoot) {
            fs.rmSync(path.join(bundleRoot, requiredPath));
          },
        });
        await assert.rejects(
          findAndVerifyArchive(tmp, "0.1.0", config.targetTriple),
          new RegExp(`missing required file: ${requiredPath.replace(".", "\\.")}`),
        );
      } finally {
        fs.rmSync(tmp, { recursive: true, force: true });
      }
    });
  }
});

test("stage-npm-packages rejects unexpected duplicate payloads", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "akra-stage-duplicate-"));
  const releaseAssetsDir = path.join(tmp, "release-assets");
  const outDir = path.join(tmp, "publish");
  fs.mkdirSync(releaseAssetsDir, { recursive: true });

  try {
    const archives = PLATFORM_CONFIGS.map((config) =>
      createArchive(releaseAssetsDir, config),
    );
    fs.copyFileSync(
      archives[0],
      path.join(
        releaseAssetsDir,
        `duplicate-${PLATFORM_CONFIGS[0].targetTriple}.tar.gz`,
      ),
    );

    assert.throws(
      () => runStage(releaseAssetsDir, outDir),
      /missing, duplicate, nested, or unexpected payloads/,
    );
    assert(!fs.existsSync(outDir));
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test("stage-npm-packages rejects traversal and link archive members", () => {
  for (const unsafeKind of ["traversal", "symlink", "hardlink"]) {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), `akra-stage-${unsafeKind}-`));
    const releaseAssetsDir = path.join(tmp, "release-assets");
    const outDir = path.join(tmp, "publish");
    fs.mkdirSync(releaseAssetsDir, { recursive: true });

    try {
      const archives = PLATFORM_CONFIGS.map((config) =>
        createArchive(releaseAssetsDir, config),
      );
      const archivePath = archives[0];
      fs.rmSync(archivePath);
      const sourceName = `codex-exec-loop-native-0.1.0-${PLATFORM_CONFIGS[0].targetTriple}`;
      const source = path.join(tmp, sourceName);
      fs.mkdirSync(source);
      writeFile(path.join(source, "payload"), "unsafe\n");
      if (unsafeKind === "symlink") {
        fs.symlinkSync("payload", path.join(source, "linked"));
      } else if (unsafeKind === "hardlink") {
        fs.linkSync(path.join(source, "payload"), path.join(source, "linked"));
      }
      if (unsafeKind === "traversal") {
        // GNU tar's --transform is unavailable on macOS's BSD tar.  Build the
        // one malicious member directly so this security fixture describes the
        // same archive on every supported host.
        const payload = Buffer.from("unsafe\n");
        const paddedPayload = Buffer.alloc(Math.ceil(payload.length / 512) * 512);
        payload.copy(paddedPayload);
        const tar = Buffer.concat([
          tarHeader("../escape/payload", payload.length, "0"),
          paddedPayload,
          Buffer.alloc(1024),
        ]);
        fs.writeFileSync(archivePath, gzipSync(tar));
      } else {
        execFileSync("tar", [
          "--format=ustar",
          "-czf",
          archivePath,
          "-C",
          tmp,
          sourceName,
        ]);
      }
      writeChecksum(archivePath);

      assert.throws(
        () => runStage(releaseAssetsDir, outDir),
        /unsafe member path|escapes the expected root|non-file member/,
        unsafeKind,
      );
      assert(!fs.existsSync(outDir), unsafeKind);
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  }
});

test("stage-npm-packages rejects missing or empty required font notices", () => {
  for (const noticeBody of [null, ""]) {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "akra-stage-notice-"));
    const releaseAssetsDir = path.join(tmp, "release-assets");
    const outDir = path.join(tmp, "publish");
    fs.mkdirSync(releaseAssetsDir, { recursive: true });

    try {
      for (const [index, config] of PLATFORM_CONFIGS.entries()) {
        createArchive(
          releaseAssetsDir,
          config,
          index === 0 ? { noticeBody } : undefined,
        );
      }

      assert.throws(
        () => runStage(releaseAssetsDir, outDir),
        /Required third-party notice (?:is missing|must be a nonempty)/,
      );
      assert(!fs.existsSync(outDir));
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  }
});

test("verified extraction remains bound to the private archive snapshot", async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "akra-stage-snapshot-"));
  const releaseAssetsDir = path.join(tmp, "release-assets");
  const extractDir = path.join(tmp, "extract");
  fs.mkdirSync(releaseAssetsDir, { recursive: true });
  const config = PLATFORM_CONFIGS[0];

  let verified;
  try {
    const archivePath = createArchive(releaseAssetsDir, config);
    verified = await findAndVerifyArchive(
      releaseAssetsDir,
      "0.1.0",
      config.targetTriple,
    );
    fs.writeFileSync(archivePath, "replacement bytes that were never verified");

    const root = extractVerifiedArchive(verified, extractDir);
    assert.equal(
      fs.readFileSync(path.join(root, config.binaryName), "utf8"),
      "binary",
    );
    const manifestPath = path.join(root, "SHA256SUMS.txt");
    const reorderedManifest = `${fs
      .readFileSync(manifestPath, "utf8")
      .trimEnd()
      .split("\n")
      .reverse()
      .join("\n")}\n`;
    fs.writeFileSync(manifestPath, reorderedManifest);
    assert.throws(
      () =>
        verifyExtractedBundle(
          root,
          verified.contract,
          verified.manifest,
          verified.fileDigests,
        ),
      /files differ from the verified archive payloads/,
    );
  } finally {
    if (verified) {
      cleanupVerifiedReleaseAssets(new Map([[config.targetTriple, verified]]));
    }
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test("archive preflight rejects an oversized declared payload before extraction", async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "akra-stage-bomb-"));
  const config = PLATFORM_CONFIGS[0];
  const rootName = `codex-exec-loop-native-0.1.0-${config.targetTriple}`;
  const archivePath = path.join(tmp, `${rootName}.tar.gz`);
  const tar = Buffer.concat([
    tarHeader(`${rootName}/`, 0, "5"),
    tarHeader(`${rootName}/payload`, 1024 * 1024 * 1024 + 1, "0"),
    Buffer.alloc(1024),
  ]);
  fs.writeFileSync(archivePath, gzipSync(tar));
  writeChecksum(archivePath);

  try {
    await assert.rejects(
      findAndVerifyArchive(tmp, "0.1.0", config.targetTriple),
      /expands beyond the allowed size/,
    );
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test("archive preflight aborts an excessive gzip expansion ratio", async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "akra-stage-ratio-"));
  const config = PLATFORM_CONFIGS[0];
  const rootName = `codex-exec-loop-native-0.1.0-${config.targetTriple}`;
  const archivePath = path.join(tmp, `${rootName}.tar.gz`);
  const payloadBytes = 32 * 1024 * 1024;
  const tar = Buffer.concat([
    tarHeader(`${rootName}/`, 0, "5"),
    tarHeader(`${rootName}/payload`, payloadBytes, "0"),
    Buffer.alloc(payloadBytes),
    Buffer.alloc(1024),
  ]);
  fs.writeFileSync(archivePath, gzipSync(tar));
  writeChecksum(archivePath);

  try {
    await assert.rejects(
      findAndVerifyArchive(tmp, "0.1.0", config.targetTriple),
      /bounded tar stream or compression-ratio limit/,
    );
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});
