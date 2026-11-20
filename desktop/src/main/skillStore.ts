// 출처 장부, 스냅샷, 복구 저널이다 (#2383). 모두 앱 데이터 폴더에 둔다. 설치 폴더의 교체는 여기서만 한다.
//
// 교체 순서: 잠금 → 로컬 확인 → 스냅샷 → 같은 폴더 옆에 새 묶음 준비 → 저널(prepared) → 설치 폴더를 옆으로 옮김 →
// 새 묶음을 설치 폴더로 옮김 → 장부 기록 → 저널(committed) → 옛 폴더 삭제 → 저널 삭제.
// `committed` 전에 멈추면 파일과 장부를 모두 이전 상태로 되돌린다. 뒤에 멈추면 남은 정리만 한다.
// 이 파일의 함수는 모두 동기이다: 한 프로세스 안에서는 교체가 겹치지 않는다. 프로세스 사이는 잠금 파일로 막는다.
import { randomBytes } from "node:crypto";
import { closeSync, fsyncSync, lstatSync, mkdirSync, openSync, readdirSync, readFileSync, renameSync, rmSync, writeFileSync, writeSync } from "node:fs";
import { basename, dirname, join } from "node:path";

import type { SkillLedgerEntry } from "../shared/skills";
import { installRoot } from "./skillInstall";
import { manifestHash, readSkillFiles, type SkillFile } from "./skillManifest";

const STAGE_PREFIX = ".akra-skill-stage-";
const BACKUP_PREFIX = ".akra-skill-old-";
/** shortcut: 설치마다 최근 스냅샷 5개만 둔다. 더 오래 두어야 하면 설정으로 옮긴다. */
const SNAPSHOT_LIMIT = 5;

/** 미리보기 뒤에 로컬 파일이나 출처 상태가 바뀌었다. 아무것도 바꾸지 않았다. */
export class SkillChangedError extends Error {}

export interface SkillJournal {
  version: 1;
  key: string;
  installPath: string;
  stagePath: string;
  backupPath: string;
  /** 새 묶음의 매니페스트 해시이다. 복구는 이 해시와 같은 폴더만 지운다. */
  targetHash: string;
  /** 이 교체가 만든 스냅샷이다. 교체를 되돌리면 지운다: 그 스냅샷은 지금 파일과 같다. */
  snapshotPath: string;
  phase: "prepared" | "committed";
  previousEntry: SkillLedgerEntry;
}

export interface SkillSnapshot {
  /** 스냅샷의 파일이 있는 폴더이다. */
  filesPath: string;
  takenAt: string;
  manifestHash: string;
  /** 스냅샷을 만들 때의 장부 항목이다. 되돌리면 이것도 함께 되살린다. */
  entry: SkillLedgerEntry;
}

export interface ReplaceRequest {
  key: string;
  installPath: string;
  /** 새 파일 묶음이다. 이미 검증한 것이다. */
  files: readonly SkillFile[];
  /** 사용자가 본 로컬 해시이다. 지금과 다르면 중단한다. */
  expectedLocalHash: string;
  /** ISO 시각이다. */
  at: string;
}

function exists(path: string): boolean {
  return lstatSync(path, { throwIfNoEntry: false }) !== undefined;
}

function remove(path: string): void {
  rmSync(path, { recursive: true, force: true });
}

/** 임시 파일에 쓰고 디스크에 내린 뒤 바꿔 넣는다. 실패하면 던진다. */
function writeJsonAtomic(path: string, value: unknown): void {
  mkdirSync(dirname(path), { recursive: true });
  const temporary = `${path}.tmp-${process.pid}-${randomBytes(8).toString("hex")}`;
  const descriptor = openSync(temporary, "wx");
  try {
    try {
      writeSync(descriptor, `${JSON.stringify(value, null, 2)}\n`);
      fsyncSync(descriptor);
    } finally {
      closeSync(descriptor);
    }
    renameSync(temporary, path);
  } finally {
    rmSync(temporary, { force: true });
  }
}

