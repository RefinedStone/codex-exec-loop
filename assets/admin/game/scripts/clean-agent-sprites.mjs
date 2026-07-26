import { readFile, writeFile } from "node:fs/promises";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { PNG } from "pngjs";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(packageRoot, "../../..");
const spritePackRoot = resolve(
  repositoryRoot,
  "templates/admin/resources/gamebaljeonguk_sprite_pack",
);
const writeChanges = process.argv.includes("--write");
const checkOnly = process.argv.includes("--check");

if (writeChanges === checkOnly) {
  throw new Error("Pass exactly one of --write or --check.");
}

const runtimeAtlasPairs = [
  {
    runtime: resolve(
      repositoryRoot,
      "assets/admin/graphics/gamebaljeonguk_atlas_128x192.png",
    ),
    archive: resolve(spritePackRoot, "gamebaljeonguk_atlas_128x192.png"),
  },
  {
    runtime: resolve(
      repositoryRoot,
      "assets/admin/graphics/gamebaljeonguk_atlas_64x96.png",
    ),
    archive: resolve(spritePackRoot, "gamebaljeonguk_atlas_64x96.png"),
  },
];
const standaloneSprites = [
  resolve(spritePackRoot, "gamebaljeonguk_original_transparent.png"),
  resolve(spritePackRoot, "$gamebaljeonguk_planner.png"),
  resolve(spritePackRoot, "$gamebaljeonguk_coffee_addict.png"),
];

const WHITE_MATTE_MIN_BRIGHTNESS = 150;
const WHITE_MATTE_MAX_CHROMA = 28;
const CHECKER_SIZE = 32;
const CHECKER_LIGHT = 210;
const CHECKER_DARK = 160;

const pixelOffset = (image, x, y) => (y * image.width + x) * 4;

const hasTransparentNeighbor = (image, source, x, y) => {
  const neighbors = [
    [x - 1, y],
    [x + 1, y],
    [x, y - 1],
    [x, y + 1],
  ];
  return neighbors.some(([neighborX, neighborY]) => {
    if (
      neighborX < 0 ||
      neighborY < 0 ||
      neighborX >= image.width ||
      neighborY >= image.height
    ) {
      return true;
    }
    return source[pixelOffset(image, neighborX, neighborY) + 3] === 0;
  });
};

const recoverWhiteMattePixel = (red, green, blue) => {
  const minimum = Math.min(red, green, blue);
  const alpha = 255 - minimum;
  if (alpha === 0) {
    return [0, 0, 0, 0];
  }
  const recoverChannel = (channel) =>
    Math.max(0, Math.min(255, Math.round(((channel - minimum) * 255) / alpha)));
  return [
    recoverChannel(red),
    recoverChannel(green),
    recoverChannel(blue),
    alpha,
  ];
};

const cleanWhiteMatte = (image) => {
  const source = Buffer.from(image.data);
  let decontaminatedPixels = 0;
  let normalizedTransparentPixels = 0;

  for (let y = 0; y < image.height; y += 1) {
    for (let x = 0; x < image.width; x += 1) {
      const offset = pixelOffset(image, x, y);
      const red = source[offset];
      const green = source[offset + 1];
      const blue = source[offset + 2];
      const alpha = source[offset + 3];

      if (alpha === 0) {
        if (red !== 0 || green !== 0 || blue !== 0) {
          image.data.fill(0, offset, offset + 3);
          normalizedTransparentPixels += 1;
        }
        continue;
      }
      if (alpha !== 255 || !hasTransparentNeighbor(image, source, x, y)) {
        continue;
      }

      const maximum = Math.max(red, green, blue);
      const minimum = Math.min(red, green, blue);
      const brightness = (red + green + blue) / 3;
      if (
        brightness < WHITE_MATTE_MIN_BRIGHTNESS ||
        maximum - minimum > WHITE_MATTE_MAX_CHROMA
      ) {
        continue;
      }

      image.data.set(recoverWhiteMattePixel(red, green, blue), offset);
      decontaminatedPixels += 1;
    }
  }

  return { decontaminatedPixels, normalizedTransparentPixels };
};

