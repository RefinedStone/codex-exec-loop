import { homedir, tmpdir } from "node:os";
import { join } from "node:path";

import type { IPty } from "node-pty";
import { describe, expect, it } from "vitest";

import { IPC_CHANNELS } from "../src/shared/bridge";
import { defaultShell, TerminalService, terminalCwd, type TerminalSink } from "../src/main/terminalService";

interface FakePty {
  pty: IPty;
  emitData(data: string): void;
  emitExit(exitCode: number): void;
  written: string[];
  sizes: Array<[number, number]>;
  killed: boolean;
  options: Record<string, unknown>;
  file: string;
}

function fakeSpawn() {
  const created: FakePty[] = [];
  const spawn = ((file: string, _args: string[], options: Record<string, unknown>) => {
    let onData: (data: string) => void = () => undefined;
    let onExit: (event: { exitCode: number }) => void = () => undefined;
    const fake: FakePty = {
      written: [],
      sizes: [],
      killed: false,
      options,
      file,
      emitData: (data) => onData(data),
      emitExit: (exitCode) => onExit({ exitCode }),
      pty: {
        onData: (listener: (data: string) => void) => ((onData = listener), { dispose: () => undefined }),
        onExit: (listener: (event: { exitCode: number }) => void) => ((onExit = listener), { dispose: () => undefined }),
        write: (data: string) => fake.written.push(data),
        resize: (cols: number, rows: number) => fake.sizes.push([cols, rows]),
        kill: () => {
          fake.killed = true;
        },
      } as unknown as IPty,
    };
    created.push(fake);
    return fake.pty;
  }) as unknown as typeof import("node-pty").spawn;
  return { spawn, created };
}

function sink(id: number) {
  const sent: Array<[string, ...unknown[]]> = [];
  const owner: TerminalSink = { id, isDestroyed: () => false, send: (channel, ...args) => sent.push([channel, ...args]) };
  return { owner, sent };
}

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

describe("terminal service", () => {
  it("opens a shell in the folder and batches output into one message", async () => {
    const { spawn, created } = fakeSpawn();
    const service = new TerminalService(spawn, () => ({ file: "shell", args: [] }));
    const { owner, sent } = sink(1);
    const result = service.open(owner, { cwd: tmpdir(), cols: 100, rows: 30 });
    expect(result).toMatchObject({ ok: true, cwd: tmpdir(), shell: "shell" });
    expect(created[0]!.options).toMatchObject({ cols: 100, rows: 30, cwd: tmpdir(), name: "xterm-256color" });

    created[0]!.emitData("a");
    created[0]!.emitData("b");
    await wait(30);
    expect(sent).toEqual([[IPC_CHANNELS.terminalData, result.ok && result.id, "ab"]]);

    created[0]!.emitData("tail");
    created[0]!.emitExit(3);
    expect(sent.slice(1)).toEqual([
      [IPC_CHANNELS.terminalData, result.ok && result.id, "tail"],
      [IPC_CHANNELS.terminalExit, result.ok && result.id, 3],
    ]);
    expect(service.openCount).toBe(0);
  });

  it("only lets the owning window write, resize and close", () => {
    const { spawn, created } = fakeSpawn();
    const service = new TerminalService(spawn, () => ({ file: "shell", args: [] }));
    const result = service.open(sink(1).owner, { cwd: null, cols: 80, rows: 24 });
    const id = result.ok ? result.id : -1;

    service.write(2, id, "ls\r");
    service.resize(2, id, 10, 10);
    service.close(2, id);
    expect(created[0]!.written).toEqual([]);
    expect(created[0]!.killed).toBe(false);

    service.write(1, id, "ls\r");
    service.resize(1, id, 5000, 0);
    expect(created[0]!.written).toEqual(["ls\r"]);
    expect(created[0]!.sizes).toEqual([[1000, 2]]);
    service.close(1, id);
    expect(created[0]!.killed).toBe(true);
  });

  it("closes every terminal of a closed window and reports spawn failures", () => {
    const { spawn, created } = fakeSpawn();
    const service = new TerminalService(spawn, () => ({ file: "shell", args: [] }));
    service.open(sink(1).owner, { cwd: null, cols: 80, rows: 24 });
    service.open(sink(1).owner, { cwd: null, cols: 80, rows: 24 });
    service.open(sink(2).owner, { cwd: null, cols: 80, rows: 24 });
    service.closeAllOf(1);
    expect(created.map((fake) => fake.killed)).toEqual([true, true, false]);

    const failing = new TerminalService(
      (() => {
        throw new Error("no shell");
      }) as unknown as typeof import("node-pty").spawn,
      () => ({ file: "missing", args: [] }),
    );
    expect(failing.open(sink(3).owner, { cwd: null, cols: 80, rows: 24 })).toEqual({ ok: false, message: "no shell" });
  });

  it("falls back to the home folder and picks the login shell", () => {
    expect(terminalCwd(join(tmpdir(), "akra-missing-folder-xyz"))).toBe(homedir());
    expect(terminalCwd("relative/path")).toBe(homedir());
    expect(terminalCwd(tmpdir())).toBe(tmpdir());
    expect(defaultShell("darwin", { SHELL: "/bin/fish" })).toEqual({ file: "/bin/fish", args: ["-l"] });
    expect(defaultShell("linux", {})).toEqual({ file: "/bin/bash", args: ["-l"] });
    expect(defaultShell("win32", { SystemRoot: "C:\\Windows", PATH: "" }).file).toMatch(/powershell\.exe$/i);
  });

  // Windows 에서는 실제 셸을 띄우지 않는다 (사용자 화면에 창이 뜰 수 있다). CI(Linux) 에서 실제 pty 를 확인한다.
  it.skipIf(process.platform === "win32")("runs a real shell through node-pty", async () => {
    const { spawn } = await import("node-pty");
    const service = new TerminalService(spawn, () => ({ file: "/bin/sh", args: ["-c", "echo akra-pty-$((40+2))"] }));
    const { owner, sent } = sink(1);
    expect(service.open(owner, { cwd: tmpdir(), cols: 80, rows: 24 }).ok).toBe(true);
    for (let attempt = 0; attempt < 50 && !sent.some(([channel]) => channel === IPC_CHANNELS.terminalExit); attempt += 1) {
      await wait(100);
    }
    const output = sent.filter(([channel]) => channel === IPC_CHANNELS.terminalData).map(([, , data]) => data).join("");
    expect(output).toContain("akra-pty-42");
    expect(sent.at(-1)).toEqual([IPC_CHANNELS.terminalExit, 1, 0]);
  });
});