function writeFiles(directory: string, files: readonly SkillFile[]): void {
  for (const file of files) {
    const target = join(directory, ...file.path.split("/"));
    mkdirSync(dirname(target), { recursive: true });
    // `wx`: 이미 있는 파일이나 링크에는 쓰지 않는다.
    writeFileSync(target, file.bytes, { flag: "wx", mode: file.executable ? 0o755 : 0o644 });
  }
}

function isAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}

export class SkillStore {
  constructor(
    private readonly dataDir: string,
    /** 폴더를 옮기는 함수이다. 시험이 파일 잠금 실패를 만들 때 바꾼다. */
    private readonly rename: (from: string, to: string) => void = renameSync,
  ) {}

  private get ledgerPath(): string {
    return join(this.dataDir, "ledger.json");
  }

  private journalPath(key: string): string {
    return join(this.dataDir, "journal", `${key}.json`);
  }

  /** 장부를 읽는다. 파일이 있는데 읽지 못하면 던진다: 읽지 못한 장부를 빈 장부로 덮어쓰지 않는다. */
  readLedger(): Record<string, SkillLedgerEntry> {
    if (!exists(this.ledgerPath)) return {};
    try {
      const parsed = JSON.parse(readFileSync(this.ledgerPath, "utf8")) as { version?: unknown; entries?: unknown };
      if (parsed.version === 1 && typeof parsed.entries === "object" && parsed.entries !== null) return parsed.entries as Record<string, SkillLedgerEntry>;
    } catch {
      // 아래에서 던진다.
    }
    throw new Error(`스킬 출처 장부를 읽지 못했습니다. 파일을 바꾸지 않았습니다: ${this.ledgerPath}`);
  }

  writeEntry(key: string, entry: SkillLedgerEntry): void {
    // 모든 키가 한 파일을 공유하므로 연결·확인·교체·복구 모두 같은 프로세스 간 잠금을 쓴다.
    this.withLock("ledger", () => {
      writeJsonAtomic(this.ledgerPath, { version: 1, entries: { ...this.readLedger(), [key]: entry } });
    });
  }

  /**
   * 설치 위치 하나의 잠금이다. 다른 프로세스가 쥐고 있으면 던진다. 죽은 프로세스의 잠금은 한 번 치운다.
   * shortcut: pid 가 다시 쓰이면 죽은 잠금을 살아 있다고 본다. 그때는 `locks/` 의 파일을 지우면 풀린다.
   */
  private withLock<T>(key: string, action: () => T): T {
    const path = join(this.dataDir, "locks", `${key}.lock`);
    mkdirSync(dirname(path), { recursive: true });
    for (let attempt = 0; ; attempt += 1) {
      try {
        writeFileSync(path, String(process.pid), { flag: "wx" });
        break;
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
        let owner = Number.NaN;
        try {
          const value = readFileSync(path, "utf8").trim();
          owner = /^[1-9]\d*$/.test(value) ? Number(value) : Number.NaN;
        } catch {
          // 방금 지워졌다. 다시 시도한다.
        }
        if (attempt > 0 || !Number.isSafeInteger(owner) || isAlive(owner)) throw new Error("다른 갱신이 이 스킬을 바꾸는 중입니다. 잠시 뒤에 다시 하세요.");
        remove(path);
      }
    }
    try {
      return action();
    } finally {
      remove(path);
    }
  }

  private snapshotsPath(key: string): string {
    return join(this.dataDir, "snapshots", key);
  }

  /** 가장 최근 스냅샷이다. 없으면 null 이다. */
  latestSnapshot(key: string): SkillSnapshot | null {
    let names: string[];
    try {
      names = readdirSync(this.snapshotsPath(key)).sort().reverse();
    } catch {
      return null;
    }
    for (const name of names) {
      try {
        const meta = JSON.parse(readFileSync(join(this.snapshotsPath(key), name, "snapshot.json"), "utf8")) as Omit<SkillSnapshot, "filesPath">;
        return { ...meta, filesPath: join(this.snapshotsPath(key), name, "files") };
      } catch {
        // 만들다 멈춘 스냅샷이다. 건너뛴다.
      }
    }
    return null;
  }

