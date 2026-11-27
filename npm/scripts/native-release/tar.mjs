// allow: SIZE_OK - One incremental bounded ustar state machine and its stream driver.
import { createHash } from "node:crypto";
import fs from "node:fs";
import { pipeline } from "node:stream/promises";
import { Writable } from "node:stream";
import { createGunzip } from "node:zlib";
import { SAFE_MEMBER_PATH, UTF8_DECODER, MAX_VERSION_METADATA_BYTES, MAX_BUNDLE_MANIFEST_BYTES, verifyBundleRecords } from "./contract.mjs";
import { regularFileStat, descriptorRegularFileStat, sameFileVersion, openReadOnlyNoFollow, TAR_TIMEOUT_MS } from "./files.mjs";
const MAX_ARCHIVE_MEMBERS = 4096;
const MAX_EXPANDED_BYTES = 1024 * 1024 * 1024;
const MAX_TAR_STREAM_BYTES =
  MAX_EXPANDED_BYTES + MAX_ARCHIVE_MEMBERS * 1024 + 1024 * 1024;
const MAX_COMPRESSION_RATIO = 256n;
const COMPRESSION_RATIO_ALLOWANCE_BYTES = 16n * 1024n * 1024n;
const TAR_BLOCK_BYTES = 512;

function parseTarOctal(field, label) {
  const nul = field.indexOf(0);
  const body = field
    .subarray(0, nul === -1 ? field.length : nul)
    .toString("ascii")
    .trim();
  if (!/^[0-7]+$/.test(body)) {
    throw new Error(`Release archive has an invalid ${label} field`);
  }
  const value = Number.parseInt(body, 8);
  if (!Number.isSafeInteger(value)) {
    throw new Error(`Release archive ${label} exceeds the supported integer range`);
  }
  return value;
}

function parseTarString(field, label) {
  const nul = field.indexOf(0);
  const body = field.subarray(0, nul === -1 ? field.length : nul);
  if (nul !== -1 && field.subarray(nul + 1).some((byte) => byte !== 0)) {
    throw new Error(`Release archive has ambiguous ${label} padding`);
  }
  try {
    return UTF8_DECODER.decode(body);
  } catch {
    throw new Error(`Release archive has a non-UTF-8 ${label}`);
  }
}

function verifyTarHeaderChecksum(header) {
  const expected = parseTarOctal(header.subarray(148, 156), "header checksum");
  let actual = 0;
  for (let index = 0; index < header.length; index += 1) {
    actual += index >= 148 && index < 156 ? 0x20 : header[index];
  }
  if (actual !== expected) {
    throw new Error("Release archive has an invalid tar header checksum");
  }
}

class TarStreamInspector extends Writable {
  constructor(contract, maximumStreamBytes) {
    super();
    this.contract = contract;
    this.expectedRoot = contract.expectedRoot;
    this.maximumStreamBytes = maximumStreamBytes;
    this.header = Buffer.alloc(TAR_BLOCK_BYTES);
    this.headerBytes = 0;
    this.payloadBytes = 0;
    this.paddingBytes = 0;
    this.streamBytes = 0;
    this.expandedBytes = 0;
    this.memberCount = 0;
    this.zeroBlocks = 0;
    this.finished = false;
    this.rootSeen = false;
    this.uniqueNames = new Set();
    this.fileDigests = new Map();
    this.metadataBodies = new Map();
    this.currentFile = null;
    this.pendingLongName = null;
    this.manifest = null;
  }

  _write(chunk, _encoding, callback) {
    try {
      this.streamBytes += chunk.length;
      if (this.streamBytes > this.maximumStreamBytes) {
        throw new Error(
          `Release archive exceeds the bounded tar stream or compression-ratio limit (${this.maximumStreamBytes} bytes)`,
        );
      }
      this.consume(chunk);
      callback();
    } catch (error) {
      callback(error);
    }
  }

  _final(callback) {
    try {
      if (
        !this.finished ||
        this.headerBytes !== 0 ||
        this.payloadBytes !== 0 ||
        this.paddingBytes !== 0
      ) {
        throw new Error("Release archive ended before a complete ustar stream terminator");
      }
      if (
        this.memberCount === 0 ||
        this.memberCount > MAX_ARCHIVE_MEMBERS ||
        !this.rootSeen ||
        this.currentFile !== null ||
        this.pendingLongName !== null
      ) {
        throw new Error("Release archive has an invalid member layout");
      }
      this.manifest = verifyBundleRecords(
        this.fileDigests,
        this.metadataBodies,
        this.contract,
      );
      callback();
    } catch (error) {
      callback(error);
    }
  }

