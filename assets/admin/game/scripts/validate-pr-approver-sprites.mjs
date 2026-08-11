import { readFile } from "node:fs/promises";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { PNG } from "pngjs";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(packageRoot, "../../..");
const spritePackRoot = resolve(
  repositoryRoot,
  "templates/admin/resources/pr_approver_sprite_pack",
);
const metadataPath = resolve(
  spritePackRoot,
  "pr-approver-sprite-metadata.json",
);
const metadata = JSON.parse(await readFile(metadataPath, "utf8"));
const failures = [];

const displayPath = (path) => relative(repositoryRoot, path).replaceAll("\\", "/");
const pixelOffset = (image, x, y) => (y * image.width + x) * 4;
const loadPng = async (path) => PNG.sync.read(await readFile(path));

const recordFailure = (message) => failures.push(message);

const validateMetadata = () => {
  const output = metadata.output.large;
  if (output.columns !== 6 || output.rows !== 4 || metadata.frames.length !== 24) {
    recordFailure("Metadata must describe exactly 24 frames in a 6x4 atlas.");
    return;
  }

  const seen = new Set();
  for (const frame of metadata.frames) {
    const expectedIndex = frame.row * output.columns + frame.col;
    const key = `${frame.row}:${frame.col}`;
    if (frame.atlas_index !== expectedIndex) {
      recordFailure(`${frame.name} has atlas_index ${frame.atlas_index}, expected ${expectedIndex}.`);
    }
    if (seen.has(key)) {
      recordFailure(`Metadata duplicates frame cell ${key}.`);
    }
    seen.add(key);
  }

  for (let row = 0; row < output.rows; row += 1) {
    for (let col = 0; col < output.columns; col += 1) {
      if (!seen.has(`${row}:${col}`)) {
        recordFailure(`Metadata omits frame cell ${row}:${col}.`);
      }
    }
  }

  const reserved = metadata.frames.filter((frame) => frame.usage === "reserved");
  if (reserved.length !== 1 || reserved[0].row !== 0 || reserved[0].col !== 1) {
    recordFailure("Only intake row 0 column 1 may be marked reserved.");
  }
};

const validateDimensions = (image, output, label) => {
  if (image.width !== output.atlas_width || image.height !== output.atlas_height) {
    recordFailure(
      `${label} is ${image.width}x${image.height}; expected ${output.atlas_width}x${output.atlas_height}.`,
    );
  }
};

const validateTransparentRgb = (image, label) => {
  let invalidCount = 0;
  const samples = [];
  for (let y = 0; y < image.height; y += 1) {
    for (let x = 0; x < image.width; x += 1) {
      const offset = pixelOffset(image, x, y);
      if (
        image.data[offset + 3] === 0 &&
        (image.data[offset] !== 0 ||
          image.data[offset + 1] !== 0 ||
          image.data[offset + 2] !== 0)
      ) {
        invalidCount += 1;
        if (samples.length < 4) {
          samples.push(`${x},${y}`);
        }
      }
    }
  }
  if (invalidCount > 0) {
    recordFailure(
      `${label} has ${invalidCount} transparent pixels with non-zero RGB (${samples.join("; ")}).`,
    );
  }
};

const validateCellsAndBorders = (
  image,
  output,
  margin,
  label,
  expectedFootBaseline = undefined,
) => {
  for (let row = 0; row < output.rows; row += 1) {
    for (let col = 0; col < output.columns; col += 1) {
      const cellLeft = col * output.frame_width;
      const cellTop = row * output.frame_height;
      let occupied = 0;
      let borderOccupied = 0;
      let lowestOccupiedY = -1;

      for (let localY = 0; localY < output.frame_height; localY += 1) {
        for (let localX = 0; localX < output.frame_width; localX += 1) {
          const alpha =
            image.data[
              pixelOffset(image, cellLeft + localX, cellTop + localY) + 3
            ];
          if (alpha === 0) {
            continue;
          }
          occupied += 1;
          lowestOccupiedY = Math.max(lowestOccupiedY, localY);
          if (
            localX < margin ||
            localY < margin ||
            localX >= output.frame_width - margin ||
            localY >= output.frame_height - margin
          ) {
            borderOccupied += 1;
          }
        }
      }

      if (occupied === 0) {
        recordFailure(`${label} frame ${row}:${col} is empty.`);
      }
      if (borderOccupied > 0) {
        recordFailure(
          `${label} frame ${row}:${col} has ${borderOccupied} occupied pixels in its ${margin}px border.`,
        );
      }
      if (
        expectedFootBaseline !== undefined &&
        lowestOccupiedY !== expectedFootBaseline
      ) {
        recordFailure(
          `${label} frame ${row}:${col} foot baseline is ${lowestOccupiedY}, expected ${expectedFootBaseline}.`,
        );
      }
    }
  }
};

