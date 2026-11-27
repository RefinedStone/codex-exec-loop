import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { gunzipSync, gzipSync } from "node:zlib";
import { inspectArchiveMembers } from "../scripts/native-release/tar.mjs";

const contract = {
  format: "published-windows-1.3.5",
  name: "codex-exec-loop-native",
  version: "1.3.5",
  targetTriple: "x86_64-pc-windows-msvc",
  profile: "release",
  binaryName: "codex-exec-loop-native.exe",
  launcherName: "akra.cmd",
  expectedRoot: "codex-exec-loop-native-1.3.5-x86_64-pc-windows-msvc",
};

function archiveFixture(relativePath) {
  const directory = mkdtempSync(path.join(tmpdir(), "akra-legacy-format-"));
  const entries = new Map([
    [contract.binaryName, "synthetic binary"],
    [contract.launcherName, "synthetic launcher"],
    [relativePath, "long-name payload"],
    ["VERSION.txt", [
      "name=codex-exec-loop-native", "version=1.3.5", "target=x86_64-pc-windows-msvc",
      "profile=release", "binary=codex-exec-loop-native.exe", "launcher=akra", "",
    ].join("\n")],
  ]);
  entries.set("SHA256SUMS.txt", [...entries].map(([name, body]) =>
    createHash("sha256").update(body).digest("hex") + "  " + name).join("\n") + "\n");
  for (const [name, body] of entries) {
    const filename = path.join(directory, contract.expectedRoot, name);
    mkdirSync(path.dirname(filename), { recursive: true });
    writeFileSync(filename, body);
  }
  const archive = path.join(directory, "archive.tar.gz");
  try {
    execFileSync("tar", [
      "--format=" + (process.platform === "win32" ? "gnutar" : "gnu"),
      "-czf", archive, "-C", directory, contract.expectedRoot,
    ], { stdio: "pipe" });
    // BSD tar spells this metadata @LongName; the genuine GNU release uses @LongLink.
    const tar = gunzipSync(readFileSync(archive));
    for (let offset = 0; offset + 512 <= tar.length;) {
      const header = tar.subarray(offset, offset + 512);
      const size = Number.parseInt(header.subarray(124, 136).toString().replaceAll("\0", "").trim() || "0", 8);
      if (header[156] === 0x4c) {
        header.fill(0, 0, 100);
        header.write("././@LongLink", 0, "ascii");
        header.fill(0x20, 148, 156);
        const sum = header.reduce((total, byte) => total + byte, 0);
        header.write(sum.toString(8).padStart(6, "0") + "\0 ", 148, "ascii");
      }
      offset += 512 + Math.ceil(size / 512) * 512;
    }
    writeFileSync(archive, gzipSync(tar));
  } catch (error) {
    rmSync(directory, { recursive: true, force: true });
    throw error;
  }
  return { directory, archive };
}

test("historical GNU long names retain their exact checksummed payload path", async () => {
  const name = "assets/app-server/skills/akra-planning-queue-mutation/SKILL.md";
  const fixture = archiveFixture(name);
  try {
    const inspected = await inspectArchiveMembers(
      fixture.archive, contract, statSync(fixture.archive, { bigint: true }).size,
    );
    assert.equal(inspected.fileDigests.get(name),
      createHash("sha256").update("long-name payload").digest("hex"));
    await assert.rejects(inspectArchiveMembers(
      fixture.archive, { ...contract, format: "current" },
      statSync(fixture.archive, { bigint: true }).size,
    ), /non-file member/);
  } finally {
    rmSync(fixture.directory, { recursive: true, force: true });
  }
});

test("historical GNU long names retain the existing path size bound", async () => {
  const fixture = archiveFixture("a".repeat(100) + "/" + "b".repeat(100) + "/payload");
  try {
    await assert.rejects(inspectArchiveMembers(
      fixture.archive, contract, statSync(fixture.archive, { bigint: true }).size,
    ), /invalid GNU long-name record|escapes the expected root/);
  } finally {
    rmSync(fixture.directory, { recursive: true, force: true });
  }
});