  consume(chunk) {
    let offset = 0;
    while (offset < chunk.length) {
      if (this.finished) {
        if (chunk.subarray(offset).some((byte) => byte !== 0)) {
          throw new Error("Release archive contains data after its ustar terminator");
        }
        return;
      }
      if (this.payloadBytes > 0) {
        const consumed = Math.min(this.payloadBytes, chunk.length - offset);
        const payload = chunk.subarray(offset, offset + consumed);
        this.currentFile.hash.update(payload);
        if (this.currentFile.chunks !== null) {
          this.currentFile.chunks.push(Buffer.from(payload));
        }
        this.payloadBytes -= consumed;
        offset += consumed;
        if (this.payloadBytes === 0) this.finishCurrentFile();
        continue;
      }
      if (this.paddingBytes > 0) {
        const consumed = Math.min(this.paddingBytes, chunk.length - offset);
        if (chunk.subarray(offset, offset + consumed).some((byte) => byte !== 0)) {
          throw new Error("Release archive has nonzero file padding");
        }
        this.paddingBytes -= consumed;
        offset += consumed;
        continue;
      }

      const copied = Math.min(
        TAR_BLOCK_BYTES - this.headerBytes,
        chunk.length - offset,
      );
      chunk.copy(this.header, this.headerBytes, offset, offset + copied);
      this.headerBytes += copied;
      offset += copied;
      if (this.headerBytes === TAR_BLOCK_BYTES) {
        this.inspectHeader();
        this.headerBytes = 0;
      }
    }
  }

  inspectHeader() {
    if (this.header.every((byte) => byte === 0)) {
      this.zeroBlocks += 1;
      if (this.zeroBlocks === 2) this.finished = true;
      return;
    }
    if (this.zeroBlocks !== 0) {
      throw new Error("Release archive has an interrupted ustar terminator");
    }

    verifyTarHeaderChecksum(this.header);
    if (this.header.subarray(257, 262).toString("ascii") !== "ustar") {
      throw new Error("Release archive must use the bounded ustar format");
    }

    const name = parseTarString(this.header.subarray(0, 100), "member name");
    const prefix = parseTarString(this.header.subarray(345, 500), "member prefix");
    const headerName = prefix ? `${prefix}/${name}` : name;
    const type = this.header[156];
    const size = parseTarOctal(this.header.subarray(124, 136), "member size");
    this.memberCount += 1;
    if (this.memberCount > MAX_ARCHIVE_MEMBERS) {
      throw new Error("Release archive has too many members");
    }
    if (type === 0x4c) {
      switch (this.contract.format) {
        case "published-windows-1.3.5":
          if (headerName !== "././@LongLink" || size < 2 || size > 241 ||
              this.pendingLongName !== null) {
            throw new Error("Historical archive has an invalid GNU long-name record");
          }
          this.payloadBytes = size;
          this.paddingBytes = (TAR_BLOCK_BYTES - (size % TAR_BLOCK_BYTES)) % TAR_BLOCK_BYTES;
          this.currentFile = { kind: "long-name", hash: createHash("sha256"), chunks: [] };
          return;
        case "current":
          throw new Error(`Release archive contains a non-file member: ${headerName}`);
        default:
          throw new Error("Unsupported release contract");
      }
    }
    const member = this.pendingLongName ?? headerName;
    this.pendingLongName = null;
    const isFile = type === 0 || type === 0x30;
    const isDirectory = type === 0x35;
    if (!isFile && !isDirectory) {
      throw new Error(`Release archive contains a non-file member: ${member}`);
    }

    if (isDirectory && size !== 0) {
      throw new Error(`Release archive directory has a payload: ${member}`);
    }
    this.expandedBytes += size;
    if (this.expandedBytes > MAX_EXPANDED_BYTES) {
      throw new Error("Release archive expands beyond the allowed size");
    }

    if (!SAFE_MEMBER_PATH.test(member)) {
      throw new Error(`Release archive has an unsafe member path: ${member}`);
    }
    const normalized = member.endsWith("/") ? member.slice(0, -1) : member;
    const components = normalized.split("/");
    if (
      Buffer.byteLength(normalized) > 240 ||
      components.length > 32 ||
      components.some((component) => component === "." || component === "..") ||
      components[0] !== this.expectedRoot ||
      (normalized === this.expectedRoot && !isDirectory) ||
      (normalized !== this.expectedRoot && components.length < 2) ||
      (isFile && member.endsWith("/"))
    ) {
      throw new Error(`Release archive member escapes the expected root: ${member}`);
    }
    if (this.uniqueNames.has(normalized)) {
      throw new Error(`Release archive repeats a member path: ${member}`);
    }
    this.uniqueNames.add(normalized);
    if (normalized === this.expectedRoot) this.rootSeen = true;

    this.payloadBytes = size;
    this.paddingBytes =
      size === 0 ? 0 : (TAR_BLOCK_BYTES - (size % TAR_BLOCK_BYTES)) % TAR_BLOCK_BYTES;
    if (isFile) {
      const relativePath = components.slice(1).join("/");
      const captureLimit =
        relativePath === "VERSION.txt"
          ? MAX_VERSION_METADATA_BYTES
          : relativePath === "SHA256SUMS.txt"
            ? MAX_BUNDLE_MANIFEST_BYTES
            : null;
      if (captureLimit !== null && size > captureLimit) {
        throw new Error(
          `Release ${relativePath} is unexpectedly large (${size} bytes)`,
        );
      }
      this.currentFile = {
        kind: "file",
        relativePath,
        hash: createHash("sha256"),
        chunks: captureLimit === null ? null : [],
      };
      if (size === 0) this.finishCurrentFile();
    }
  }

