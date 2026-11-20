// Rust가 묶는 정적 자산의 삭제/이동도 Node-only gate에서 찾는다.
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

function assertEmbeddedAssets(sourcePath) {
  const source = readFileSync(sourcePath, "utf8");
  const declarations = [...source.matchAll(/bundled_asset!\s*\(\s*\w+\s*,\s*\w+\s*,\s*"([^"]+)"\s*,?\s*\)/g)];
  const count = [...source.matchAll(/bundled_asset!\s*\(/g)].length;
  assert.ok(count > 0, "the Rust asset manifest must contain declarations");
  assert.equal(declarations.length, count, "every bundled_asset declaration must have a checked literal path");
  for (const [, relativePath] of declarations) {
    const assetPath = resolve(dirname(sourcePath), relativePath);
    assert.ok(statSync(assetPath).isFile(), `embedded asset is not a regular file: ${relativePath}`);
  }
}

test("every Rust-embedded Admin asset exists before packaging", () => {
  const manifest = fileURLToPath(new URL("../../../src/adapter/inbound/admin_api/static_assets.rs", import.meta.url));
  assertEmbeddedAssets(manifest);
});

test("removing or moving an embedded asset fails the Node gate", () => {
  const directory = mkdtempSync(join(tmpdir(), "akra-embedded-assets-"));
  try {
    const manifest = join(directory, "static_assets.rs");
    const asset = join(directory, "admin-shell.js");
    writeFileSync(manifest, 'bundled_asset!(SHELL, SHELL_ETAG, "admin-shell.js");');
    writeFileSync(asset, "void 0;");
    assert.doesNotThrow(() => assertEmbeddedAssets(manifest));
    rmSync(asset);
    assert.throws(() => assertEmbeddedAssets(manifest), /ENOENT/);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("an unsupported embedded path declaration cannot silently escape validation", () => {
  const directory = mkdtempSync(join(tmpdir(), "akra-embedded-manifest-"));
  try {
    const manifest = join(directory, "static_assets.rs");
    writeFileSync(manifest, "bundled_asset!(SHELL, SHELL_ETAG, SOME_OTHER_PATH);");
    assert.throws(() => assertEmbeddedAssets(manifest), /checked literal path/);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