  private takeSnapshot(key: string, files: readonly SkillFile[], entry: SkillLedgerEntry, at: string): string {
    // 이름은 차례 번호로 시작한다: 같은 시각에 만든 스냅샷도 만든 순서로 정렬된다.
    mkdirSync(this.snapshotsPath(key), { recursive: true });
    const last = readdirSync(this.snapshotsPath(key)).sort().at(-1);
    const sequence = String((Number.parseInt(last ?? "0", 10) || 0) + 1).padStart(8, "0");
    const directory = join(this.snapshotsPath(key), `${sequence}-${at.replace(/[:.]/g, "-")}`);
    const hash = manifestHash(files);
    mkdirSync(join(directory, "files"), { recursive: true });
    writeFiles(join(directory, "files"), files);
    if (manifestHash(readSkillFiles(join(directory, "files"))) !== hash) throw new Error("스냅샷이 원본과 다릅니다. 아무것도 바꾸지 않았습니다.");
    // 메타 파일을 마지막에 쓴다: 메타가 있는 스냅샷만 완전한 것이다.
    writeJsonAtomic(join(directory, "snapshot.json"), { takenAt: at, manifestHash: hash, entry });
    return directory;
  }

  /**
   * 교체를 준비한다: 로컬 파일이 사용자가 본 것과 같은지 확인하고, 스냅샷을 만들고, 설치 폴더 옆(같은 파일시스템)에
   * 새 묶음을 쓰고, 저널을 적는다. 설치 폴더는 아직 그대로이다.
   */
  prepareReplace(request: ReplaceRequest, previousEntry: SkillLedgerEntry): SkillJournal {
    const current = readSkillFiles(request.installPath);
    if (manifestHash(current) !== request.expectedLocalHash) throw new SkillChangedError("미리보기 뒤에 로컬 파일이 바뀌었습니다. 다시 검토하세요.");
    const snapshotPath = this.takeSnapshot(request.key, current, previousEntry, request.at);
    const suffix = randomBytes(6).toString("hex");
    const parent = dirname(request.installPath);
    const journal: SkillJournal = {
      version: 1,
      key: request.key,
      installPath: request.installPath,
      stagePath: join(parent, `${STAGE_PREFIX}${suffix}`),
      backupPath: join(parent, `${BACKUP_PREFIX}${suffix}`),
      targetHash: manifestHash(request.files),
      snapshotPath,
      phase: "prepared",
      previousEntry,
    };
    try {
      mkdirSync(journal.stagePath);
      writeFiles(journal.stagePath, request.files);
      if (manifestHash(readSkillFiles(journal.stagePath)) !== journal.targetHash) throw new Error("준비한 묶음이 승인한 묶음과 다릅니다.");
      writeJsonAtomic(this.journalPath(request.key), journal);
    } catch (error) {
      remove(journal.stagePath);
      remove(snapshotPath);
      throw error;
    }
    return journal;
  }

  /** 준비한 묶음으로 바꾸고 장부를 확정한다. `committed` 를 적기 전에 던지면 `recoverJournal` 이 되돌린다. */
  commitReplace(journal: SkillJournal, nextEntry: SkillLedgerEntry): void {
    this.rename(journal.installPath, journal.backupPath);
    this.rename(journal.stagePath, journal.installPath);
    this.writeEntry(journal.key, nextEntry);
    writeJsonAtomic(this.journalPath(journal.key), { ...journal, phase: "committed" });
    try {
      this.recoverJournal(journal.key);
    } catch {
      // 옛 폴더를 지우지 못했다. 교체는 끝났다. 남은 정리는 다음 복구가 한다.
    }
  }