  finishCurrentFile() {
    if (this.currentFile === null) {
      throw new Error("Release archive payload is not bound to a regular file");
    }
    const { relativePath, hash, chunks } = this.currentFile;
    switch (this.currentFile.kind) {
      case "file":
        this.fileDigests.set(relativePath, hash.digest("hex"));
        if (chunks !== null) {
          this.metadataBodies.set(relativePath, Buffer.concat(chunks));
        }
        break;
      case "long-name": {
        const body = Buffer.concat(chunks);
        if (body.at(-1) !== 0 || body.subarray(0, -1).includes(0)) {
          throw new Error("Historical archive has an invalid GNU long-name payload");
        }
        this.pendingLongName = UTF8_DECODER.decode(body.subarray(0, -1));
        break;
      }
      default:
        throw new Error("Archive payload has an unknown kind");
    }
    this.currentFile = null;
  }
}

async function inspectArchiveMembers(archivePath, contract, compressedSize) {
  const ratioLimit =
    compressedSize * MAX_COMPRESSION_RATIO + COMPRESSION_RATIO_ALLOWANCE_BYTES;
  const maximumStreamBytes = Number(
    ratioLimit < BigInt(MAX_TAR_STREAM_BYTES)
      ? ratioLimit
      : BigInt(MAX_TAR_STREAM_BYTES),
  );
  const before = regularFileStat(archivePath, "release snapshot");
  const descriptor = openReadOnlyNoFollow(archivePath);
  const opened = descriptorRegularFileStat(
    descriptor,
    archivePath,
    "release snapshot",
  );
  if (!sameFileVersion(before, opened)) {
    fs.closeSync(descriptor);
    throw new Error(`Release snapshot changed before inspection: ${archivePath}`);
  }

  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), TAR_TIMEOUT_MS);
  timeout.unref();
  const inspector = new TarStreamInspector(contract, maximumStreamBytes);
  try {
    await pipeline(
      fs.createReadStream(archivePath, { fd: descriptor, autoClose: true }),
      createGunzip(),
      inspector,
      { signal: controller.signal },
    );
  } catch (error) {
    if (error?.name === "AbortError") {
      throw new Error(`Release archive inspection exceeded ${TAR_TIMEOUT_MS}ms`);
    }
    throw error;
  } finally {
    clearTimeout(timeout);
  }

  const after = regularFileStat(archivePath, "release snapshot");
  if (!sameFileVersion(before, after)) {
    throw new Error(`Release snapshot changed while it was being inspected: ${archivePath}`);
  }
  return {
    fileDigests: inspector.fileDigests,
    manifest: inspector.manifest,
  };
}

export { inspectArchiveMembers };
