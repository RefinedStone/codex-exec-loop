// 앱 안의 터미널이다 (R2). main 이 node-pty 로 사용자 셸을 열고, 출력은 대화 메시지 버스와 다른 IPC 통로로 보낸다.
// 에이전트 실행 환경(sandbox, 승인)과 관계없는 사용자 자신의 셸이다. Windows 는 ConPTY 를 쓰므로 콘솔 창이 뜨지 않는다.
import { existsSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, isAbsolute, join } from "node:path";

import type { IPty } from "node-pty";

import { IPC_CHANNELS, type TerminalOpenRequest, type TerminalOpenResult } from "../shared/bridge";

/** 출력을 이 간격으로 모아 보낸다. 많은 출력이 IPC 메시지 수천 개로 쪼개지지 않게 한다. */
const FLUSH_INTERVAL_MS = 8;
const MIN_SIZE = 2;
const MAX_SIZE = 1000;

export interface ShellCommand {
  file: string;
  args: string[];
}

export interface TerminalSink {
  readonly id: number;
  isDestroyed(): boolean;
  send(channel: string, ...args: unknown[]): void;
}

interface OpenTerminal {
  pty: IPty;
  ownerId: number;
  pending: string;
  timer: NodeJS.Timeout | null;
}

function findOnPath(name: string, pathValue: string | undefined): string | null {
  for (const directory of (pathValue ?? "").split(delimiter)) {
    if (directory && existsSync(join(directory, name))) {
      return join(directory, name);
    }
  }
  return null;
}

/** 사용자 로그인 셸이다. Windows 는 PowerShell 7 이 있으면 그것을, 없으면 Windows PowerShell 을 쓴다. */
export function defaultShell(platform: NodeJS.Platform = process.platform, env: NodeJS.ProcessEnv = process.env): ShellCommand {
  if (platform === "win32") {
    const pwsh = findOnPath("pwsh.exe", env.PATH ?? env.Path);
    const windowsPowerShell = join(env.SystemRoot ?? "C:\\Windows", "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
    return { file: pwsh ?? windowsPowerShell, args: ["-NoLogo"] };
  }
  return { file: env.SHELL || (platform === "darwin" ? "/bin/zsh" : "/bin/bash"), args: ["-l"] };
}

/** 스레드 cwd 가 없거나 지워졌으면 홈 폴더에서 연다. */
export function terminalCwd(requested: unknown): string {
  if (typeof requested === "string" && isAbsolute(requested)) {
    try {
      if (statSync(requested).isDirectory()) {
        return requested;
      }
    } catch {
      // 지워진 폴더이다. 아래에서 홈 폴더를 쓴다.
    }
  }
  return homedir();
}

function clampSize(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? Math.min(MAX_SIZE, Math.max(MIN_SIZE, Math.floor(value))) : fallback;
}

export class TerminalService {
  private readonly terminals = new Map<number, OpenTerminal>();
  private nextId = 1;

  constructor(
    private readonly spawnPty: typeof import("node-pty").spawn,
    private readonly shell: () => ShellCommand = defaultShell,
  ) {}

  open(owner: TerminalSink, request: TerminalOpenRequest): TerminalOpenResult {
    const shell = this.shell();
    const cwd = terminalCwd(request?.cwd);
    let pty: IPty;
    try {
      pty = this.spawnPty(shell.file, shell.args, {
        name: "xterm-256color",
        cols: clampSize(request?.cols, 80),
        rows: clampSize(request?.rows, 24),
        cwd,
        env: { ...process.env, TERM: "xterm-256color", COLORTERM: "truecolor" } as Record<string, string>,
      });
    } catch (error) {
      return { ok: false, message: error instanceof Error ? error.message : String(error) };
    }
    const id = this.nextId++;
    const terminal: OpenTerminal = { pty, ownerId: owner.id, pending: "", timer: null };
    this.terminals.set(id, terminal);
    pty.onData((data) => {
      terminal.pending += data;
      terminal.timer ??= setTimeout(() => this.flush(id, owner), FLUSH_INTERVAL_MS);
    });
    pty.onExit(({ exitCode }) => {
      this.flush(id, owner);
      this.terminals.delete(id);
      if (!owner.isDestroyed()) {
        owner.send(IPC_CHANNELS.terminalExit, id, exitCode);
      }
    });
    return { ok: true, id, cwd, shell: shell.file };
  }

  write(ownerId: number, id: unknown, data: unknown): void {
    if (typeof data === "string") {
      this.owned(ownerId, id)?.pty.write(data);
    }
  }

  resize(ownerId: number, id: unknown, cols: unknown, rows: unknown): void {
    this.owned(ownerId, id)?.pty.resize(clampSize(cols, 80), clampSize(rows, 24));
  }

  close(ownerId: number, id: unknown): void {
    const terminal = this.owned(ownerId, id);
    if (terminal) {
      this.kill(id as number, terminal);
    }
  }

  /** 창이 닫히면 그 창의 터미널을 모두 닫는다. */
  closeAllOf(ownerId: number): void {
    for (const [id, terminal] of this.terminals) {
      if (terminal.ownerId === ownerId) {
        this.kill(id, terminal);
      }
    }
  }

  closeAll(): void {
    for (const [id, terminal] of this.terminals) {
      this.kill(id, terminal);
    }
  }

  get openCount(): number {
    return this.terminals.size;
  }

  /** 다른 창의 터미널에는 쓰지 못하게 한다. */
  private owned(ownerId: number, id: unknown): OpenTerminal | null {
    const terminal = typeof id === "number" ? this.terminals.get(id) : undefined;
    return terminal && terminal.ownerId === ownerId ? terminal : null;
  }

  private flush(id: number, owner: TerminalSink): void {
    const terminal = this.terminals.get(id);
    if (!terminal) return;
    if (terminal.timer) clearTimeout(terminal.timer);
    terminal.timer = null;
    if (terminal.pending && !owner.isDestroyed()) {
      owner.send(IPC_CHANNELS.terminalData, id, terminal.pending);
    }
    terminal.pending = "";
  }

  private kill(id: number, terminal: OpenTerminal): void {
    if (terminal.timer) clearTimeout(terminal.timer);
    this.terminals.delete(id);
    try {
      terminal.pty.kill();
    } catch {
      // 이미 끝난 셸이다.
    }
  }
}
