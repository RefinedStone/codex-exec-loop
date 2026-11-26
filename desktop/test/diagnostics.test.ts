import { mkdtemp, mkdir, readFile, readdir, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { DIAGNOSTIC_LOG_BYTES, readDiagnosticLogs, sanitizeDiagnosticLogLines } from "../src/main/diagnosticLogs";
import { DiagnosticsService, writeDiagnosticFile, type DiagnosticsOptions } from "../src/main/diagnosticsService";

const directories: string[] = [];
afterEach(async () => { await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true }))); });
async function temporary() { const path = await mkdtemp(join(tmpdir(), "akra-diagnostics-")); directories.push(path); return path; }
const CANARY = "PRIVATE_CANARY_secret_123";
const UUID = "d1391706-2806-4203-8db3-b8e736c4f339";
const entry = (extra: Record<string, unknown> = {}) => JSON.stringify({ target: "akra_ade::adapter::inbound::gateway::session", level: "WARN", timestamp: "2026-11-26T01:02:03Z", op_id: UUID, message: CANARY, error: CANARY, api_key: CANARY, ...extra });
function options(overrides: Partial<DiagnosticsOptions> = {}): DiagnosticsOptions {
  return { desktopVersion: "0.0.0-development", electronVersion: "45.2.3", nodeVersion: "24.22.3", platform: "linux",
    readLogs: async () => ({ status: "absent", entries: [], truncated: false, omittedLines: 0 }), ...overrides };
}

describe("local connection diagnostic sanitization", () => {
  it("copies only known metadata and drops secrets, raw text, paths and unknown fields", () => {
    const result = sanitizeDiagnosticLogLines([entry({ endpoint: "/home/private/akra.sock", body: CANARY }), entry({ target: CANARY }), entry({ op_id: CANARY }), CANARY].join("\n"));
    expect(result.entries).toHaveLength(2);
    expect(result.entries[0]?.correlationId).toBe(UUID);
    expect(result.entries[1]?.correlationId).toBeNull();
    expect(JSON.stringify(result)).not.toContain(CANARY);
    expect(JSON.stringify(result)).not.toContain("/home/private");
    expect(result.omittedLines).toBe(2);
  });

  it("reads at most a 64 KiB tail and exports at most 40 metadata entries", async () => {
    const directory = await temporary();
    const path = join(directory, "trace.jsonl");
    await writeFile(path, CANARY.repeat(DIAGNOSTIC_LOG_BYTES) + "\n" + Array.from({ length: 100 }, () => entry()).join("\n"));
    const result = await readDiagnosticLogs(directory, path);
    expect(result.entries).toHaveLength(40);
    expect(result.truncated).toBe(true);
    expect(JSON.stringify(result)).not.toContain(CANARY);
  });

  it("reuses the existing default rolling trace and distinguishes absent, empty and unsafe files", async () => {
    const directory = await temporary();
    expect((await readDiagnosticLogs(directory)).status).toBe("absent");
    const logs = join(directory, ".akra", "runtime", "log");
    await mkdir(logs, { recursive: true });
    const path = join(logs, "akra-trace.jsonl.2026-11-26");
    await writeFile(path, CANARY);
    expect((await readDiagnosticLogs(directory)).status).toBe("empty");
    await writeFile(path, entry());
    expect((await readDiagnosticLogs(directory)).entries).toHaveLength(1);
    expect((await readDiagnosticLogs(directory, logs)).status).toBe("unreadable");
  });

  it.skipIf(process.platform === "win32")("does not follow a trace-file symlink", async () => {
    const directory = await temporary();
    await writeFile(join(directory, "private"), entry());
    await symlink(join(directory, "private"), join(directory, "trace"));
    expect((await readDiagnosticLogs(directory, join(directory, "trace"))).status).toBe("unreadable");
  });

  it("prepares an offline preview with explicit missing versions, redacted error and no implicit network call", async () => {
    const service = new DiagnosticsService(options());
    service.observe({ phase: "disconnected", endpoint: `/home/${CANARY}/gateway.sock`, detail: `Bearer ${CANARY}` });
    const preview = await service.preview({ gatewayVersion: CANARY, codexVersion: null });
    expect(preview.offline).toBe(true);
    expect(preview.logStatus).toBe("absent");
    expect(preview.missing.some(item => item.includes("Gateway 버전"))).toBe(true);
    expect(preview.content).toContain("detail_omitted");
    expect(preview.missing.some(item => item.includes("자유형 원문"))).toBe(true);
    expect(preview.content).not.toContain(CANARY);
    expect(JSON.parse(preview.content).versions.gateway).toBeNull();
    const prefixed = JSON.parse((await service.preview({ gatewayVersion: `1.2.3-${CANARY}`, codexVersion: "codex-cli 0.128.0-alpha.1" })).content);
    expect(prefixed.versions.gateway).toBeNull();
    expect(prefixed.versions.codex).toBe("0.128.0-alpha.1");
    service.observe({ phase: "disconnected", endpoint: "\\\\.\\pipe\\private-user", detail: "ECONNREFUSED", retryInMs: 1000 });
    const next = JSON.parse((await service.preview({ gatewayVersion: "1.2.3" })).content);
    expect(next.connection).toEqual({ phase: "disconnected", endpointKind: "named_pipe", errorCode: "ECONNREFUSED", retryInMs: 1000 });
  });
});

