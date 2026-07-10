import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";

const options = Object.fromEntries(
  process.argv.slice(2).map((argument) => {
    const [key, ...value] = argument.split("=");
    return [key.replace(/^--/, ""), value.join("=")];
  }),
);

for (const name of [
  "input",
  "output",
  "source-cli",
  "generated-at",
  "schema-id",
  "schema-version",
  "description",
  "source-artifact",
]) {
  if (!options[name]) throw new Error(`--${name}=... is required`);
}

const source = await readFile(options.input);
let schema = sortJson(JSON.parse(source.toString("utf8")));
const generatedContentSha256 = createHash("sha256")
  .update(JSON.stringify(schema))
  .digest("hex");

function sortJson(value) {
  if (Array.isArray(value)) return value.map(sortJson);
  if (value === null || typeof value !== "object") return value;
  return Object.fromEntries(
    Object.keys(value)
      .sort()
      .map((key) => [key, sortJson(value[key])]),
  );
}

const exactInteger = {
  int64Min: "__AKRA_JSON_INT64_MIN__",
  int64Max: "__AKRA_JSON_INT64_MAX__",
  uint64Max: "__AKRA_JSON_UINT64_MAX__",
};
const formatBounds = {
  uint: [0, exactInteger.uint64Max],
  uint64: [0, exactInteger.uint64Max],
  uint32: [0, 4294967295],
  uint16: [0, 65535],
  uint8: [0, 255],
  int64: [exactInteger.int64Min, exactInteger.int64Max],
  int32: [-2147483648, 2147483647],
};

const hasIntegerType = (node) =>
  node?.type === "integer" ||
  (Array.isArray(node?.type) && node.type.includes("integer"));

const normalizedMinimum = (existing, typeMinimum) => {
  if (!Number.isSafeInteger(existing)) return typeMinimum;
  if (typeof typeMinimum !== "number") return existing;
  return Math.max(existing, typeMinimum);
};

const normalizedMaximum = (existing, typeMaximum) => {
  if (!Number.isSafeInteger(existing)) return typeMaximum;
  if (typeof typeMaximum !== "number") return existing;
  return Math.min(existing, typeMaximum);
};

const normalizeIntegerBounds = (node, path = []) => {
  if (!node || typeof node !== "object") return;
  if (hasIntegerType(node) && Object.hasOwn(formatBounds, node.format)) {
    const [minimum, maximum] = formatBounds[node.format];
    node.minimum = normalizedMinimum(node.minimum, minimum);
    node.maximum = normalizedMaximum(node.maximum, maximum);
    if (path.join("/") === "definitions/ThreadRollbackParams/properties/numTurns") {
      node.minimum = Math.max(node.minimum, 1);
    }
  }
  for (const [key, value] of Object.entries(node)) {
    normalizeIntegerBounds(value, [...path, key]);
  }
};

normalizeIntegerBounds(schema);
Object.assign(schema, {
  $id: options["schema-id"],
  description: options.description,
  version: options["schema-version"],
  "x-generated-from": "codex app-server generate-json-schema --experimental",
  "x-generated-artifact": options["source-artifact"],
  "x-source-cli-version": options["source-cli"],
  "x-generated-at": options["generated-at"],
  "x-generated-content-sha256": generatedContentSha256,
});
schema = sortJson(schema);

let output = `${JSON.stringify(schema, null, 2)}\n`;
for (const [sentinel, value] of [
  [exactInteger.int64Min, "-9223372036854775808"],
  [exactInteger.int64Max, "9223372036854775807"],
  [exactInteger.uint64Max, "18446744073709551615"],
]) {
  output = output.replaceAll(`"${sentinel}"`, value);
}
if (output.includes("__AKRA_JSON_")) {
  throw new Error("integer sentinel remained after schema serialization");
}

await writeFile(options.output, output, { mode: 0o644 });
