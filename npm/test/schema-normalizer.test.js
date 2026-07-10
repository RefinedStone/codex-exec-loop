import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const normalizer = path.resolve(
  packageRoot,
  "..",
  "scripts",
  "normalize_codex_app_server_schema.mjs",
);

test("schema normalizer is deterministic across generated object key order", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "akra-schema-normalizer-"));
  try {
    const firstInput = path.join(root, "first.json");
    const secondInput = path.join(root, "second.json");
    const firstOutput = path.join(root, "first-output.json");
    const secondOutput = path.join(root, "second-output.json");
    fs.writeFileSync(
      firstInput,
      JSON.stringify({
        title: "Fixture",
        definitions: {
          Zed: { type: "string", description: "last" },
          Wide: { maximum: 70_000, minimum: -1, format: "uint16", type: "integer" },
          Alpha: { maximum: 2, minimum: 1, format: "uint32", type: "integer" },
          ThreadRollbackParams: {
            properties: { numTurns: { type: "integer", format: "uint32" } },
            type: "object",
          },
        },
        type: "object",
      }),
    );
    fs.writeFileSync(
      secondInput,
      JSON.stringify({
        type: "object",
        definitions: {
          Alpha: { type: "integer", format: "uint32", minimum: 1, maximum: 2 },
          Wide: { type: "integer", format: "uint16", minimum: -1, maximum: 70_000 },
          ThreadRollbackParams: {
            type: "object",
            properties: { numTurns: { format: "uint32", type: "integer" } },
          },
          Zed: { description: "last", type: "string" },
        },
        title: "Fixture",
      }),
    );

    normalize(firstInput, firstOutput);
    normalize(secondInput, secondOutput);

    const first = fs.readFileSync(firstOutput, "utf8");
    const second = fs.readFileSync(secondOutput, "utf8");
    assert.equal(first, second);
    const schema = JSON.parse(first);
    assert.equal(schema.definitions.Alpha.minimum, 1);
    assert.equal(schema.definitions.Alpha.maximum, 2);
    assert.equal(schema.definitions.Wide.minimum, 0);
    assert.equal(schema.definitions.Wide.maximum, 65535);
    assert.equal(
      schema.definitions.ThreadRollbackParams.properties.numTurns.minimum,
      1,
    );
    assert.match(schema["x-generated-content-sha256"], /^[a-f0-9]{64}$/);
    assert.deepEqual(Object.keys(schema), [...Object.keys(schema)].sort());
    assert.deepEqual(
      Object.keys(schema.definitions),
      [...Object.keys(schema.definitions)].sort(),
    );
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

function normalize(input, output) {
  const result = spawnSync(
    process.execPath,
    [
      normalizer,
      `--input=${input}`,
      `--output=${output}`,
      "--source-cli=codex-cli test",
      "--generated-at=2026-07-10",
      "--schema-id=urn:akra:test",
      "--schema-version=test",
      "--description=deterministic fixture",
      "--source-artifact=fixture.json",
    ],
    { encoding: "utf8" },
  );
  assert.equal(result.status, 0, result.stderr);
}