const decode = async (path) => PNG.sync.read(await readFile(path));
const encode = (image) => PNG.sync.write(image, { colorType: 6 });

const clean = async (path) => {
  const image = await decode(path);
  const result = cleanWhiteMatte(image);
  return { image, result, encoded: encode(image) };
};

const renderCheckerboard = (source) => {
  const preview = new PNG({ width: source.width, height: source.height });
  for (let y = 0; y < source.height; y += 1) {
    for (let x = 0; x < source.width; x += 1) {
      const offset = pixelOffset(source, x, y);
      const checker =
        (Math.floor(x / CHECKER_SIZE) + Math.floor(y / CHECKER_SIZE)) % 2 === 0
          ? CHECKER_LIGHT
          : CHECKER_DARK;
      const alpha = source.data[offset + 3] / 255;
      for (let channel = 0; channel < 3; channel += 1) {
        preview.data[offset + channel] = Math.round(
          source.data[offset + channel] * alpha + checker * (1 - alpha),
        );
      }
      preview.data[offset + 3] = 255;
    }
  }
  return encode(preview);
};

const summaries = [];
let pendingChanges = 0;
let cleanedOriginal = null;
let cleanedLargeAtlas = null;

for (const pair of runtimeAtlasPairs) {
  const cleaned = await clean(pair.runtime);
  const archiveBytes = await readFile(pair.archive);
  const runtimeBytes = await readFile(pair.runtime);
  const runtimeChanged = !runtimeBytes.equals(cleaned.encoded);
  const archiveChanged = !archiveBytes.equals(cleaned.encoded);
  pendingChanges += Number(runtimeChanged) + Number(archiveChanged);
  summaries.push({
    file: pair.runtime,
    ...cleaned.result,
    changed: runtimeChanged,
  });
  summaries.push({
    file: pair.archive,
    mirroredFromRuntime: true,
    changed: archiveChanged,
  });
  if (writeChanges) {
    await writeFile(pair.runtime, cleaned.encoded);
    await writeFile(pair.archive, cleaned.encoded);
  }
  if (pair.runtime.includes("128x192")) {
    cleanedLargeAtlas = PNG.sync.read(cleaned.encoded);
  }
}

for (const path of standaloneSprites) {
  const cleaned = await clean(path);
  const current = await readFile(path);
  const changed = !current.equals(cleaned.encoded);
  pendingChanges += Number(changed);
  summaries.push({ file: path, ...cleaned.result, changed });
  if (writeChanges) {
    await writeFile(path, cleaned.encoded);
  }
  if (path.endsWith("gamebaljeonguk_original_transparent.png")) {
    cleanedOriginal = PNG.sync.read(cleaned.encoded);
  }
}

const previews = [
  {
    path: resolve(spritePackRoot, "preview_atlas_checkerboard.png"),
    bytes: renderCheckerboard(cleanedLargeAtlas),
  },
  {
    path: resolve(spritePackRoot, "preview_original_checkerboard.png"),
    bytes: renderCheckerboard(cleanedOriginal),
  },
];
for (const preview of previews) {
  const current = await readFile(preview.path);
  const changed = !current.equals(preview.bytes);
  pendingChanges += Number(changed);
  summaries.push({ file: preview.path, regenerated: true, changed });
  if (writeChanges) {
    await writeFile(preview.path, preview.bytes);
  }
}

for (const summary of summaries) {
  const details = [
    summary.decontaminatedPixels !== undefined
      ? `decontaminated=${summary.decontaminatedPixels}`
      : null,
    summary.normalizedTransparentPixels !== undefined
      ? `transparentRgb=${summary.normalizedTransparentPixels}`
      : null,
    summary.mirroredFromRuntime ? "mirrored" : null,
    summary.regenerated ? "preview" : null,
  ]
    .filter(Boolean)
    .join(" ");
  console.log(
    `${summary.changed ? "change" : "clean "} ${relative(repositoryRoot, summary.file)} ${details}`.trim(),
  );
}

if (checkOnly && pendingChanges > 0) {
  throw new Error(
    `${pendingChanges} sprite asset(s) need cleanup. Run npm run sprites:clean.`,
  );
}
