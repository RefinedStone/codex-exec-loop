import { readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
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
const alphaSourcePath = resolve(spritePackRoot, metadata.source.alpha_file);
const alphaSource = PNG.sync.read(await readFile(alphaSourcePath));

const pixelOffset = (image, x, y) => (y * image.width + x) * 4;

const assertSourceContract = () => {
  if (
    alphaSource.width !== metadata.source.width ||
    alphaSource.height !== metadata.source.height
  ) {
    throw new Error(
      `Unexpected alpha source size ${alphaSource.width}x${alphaSource.height}.`,
    );
  }
  if (
    metadata.source.column_edges.length !== metadata.output.large.columns + 1 ||
    metadata.source.visual_row_bounds.length !== metadata.output.large.rows
  ) {
    throw new Error("Source segmentation does not match the 6x4 output grid.");
  }
};

const frameSourceBounds = (row, col) => {
  const [top, bottom] = metadata.source.visual_row_bounds[row];
  const left = metadata.source.column_edges[col];
  const right = metadata.source.column_edges[col + 1];
  return { left, top, width: right - left, height: bottom - top };
};

const lowerBodyAnchorX = ({ left, top, width, height }) => {
  const bandHeight = metadata.normalization.body_anchor.lower_band_height;
  const bandTop = Math.max(top, top + height - bandHeight);
  let weightedX = 0;
  let totalAlpha = 0;

  for (let y = bandTop; y < top + height; y += 1) {
    for (let localX = 0; localX < width; localX += 1) {
      const sourceOffset = pixelOffset(alphaSource, left + localX, y);
      const alpha = alphaSource.data[sourceOffset + 3];
      weightedX += (localX + 0.5) * alpha;
      totalAlpha += alpha;
    }
  }

  if (totalAlpha === 0) {
    throw new Error(`Cannot derive a lower-body anchor at ${left},${top}.`);
  }
  return weightedX / totalAlpha;
};

const resizedFootY = (source, resizedWidth, resizedHeight) => {
  let lowestOccupiedY = -1;
  for (let outputY = 0; outputY < resizedHeight; outputY += 1) {
    const sourceY = Math.min(
      source.height - 1,
      Math.floor(((outputY + 0.5) * source.height) / resizedHeight),
    );
    for (let outputX = 0; outputX < resizedWidth; outputX += 1) {
      const sourceX = Math.min(
        source.width - 1,
        Math.floor(((outputX + 0.5) * source.width) / resizedWidth),
      );
      if (
        alphaSource.data[
          pixelOffset(
            alphaSource,
            source.left + sourceX,
            source.top + sourceY,
          ) + 3
        ] !== 0
      ) {
        lowestOccupiedY = outputY;
        break;
      }
    }
  }
  if (lowestOccupiedY < 0) {
    throw new Error(`Cannot derive a foot baseline at ${source.left},${source.top}.`);
  }
  return lowestOccupiedY;
};

const copyNormalizedFrame = (atlas, row, col) => {
  const source = frameSourceBounds(row, col);
  const output = metadata.output.large;
  const normalization = metadata.normalization;
  const resizedWidth = normalization.resized_source_cell_width;
  const resizedHeight = normalization.resized_source_cell_height;
  const anchorX = lowerBodyAnchorX(source);
  const destinationLeft = Math.round(
    col * output.frame_width +
      normalization.body_center_x -
      (anchorX * resizedWidth) / source.width,
  );
  const footY = resizedFootY(source, resizedWidth, resizedHeight);
  const destinationTop =
    row * output.frame_height +
    normalization.foot_baseline_y -
    footY;
  const cellLeft = col * output.frame_width;
  const cellTop = row * output.frame_height;
  const interiorLeft = cellLeft + normalization.outer_transparent_margin;
  const interiorTop = cellTop + normalization.outer_transparent_margin;
  const interiorRight =
    cellLeft + output.frame_width - normalization.outer_transparent_margin;
  const interiorBottom =
    cellTop + output.frame_height - normalization.outer_transparent_margin;

  for (let outputY = 0; outputY < resizedHeight; outputY += 1) {
    const sourceY = Math.min(
      source.height - 1,
      Math.floor(((outputY + 0.5) * source.height) / resizedHeight),
    );
    const destinationY = destinationTop + outputY;
    if (destinationY < interiorTop || destinationY >= interiorBottom) {
      continue;
    }

    for (let outputX = 0; outputX < resizedWidth; outputX += 1) {
      const destinationX = destinationLeft + outputX;
      if (destinationX < interiorLeft || destinationX >= interiorRight) {
        continue;
      }
      const sourceX = Math.min(
        source.width - 1,
        Math.floor(((outputX + 0.5) * source.width) / resizedWidth),
      );
      const sourceOffset = pixelOffset(
        alphaSource,
        source.left + sourceX,
        source.top + sourceY,
      );
      const alpha = alphaSource.data[sourceOffset + 3];
      if (alpha === 0) {
        continue;
      }

      const destinationOffset = pixelOffset(atlas, destinationX, destinationY);
      atlas.data[destinationOffset] = alphaSource.data[sourceOffset];
      atlas.data[destinationOffset + 1] = alphaSource.data[sourceOffset + 1];
      atlas.data[destinationOffset + 2] = alphaSource.data[sourceOffset + 2];
      atlas.data[destinationOffset + 3] = alpha;
    }
  }
};

const buildLargeAtlas = () => {
  const output = metadata.output.large;
  const atlas = new PNG({
    width: output.atlas_width,
    height: output.atlas_height,
    colorType: 6,
    inputColorType: 6,
  });
  atlas.data.fill(0);

  for (let row = 0; row < output.rows; row += 1) {
    for (let col = 0; col < output.columns; col += 1) {
      copyNormalizedFrame(atlas, row, col);
    }
  }
  return atlas;
};

const buildHalfAtlas = (largeAtlas) => {
  const output = metadata.output.half;
  const atlas = new PNG({
    width: output.atlas_width,
    height: output.atlas_height,
    colorType: 6,
    inputColorType: 6,
  });

  for (let y = 0; y < atlas.height; y += 1) {
    for (let x = 0; x < atlas.width; x += 1) {
      const sourceOffset = pixelOffset(largeAtlas, x * 2, y * 2);
      const destinationOffset = pixelOffset(atlas, x, y);
      largeAtlas.data.copy(
        atlas.data,
        destinationOffset,
        sourceOffset,
        sourceOffset + 4,
      );
    }
  }
  return atlas;
};

const buildDarkPreview = (largeAtlas) => {
  const [backgroundRed, backgroundGreen, backgroundBlue] =
    metadata.validation.preview_background_rgb;
  const preview = new PNG({
    width: largeAtlas.width,
    height: largeAtlas.height,
    colorType: 6,
    inputColorType: 6,
  });

  for (let offset = 0; offset < largeAtlas.data.length; offset += 4) {
    const alpha = largeAtlas.data[offset + 3] / 255;
    preview.data[offset] = Math.round(
      largeAtlas.data[offset] * alpha + backgroundRed * (1 - alpha),
    );
    preview.data[offset + 1] = Math.round(
      largeAtlas.data[offset + 1] * alpha + backgroundGreen * (1 - alpha),
    );
    preview.data[offset + 2] = Math.round(
      largeAtlas.data[offset + 2] * alpha + backgroundBlue * (1 - alpha),
    );
    preview.data[offset + 3] = 255;
  }
  return preview;
};

const writeAtlasPair = async (atlas, output) => {
  const bytes = PNG.sync.write(atlas);
  await Promise.all([
    writeFile(resolve(spritePackRoot, output.archive_file), bytes),
    writeFile(resolve(repositoryRoot, output.runtime_file), bytes),
  ]);
};

assertSourceContract();
const largeAtlas = buildLargeAtlas();
const halfAtlas = buildHalfAtlas(largeAtlas);
const preview = buildDarkPreview(largeAtlas);

await Promise.all([
  writeAtlasPair(largeAtlas, metadata.output.large),
  writeAtlasPair(halfAtlas, metadata.output.half),
  writeFile(
    resolve(spritePackRoot, metadata.output.preview_file),
    PNG.sync.write(preview),
  ),
]);

console.log(
  `Prepared ${metadata.frames.length} PR approver frames with a shared body anchor and baseline.`,
);