const exteriorTransparencyMask = (image, cellLeft, cellTop, width, height) => {
  const mask = new Uint8Array(width * height);
  const queue = [];
  const enqueue = (x, y) => {
    const localOffset = y * width + x;
    if (
      mask[localOffset] !== 0 ||
      image.data[pixelOffset(image, cellLeft + x, cellTop + y) + 3] !== 0
    ) {
      return;
    }
    mask[localOffset] = 1;
    queue.push(localOffset);
  };

  for (let x = 0; x < width; x += 1) {
    enqueue(x, 0);
    enqueue(x, height - 1);
  }
  for (let y = 1; y < height - 1; y += 1) {
    enqueue(0, y);
    enqueue(width - 1, y);
  }

  for (let index = 0; index < queue.length; index += 1) {
    const localOffset = queue[index];
    const x = localOffset % width;
    const y = Math.floor(localOffset / width);
    if (x > 0) enqueue(x - 1, y);
    if (x + 1 < width) enqueue(x + 1, y);
    if (y > 0) enqueue(x, y - 1);
    if (y + 1 < height) enqueue(x, y + 1);
  }
  return mask;
};

const validateChromaFringe = (image, output, label) => {
  const [keyRed, keyGreen, keyBlue] = metadata.validation.chroma_reference_rgb;
  const threshold = metadata.validation.chroma_distance_threshold;
  const radius = metadata.validation.exterior_edge_radius;
  let fringeCount = 0;
  const samples = [];

  for (let row = 0; row < output.rows; row += 1) {
    for (let col = 0; col < output.columns; col += 1) {
      const cellLeft = col * output.frame_width;
      const cellTop = row * output.frame_height;
      const exterior = exteriorTransparencyMask(
        image,
        cellLeft,
        cellTop,
        output.frame_width,
        output.frame_height,
      );

      for (let y = 0; y < output.frame_height; y += 1) {
        for (let x = 0; x < output.frame_width; x += 1) {
          const offset = pixelOffset(image, cellLeft + x, cellTop + y);
          if (image.data[offset + 3] === 0) {
            continue;
          }
          let nearExterior = false;
          for (let dy = -radius; dy <= radius && !nearExterior; dy += 1) {
            for (let dx = -radius; dx <= radius; dx += 1) {
              if (Math.abs(dx) + Math.abs(dy) > radius) {
                continue;
              }
              const neighborX = x + dx;
              const neighborY = y + dy;
              if (
                neighborX >= 0 &&
                neighborY >= 0 &&
                neighborX < output.frame_width &&
                neighborY < output.frame_height &&
                exterior[neighborY * output.frame_width + neighborX] !== 0
              ) {
                nearExterior = true;
                break;
              }
            }
          }
          if (!nearExterior) {
            continue;
          }

          const redDelta = image.data[offset] - keyRed;
          const greenDelta = image.data[offset + 1] - keyGreen;
          const blueDelta = image.data[offset + 2] - keyBlue;
          const distance = Math.hypot(redDelta, greenDelta, blueDelta);
          if (distance < threshold) {
            fringeCount += 1;
            if (samples.length < 4) {
              samples.push(`${row}:${col}@${x},${y} d=${distance.toFixed(1)}`);
            }
          }
        }
      }
    }
  }

  if (fringeCount > 0) {
    recordFailure(
      `${label} has ${fringeCount} chroma-like exterior edge pixels (${samples.join("; ")}).`,
    );
  }
};

const validateByteEquality = (runtimeBytes, archiveBytes, label) => {
  if (!runtimeBytes.equals(archiveBytes)) {
    recordFailure(`${label} runtime and archive PNG bytes differ.`);
  }
};

const validateExactHalf = (large, half) => {
  if (half.width * 2 !== large.width || half.height * 2 !== large.height) {
    recordFailure("Half atlas dimensions are not exactly half of the large atlas.");
    return;
  }
  let mismatchCount = 0;
  const samples = [];
  for (let y = 0; y < half.height; y += 1) {
    for (let x = 0; x < half.width; x += 1) {
      const largeOffset = pixelOffset(large, x * 2, y * 2);
      const halfOffset = pixelOffset(half, x, y);
      let matches = true;
      for (let channel = 0; channel < 4; channel += 1) {
        if (half.data[halfOffset + channel] !== large.data[largeOffset + channel]) {
          matches = false;
          break;
        }
      }
      if (!matches) {
        mismatchCount += 1;
        if (samples.length < 4) {
          samples.push(`${x},${y}`);
        }
      }
    }
  }
  if (mismatchCount > 0) {
    recordFailure(
      `Half atlas has ${mismatchCount} pixels that violate half(x,y)=large(2x,2y) (${samples.join("; ")}).`,
    );
  }
};