  private readJournal(key: string): SkillJournal | null {
    if (!exists(this.journalPath(key))) return null;
    let journal: SkillJournal;
    try {
      journal = JSON.parse(readFileSync(this.journalPath(key), "utf8")) as SkillJournal;
    } catch {
      throw new Error(`복구 저널을 읽지 못했습니다: ${this.journalPath(key)}`);
    }
    const parent = typeof journal.installPath === "string" ? dirname(journal.installPath) : "";
    // 저널의 경로를 그대로 믿지 않는다. 아는 스킬 폴더 아래이고, 옆 폴더가 Akra 가 만든 이름일 때만 손댄다.
    const trusted =
      journal.version === 1 &&
      journal.key === key &&
      journal.previousEntry?.identity?.installPath === journal.installPath &&
      installRoot(journal.previousEntry.identity.runtime, journal.installPath) !== null &&
      typeof journal.stagePath === "string" &&
      typeof journal.backupPath === "string" &&
      dirname(journal.stagePath) === parent &&
      dirname(journal.backupPath) === parent &&
      typeof journal.snapshotPath === "string" &&
      dirname(journal.snapshotPath) === this.snapshotsPath(key) &&
      basename(journal.stagePath).startsWith(STAGE_PREFIX) &&
      basename(journal.backupPath).startsWith(BACKUP_PREFIX);
    if (!trusted) throw new Error(`복구 저널의 경로가 올바르지 않아 손대지 않았습니다: ${this.journalPath(key)}`);
    return journal;
  }

  /**
   * 멈춘 교체를 끝낸다. `committed` 전이면 설치 폴더와 장부를 이전 상태로 되돌린다. 뒤이면 남은 폴더만 지운다.
   * 설치 폴더가 저널의 새 묶음과 다르면 지우지 않고 던진다: 다른 것이 그 자리에 쓴 것이다.
   */
  recoverJournal(key: string): void {
    const journal = this.readJournal(key);
    if (!journal) return;
    if (journal.phase !== "committed") {
      if (exists(journal.backupPath)) {
        if (exists(journal.installPath)) {
          if (manifestHash(readSkillFiles(journal.installPath)) !== journal.targetHash) {
            throw new Error(`복구를 멈췄습니다. 설치 폴더가 저널과 다릅니다. 이전 파일은 여기에 있습니다: ${journal.backupPath}`);
          }
          remove(journal.installPath);
        }
        this.rename(journal.backupPath, journal.installPath);
      }
      this.writeEntry(key, journal.previousEntry);
      remove(journal.snapshotPath);
    } else {
      for (const old of readdirSync(this.snapshotsPath(key)).sort().reverse().slice(SNAPSHOT_LIMIT)) remove(join(this.snapshotsPath(key), old));
    }
    remove(journal.stagePath);
    remove(journal.backupPath);
    remove(this.journalPath(key));
  }

  /** 앱을 다시 켰을 때 남은 저널을 모두 복구한다. 복구하지 못한 것은 이유를 돌려준다. */
  recoverAll(): string[] {
    let names: string[];
    try {
      names = readdirSync(join(this.dataDir, "journal"));
    } catch {
      return [];
    }
    const problems: string[] = [];
    for (const name of names.filter((item) => item.endsWith(".json"))) {
      const key = name.slice(0, -".json".length);
      try {
        this.withLock(key, () => this.recoverJournal(key));
      } catch (error) {
        problems.push(error instanceof Error ? error.message : String(error));
      }
    }
    return problems;
  }

  /**
   * 설치 폴더를 새 묶음으로 바꾼다. `next` 는 잠금 안에서 지금의 장부 항목을 받아 새 항목을 만든다 (바뀌었으면 던진다).
   * 실패하면 파일과 장부를 이전 상태로 되돌리고 던진다.
   */
  replace(request: ReplaceRequest, next: (previous: SkillLedgerEntry) => SkillLedgerEntry): void {
    this.withLock(request.key, () => {
      this.recoverJournal(request.key);
      const previous = this.readLedger()[request.key];
      if (!previous) throw new SkillChangedError("출처가 연결되어 있지 않습니다.");
      const nextEntry = next(previous);
      const journal = this.prepareReplace(request, previous);
      try {
        this.commitReplace(journal, nextEntry);
      } catch (error) {
        this.recoverJournal(request.key);
        throw new Error(`스킬을 바꾸지 못해 이전 상태로 되돌렸습니다: ${error instanceof Error ? error.message : String(error)}`);
      }
    });
  }
}