describe("reviewed diagnostic snapshot delivery", () => {
  it("saves the exact reviewed bytes even if the caller mutates its copy or the connection changes", async () => {
    const service = new DiagnosticsService(options());
    const preview = await service.preview({});
    const content = preview.content;
    preview.content = CANARY;
    service.observe({ phase: "connected", endpoint: "/local.sock" });
    let saved = "";
    expect(await service.save(preview.id, async () => "/selected.json", async (_path, text) => { saved = text; })).toEqual({ status: "saved" });
    expect(saved).toBe(content);
    expect(saved).not.toContain(CANARY);
  });

  it("distinguishes cancel and write failure without leaking native error text", async () => {
    const service = new DiagnosticsService(options());
    const preview = await service.preview({});
    let writes = 0;
    expect(await service.save(preview.id, async () => null, async () => { writes++; })).toEqual({ status: "cancelled" });
    expect(writes).toBe(0);
    expect(await service.save(preview.id, async () => "/selected.json", async () => { throw new Error(CANARY); })).toEqual({ status: "failed", reason: "write_failed" });
  });

  it("rejects another window's ID and invalidates an already open SaveDialog on reload", async () => {
    const service = new DiagnosticsService(options());
    const other = new DiagnosticsService(options());
    const preview = await service.preview({});
    expect(await other.save(preview.id, async () => "/selected.json")).toEqual({ status: "failed", reason: "expired" });
    let writes = 0;
    expect(await service.save(preview.id, async () => { service.invalidate(); return "/selected.json"; }, async () => { writes++; })).toEqual({ status: "failed", reason: "expired" });
    expect(writes).toBe(0);
  });

  it("rejects late collection after the owner was invalidated", async () => {
    let release!: (value: Awaited<ReturnType<DiagnosticsOptions["readLogs"]>>) => void;
    const service = new DiagnosticsService(options({ readLogs: () => new Promise(resolve => { release = resolve; }) }));
    const pending = service.preview({});
    service.invalidate();
    release({ status: "absent", entries: [], truncated: false, omittedLines: 0 });
    await expect(pending).rejects.toThrow("진단 창이 바뀌었습니다");
  });

  it("does not publish a file when the owner expires during the asynchronous write", async () => {
    const directory = await temporary();
    const destination = join(directory, "expired.json");
    const service = new DiagnosticsService(options());
    const preview = await service.preview({});
    const result = await service.save(preview.id, async () => destination, async (path, content, isCurrent) => {
      await writeDiagnosticFile(path, content, () => { service.invalidate(); return isCurrent(); });
    });
    expect(result).toEqual({ status: "failed", reason: "expired" });
    expect(await readdir(directory)).toEqual([]);
  });

  it("writes valid UTF-8 atomically and cleans temporary files after failure", async () => {
    const directory = await temporary();
    const path = join(directory, "chosen.json");
    const content = (await new DiagnosticsService(options()).preview({})).content;
    await writeDiagnosticFile(path, content);
    expect(await readFile(path, "utf8")).toBe(content);
    const blocked = join(directory, "is-directory");
    await mkdir(blocked);
    await expect(writeDiagnosticFile(blocked, content)).rejects.toThrow();
    expect((await readdir(directory)).filter(name => name.endsWith(".tmp"))).toEqual([]);
  });
});
