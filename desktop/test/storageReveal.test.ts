import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { revealStoragePath } from "../src/main/storageReveal";
describe("storage Explorer reveal", () => {
  it("only selects an existing local path; it never executes the selected file", async () => {
    const directory = await mkdtemp(join(tmpdir(), "akra-storage-reveal-"));
    try {
      const path = join(directory, "instruction.cmd"); await writeFile(path, "data, not executable instructions");
      const show = vi.fn();
      await revealStoragePath({ path, host: "local" }, show);
      expect(show).toHaveBeenCalledExactlyOnceWith(path);
      await expect(revealStoragePath({ path, host: "remote" }, show)).rejects.toThrow();
      await expect(revealStoragePath({ path: "https://example.com", host: "local" }, show)).rejects.toThrow();
      await expect(revealStoragePath({ path: "relative", host: "local" }, show)).rejects.toThrow();
      await expect(revealStoragePath({ path: join(directory, "gone"), host: "local" }, show)).rejects.toThrow();
      expect(show).toHaveBeenCalledTimes(1);
    } finally { await rm(directory, { recursive: true, force: true }); }
  });
});
