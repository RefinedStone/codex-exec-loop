import type { Readable } from "node:stream";

export const MAX_FRAME_BYTES = 8 * 1024 * 1024;

export function object(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Byte framing also handles UTF-8 characters split between stream chunks. */
export function readFrames(
  input: Readable,
  onFrame: (frame: unknown) => void,
  onInvalid: () => void,
): () => void {
  let pending = Buffer.alloc(0);
  let failed = false;
  const data = (chunk: Buffer | string) => {
    if (failed) return;
    const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    let offset = 0;
    while (offset < bytes.length) {
      const end = bytes.indexOf(10, offset);
      const part = bytes.subarray(offset, end < 0 ? bytes.length : end);
      if (pending.length + part.length > MAX_FRAME_BYTES) {
        failed = true;
        onInvalid();
        return;
      }
      pending = Buffer.concat([pending, part]);
      if (end < 0) return;
      let frame: unknown;
      try {
        frame = JSON.parse(pending.toString("utf8"));
      } catch {
        failed = true;
        onInvalid();
        return;
      }
      pending = Buffer.alloc(0);
      onFrame(frame);
      if (failed) return;
      offset = end + 1;
    }
  };
  input.on("data", data);
  return () => {
    failed = true;
    pending = Buffer.alloc(0);
    input.off("data", data);
  };
}

export function encodeFrame(value: unknown): string {
  const frame = JSON.stringify(value);
  if (Buffer.byteLength(frame) > MAX_FRAME_BYTES) throw new Error("Browser frame exceeds limit.");
  return `${frame}\n`;
}