const validatePreview = (large, preview) => {
  const [backgroundRed, backgroundGreen, backgroundBlue] =
    metadata.validation.preview_background_rgb;
  if (preview.width !== large.width || preview.height !== large.height) {
    recordFailure("Dark preview dimensions do not match the large atlas.");
    return;
  }
  let mismatchCount = 0;
  for (let offset = 0; offset < large.data.length; offset += 4) {
    const alpha = large.data[offset + 3] / 255;
    const expected = [
      Math.round(large.data[offset] * alpha + backgroundRed * (1 - alpha)),
      Math.round(large.data[offset + 1] * alpha + backgroundGreen * (1 - alpha)),
      Math.round(large.data[offset + 2] * alpha + backgroundBlue * (1 - alpha)),
    ];
    if (
      preview.data[offset] !== expected[0] ||
      preview.data[offset + 1] !== expected[1] ||
      preview.data[offset + 2] !== expected[2] ||
      preview.data[offset + 3] !== 255
    ) {
      mismatchCount += 1;
    }
  }
  if (mismatchCount > 0) {
    recordFailure(`Dark preview has ${mismatchCount} pixels that drift from the large atlas.`);
  }
};

validateMetadata();

const largeRuntimePath = resolve(repositoryRoot, metadata.output.large.runtime_file);
const largeArchivePath = resolve(spritePackRoot, metadata.output.large.archive_file);
const halfRuntimePath = resolve(repositoryRoot, metadata.output.half.runtime_file);
const halfArchivePath = resolve(spritePackRoot, metadata.output.half.archive_file);
const alphaSourcePath = resolve(spritePackRoot, metadata.source.alpha_file);
const chromaSourcePath = resolve(spritePackRoot, metadata.source.chroma_file);
const previewPath = resolve(spritePackRoot, metadata.output.preview_file);

const [
  largeRuntimeBytes,
  largeArchiveBytes,
  halfRuntimeBytes,
  halfArchiveBytes,
  alphaSource,
  chromaSource,
  preview,
] = await Promise.all([
  readFile(largeRuntimePath),
  readFile(largeArchivePath),
  readFile(halfRuntimePath),
  readFile(halfArchivePath),
  loadPng(alphaSourcePath),
  loadPng(chromaSourcePath),
  loadPng(previewPath),
]);
const large = PNG.sync.read(largeRuntimeBytes);
const half = PNG.sync.read(halfRuntimeBytes);

validateByteEquality(largeRuntimeBytes, largeArchiveBytes, "Large atlas");
validateByteEquality(halfRuntimeBytes, halfArchiveBytes, "Half atlas");
validateDimensions(large, metadata.output.large, "Large atlas");
validateDimensions(half, metadata.output.half, "Half atlas");
if (
  alphaSource.width !== metadata.source.width ||
  alphaSource.height !== metadata.source.height ||
  chromaSource.width !== metadata.source.width ||
  chromaSource.height !== metadata.source.height
) {
  recordFailure("Archived source dimensions do not match metadata.");
}
validateTransparentRgb(alphaSource, "Alpha source");
validateTransparentRgb(large, "Large atlas");
validateTransparentRgb(half, "Half atlas");
validateCellsAndBorders(
  large,
  metadata.output.large,
  metadata.normalization.outer_transparent_margin,
  "Large atlas",
  metadata.normalization.foot_baseline_y,
);
validateCellsAndBorders(
  half,
  metadata.output.half,
  metadata.normalization.outer_transparent_margin / 2,
  "Half atlas",
);
validateChromaFringe(large, metadata.output.large, "Large atlas");
validateChromaFringe(half, metadata.output.half, "Half atlas");
validateExactHalf(large, half);
validatePreview(large, preview);

if (failures.length > 0) {
  throw new Error(
    `PR approver sprite validation failed:\n- ${failures.join("\n- ")}`,
  );
}

console.log(
  `Validated ${metadata.frames.length} occupied PR approver frames, transparent borders, alpha hygiene, chroma edges, archive parity, and exact-nearest half atlas.`,
);
console.log(`Runtime: ${displayPath(largeRuntimePath)}, ${displayPath(halfRuntimePath)}`);
