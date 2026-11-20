// 설치된 스킬의 출처 추적, 확인, 미리보기, 적용, 되돌리기이다 (#2383). renderer 는 식별자와 승인 값만 보낸다.
// 경로, 장부, 원본은 여기서 다시 읽고 검증한다. 받은 파일은 실행하지 않는다. 훅이나 스크립트도 돌리지 않는다.
import { basename, dirname, isAbsolute, resolve } from "node:path";

import {
  classifySkill,
  type CodexSkillInput,
  type SkillActionResult,
  type SkillApproval,
  type SkillIdentity,
  type SkillInstallScope,
  type SkillLedgerEntry,
  type SkillListRequest,
  type SkillManagement,
  type SkillPreviewFile,
  type SkillPreviewResult,
  type SkillRow,
  type SkillRuntime,
} from "../shared/skills";
import { classifyInstall, scanClaudeSkills, skillKey } from "./skillInstall";
import { manifestHash, readSkillFiles, type SkillFile } from "./skillManifest";
import { fetchBundle, normalizeSource, resolveCommit, type SkillFetch, SkillSourceError } from "./skillSource";
import { SkillStore } from "./skillStore";

export interface SkillServiceOptions {
  /** 장부, 스냅샷, 저널을 두는 앱 데이터 폴더이다. */
  dataDir: string;
  home: string;
  fetcher: SkillFetch;
  now?: () => Date;
  /** 폴더를 옮기는 함수이다. 시험이 파일 잠금 실패를 만들 때 바꾼다. */
  rename?: (from: string, to: string) => void;
}

/** 쓰기 전에 사용자에게 묻는다. main 이 확인 창을 띄운다. */
export type SkillConfirm = (message: string, detail: string) => Promise<boolean>;

const CANCELLED: SkillActionResult = { ok: false, message: "취소했습니다.", cancelled: true };
const TEXT_LIMIT = 256 * 1024;

function failure(error: unknown): { ok: false; message: string } {
  return { ok: false, message: error instanceof Error ? error.message : String(error) };
}

/** 글로 보일 수 있는 내용이면 그 글이다. 바이너리이거나 크면 null 이다. */
function textOf(bytes: Buffer): string | null {
  if (bytes.length > TEXT_LIMIT || bytes.includes(0)) return null;
  const text = bytes.toString("utf8");
  return Buffer.from(text, "utf8").equals(bytes) ? text : null;
}

function frontMatter(bytes: Buffer | undefined): string {
  return /^---\r?\n[\s\S]*?\r?\n---/.exec((bytes && textOf(bytes)) ?? "")?.[0] ?? "";
}

/** 동작이 바뀔 수 있는 파일인지 본다: 문서가 아닌 파일(스크립트 등), 실행 파일, `SKILL.md` 의 머리(도구 선언 등). */
function changesBehavior(path: string, before: SkillFile | undefined, after: SkillFile | undefined): boolean {
  if (path === "SKILL.md") return frontMatter(before?.bytes) !== frontMatter(after?.bytes);
  return Boolean(before?.executable || after?.executable) || !/\.(md|txt)$/i.test(path);
}

/** 설치 묶음과 대상 묶음의 파일별 차이이다. 같은 파일은 넣지 않는다. */
export function diffBundles(local: readonly SkillFile[], target: readonly SkillFile[]): SkillPreviewFile[] {
  const before = new Map(local.map((file) => [file.path, file]));
  const after = new Map(target.map((file) => [file.path, file]));
  const files: SkillPreviewFile[] = [];
  for (const path of [...new Set([...before.keys(), ...after.keys()])].sort()) {
    const old = before.get(path);
    const next = after.get(path);
    if (old && next && old.bytes.equals(next.bytes)) continue;
    files.push({
      path,
      change: !old ? "add" : !next ? "delete" : "modify",
      behavior: changesBehavior(path, old, next),
      oldText: old ? textOf(old.bytes) : "",
      newText: next ? textOf(next.bytes) : "",
    });
  }
  return files;
}

const RUNTIMES: readonly SkillRuntime[] = ["codex", "claude"];
const SCOPES: readonly SkillInstallScope[] = ["user", "project", "system", "admin"];

export class SkillService {
  private readonly store: SkillStore;
  private readonly home: string;
  private readonly fetcher: SkillFetch;
  private readonly now: () => Date;

  constructor(options: SkillServiceOptions) {
    this.store = new SkillStore(options.dataDir, options.rename);
    this.home = options.home;
    this.fetcher = options.fetcher;
    this.now = options.now ?? (() => new Date());
  }

  /** 멈춘 교체를 복구한다. 앱을 켤 때와 목록을 읽을 때 부른다. 복구하지 못한 이유를 돌려준다. */
  recover(): string[] {
    return this.store.recoverAll();
  }

  /**
   * renderer 가 보낸 식별자를 검증한다. Akra 가 바꿀 수 있는 복사본이 아니면 던진다: 플러그인·시스템 설치, 링크,
   * 아는 스킬 폴더 밖의 경로에는 출처를 연결하지도 쓰지도 않는다.
   */
  private writableIdentity(raw: unknown): SkillIdentity {
    const value = (raw ?? {}) as Partial<SkillIdentity>;
    if (!RUNTIMES.includes(value.runtime as SkillRuntime) || value.host !== "local" || !SCOPES.includes(value.scope as SkillInstallScope) || typeof value.installPath !== "string") {
      throw new Error("스킬 식별자가 올바르지 않습니다.");
    }
    const identity: SkillIdentity = { runtime: value.runtime as SkillRuntime, host: "local", scope: value.scope as SkillInstallScope, installPath: value.installPath };
    const location = classifyInstall(identity.runtime, identity.installPath, this.home);
    if (location.management !== "copy" || location.scope !== identity.scope) {
      throw new Error("이 설치는 Akra 가 바꾸지 않습니다 (플러그인·시스템 설치, 링크 또는 스킬 폴더 밖). 그 설치를 관리하는 도구에서 업데이트하세요.");
    }
    return identity;
  }

  private entryOf(key: string): SkillLedgerEntry {
    const entry = this.store.readLedger()[key];
    if (!entry) throw new Error("출처가 연결되어 있지 않습니다. 먼저 출처를 연결하세요.");
    return entry;
  }

  private row(found: { runtime: SkillRuntime; name: string; installPath: string; codex?: CodexSkillInput }, ledger: Record<string, SkillLedgerEntry>): SkillRow {
    const location = classifyInstall(found.runtime, found.installPath, this.home);
    const codex = found.codex;
    // Codex 가 플러그인·시스템·관리자 것이라고 알린 설치는 경로가 어디든 읽기 전용이다.
    const management: SkillManagement = codex?.pluginId ? "plugin" : codex?.scope === "system" ? "system" : codex?.scope === "admin" ? "admin" : location.management;
    const scope: SkillInstallScope =
      location.management === "copy" && management === "copy" ? location.scope : codex ? (codex.scope === "repo" ? "project" : codex.scope) : installScopeOfClaude(found.installPath, this.home);
    const identity: SkillIdentity = { runtime: found.runtime, host: "local", scope, installPath: found.installPath };
    const key = skillKey(identity);
    let localHash: string | null = null;
    let scanError: string | null = null;
    if (management === "copy") {
      try {
        localHash = manifestHash(readSkillFiles(found.installPath));
      } catch (error) {
        scanError = error instanceof Error ? error.message : String(error);
      }
    }
    const entry = management === "copy" ? (ledger[key] ?? null) : null;
    return {
      identity,
      key,
      name: found.name,
      management,
      enabled: codex ? codex.enabled : null,
      localHash,
      scanError,
      entry,
      state: classifySkill(entry, localHash),
      hasSnapshot: management === "copy" && this.store.latestSnapshot(key) !== null,
    };
  }

  /** 설치된 스킬을 행 식별자마다 한 줄로 돌려준다. Codex 는 `skills/list` 의 결과를, Claude Code 는 디스크를 읽는다. */
  list(raw: unknown): SkillRow[] {
    this.recover();
    const request = (raw ?? {}) as Partial<SkillListRequest>;
    const cwd = typeof request.cwd === "string" ? request.cwd : null;
    const ledger = this.store.readLedger();
    const rows = new Map<string, SkillRow>();
    for (const input of Array.isArray(request.codex) ? request.codex : []) {
      const skill = input as Partial<CodexSkillInput> | null;
      if (typeof skill?.name !== "string" || typeof skill.path !== "string" || typeof skill.scope !== "string") continue;
      if (!isAbsolute(skill.path) || resolve(skill.path) !== skill.path || basename(skill.path) !== "SKILL.md") continue;
      if (!["user", "repo", "system", "admin"].includes(skill.scope)) continue;
      const codex: CodexSkillInput = { name: skill.name, path: skill.path, scope: skill.scope as CodexSkillInput["scope"], enabled: skill.enabled === true, pluginId: typeof skill.pluginId === "string" ? skill.pluginId : null };
      const row = this.row({ runtime: "codex", name: codex.name, installPath: dirname(codex.path), codex }, ledger);
      rows.set(row.key, row);
    }
    for (const found of scanClaudeSkills(this.home, cwd)) {
      const row = this.row({ runtime: "claude", name: found.name, installPath: found.installPath }, ledger);
      rows.set(row.key, row);
    }
    return [...rows.values()];
  }

  /** 출처를 연결하고 한 번 확인한다. 이름이나 파일 날짜로 연결하지 않는다: 사용자가 쓴 출처만 쓴다. */
  async link(rawIdentity: unknown, rawSource: unknown): Promise<SkillActionResult> {
    try {
      const identity = this.writableIdentity(rawIdentity);
      const source = normalizeSource(rawSource);
      const key = skillKey(identity);
      const previous = this.store.readLedger()[key];
      // 같은 저장소와 경로이면 정책만 바꾼 것이다. 확인한 설치 커밋은 그대로 둔다.
      const sameBundle = previous !== undefined && previous.source.repository === source.repository && previous.source.subPath === source.subPath;
      this.store.writeEntry(key, {
        identity,
        source,
        manifestVersion: 1,
        installedCommit: sameBundle ? previous.installedCommit : null,
        installedManifestHash: sameBundle ? previous.installedManifestHash : null,
        lastSuccess: null,
        lastFailure: null,
      });
      return await this.runCheck(identity.installPath, key);
    } catch (error) {
      return failure(error);
    }
  }

  async check(rawIdentity: unknown): Promise<SkillActionResult> {
    try {
      const identity = this.writableIdentity(rawIdentity);
      return await this.runCheck(identity.installPath, skillKey(identity));
    } catch (error) {
      return failure(error);
    }
  }

  /**
   * ref 를 커밋으로 해소하고 그 커밋의 스킬 파일 묶음의 해시를 구한다. 실패하면 마지막 성공 결과를 남기고 실패만 적는다.
   * 설치 커밋을 모르는 복사본이 그 묶음과 같으면 그 커밋을 설치 커밋으로 적는다 (일치 증거).
   */
  private async runCheck(installPath: string, key: string): Promise<SkillActionResult> {
    const entry = this.entryOf(key);
    let outcome: Pick<SkillLedgerEntry, "lastSuccess" | "lastFailure"> | Pick<SkillLedgerEntry, "lastFailure">;
    try {
      const { ref, commit } = await resolveCommit(this.fetcher, entry.source);
      // 같은 저장소·경로·커밋의 묶음은 바뀌지 않는다. 이미 아는 해시는 다시 받지 않는다.
      const known = entry.lastSuccess?.commit === commit ? entry.lastSuccess.manifestHash : entry.installedCommit === commit ? entry.installedManifestHash : null;
      const hash = known ?? manifestHash(await fetchBundle(this.fetcher, entry.source.repository, entry.source.subPath, commit));
      outcome = { lastSuccess: { at: this.now().toISOString(), ref, commit, manifestHash: hash }, lastFailure: null };
    } catch (error) {
      if (!(error instanceof SkillSourceError)) throw error;
      outcome = { lastFailure: { at: this.now().toISOString(), kind: error.kind, message: error.message } };
    }
    // 기다리는 동안 출처가 바뀌었으면 결과를 버린다.
    const current = this.store.readLedger()[key];
    if (!current || JSON.stringify(current.source) !== JSON.stringify(entry.source)) return { ok: false, message: "확인하는 동안 출처가 바뀌어 결과를 버렸습니다." };
    let next: SkillLedgerEntry = { ...current, ...outcome };
    if ("lastSuccess" in outcome && outcome.lastSuccess && next.installedManifestHash === null) {
      const checked = outcome.lastSuccess;
      if (this.localHash(installPath) === checked.manifestHash) next = { ...next, installedCommit: checked.commit, installedManifestHash: checked.manifestHash };
    }
    this.store.writeEntry(key, next);
    return next.lastFailure ? { ok: false, message: next.lastFailure.message } : { ok: true };
  }

  private localHash(installPath: string): string {
    return manifestHash(readSkillFiles(installPath));
  }

  /** 마지막으로 확인한 커밋의 묶음과 설치 묶음의 차이이다. 아무것도 쓰지 않는다. */
  async preview(rawIdentity: unknown): Promise<SkillPreviewResult> {
    try {
      const identity = this.writableIdentity(rawIdentity);
      const entry = this.entryOf(skillKey(identity));
      const target = entry.lastSuccess;
      if (!target || !classifySkill(entry, this.localHash(identity.installPath)).canReplace) return { ok: false, message: "바꿀 내용이 없습니다. 먼저 지금 확인하세요." };
      const files = await fetchBundle(this.fetcher, entry.source.repository, entry.source.subPath, target.commit);
      if (manifestHash(files) !== target.manifestHash) return { ok: false, message: "원본이 확인 때와 다릅니다. 다시 확인하세요." };
      const local = readSkillFiles(identity.installPath);
      return {
        ok: true,
        preview: {
          identity,
          source: entry.source,
          currentCommit: entry.installedCommit,
          targetCommit: target.commit,
          targetHash: target.manifestHash,
          localHash: manifestHash(local),
          files: diffBundles(local, files),
        },
      };
    } catch (error) {
      return failure(error);
    }
  }

  /**
   * 승인한 커밋과 해시 그대로 설치 폴더를 바꾼다. 승인 값이 마지막 확인과 다르거나, 미리보기 뒤에 로컬 파일이 바뀌었으면
   * 아무것도 바꾸지 않는다. 로컬 수정은 병합하지 않는다: 이전 파일 묶음을 스냅샷으로 남기고 통째로 바꾼다.
   */
  async apply(rawIdentity: unknown, rawApproval: unknown, confirm: SkillConfirm): Promise<SkillActionResult> {
    try {
      const identity = this.writableIdentity(rawIdentity);
      const approval = (rawApproval ?? {}) as Partial<SkillApproval>;
      const { commit, targetHash, localHash } = approval;
      if (typeof commit !== "string" || typeof targetHash !== "string" || typeof localHash !== "string") throw new Error("승인 값이 올바르지 않습니다.");
      const key = skillKey(identity);
      const approved = (entry: SkillLedgerEntry): boolean => entry.lastSuccess?.commit === commit && entry.lastSuccess.manifestHash === targetHash && entry.lastFailure === null;
      const entry = this.entryOf(key);
      if (!approved(entry)) return { ok: false, message: "승인한 커밋이 마지막 확인과 다릅니다. 다시 검토하세요." };
      if (this.localHash(identity.installPath) !== localHash) return { ok: false, message: "미리보기 뒤에 로컬 파일이 바뀌었습니다. 다시 검토하세요." };
      if (!classifySkill(entry, localHash).canReplace) return { ok: false, message: "바꿀 내용이 없습니다." };
      const files = await fetchBundle(this.fetcher, entry.source.repository, entry.source.subPath, commit);
      if (manifestHash(files) !== targetHash) return { ok: false, message: "받은 파일 묶음이 승인한 해시와 다릅니다. 다시 확인하세요." };
      const modified = entry.installedManifestHash !== null && entry.installedManifestHash !== localHash;
      const detail = [
        `${identity.installPath}`,
        `원본: ${entry.source.repository}${entry.source.subPath ? `/${entry.source.subPath}` : ""}`,
        `커밋: ${entry.installedCommit?.slice(0, 12) ?? "미확인"} → ${commit.slice(0, 12)}`,
        "",
        modified ? "로컬 수정이 있습니다. 병합하지 않고 통째로 바꿉니다." : "설치 폴더의 파일을 통째로 바꿉니다.",
        "이전 파일과 출처 기록은 스냅샷으로 남고, 롤백으로 되돌릴 수 있습니다. 받은 스크립트는 실행하지 않습니다.",
      ].join("\n");
      if (!(await confirm("이 스킬을 승인한 커밋의 파일로 바꿉니까?", detail))) return CANCELLED;
      // 확인 창을 기다리는 동안 폴더가 링크로 바뀌었을 수 있다. 쓰기 직전에 다시 검증한다.
      this.writableIdentity(identity);
      this.store.replace({ key, installPath: identity.installPath, files, expectedLocalHash: localHash, at: this.now().toISOString() }, (previous) => {
        if (!approved(previous) || JSON.stringify(previous.source) !== JSON.stringify(entry.source)) throw new Error("출처 상태가 바뀌었습니다. 다시 검토하세요.");
        return { ...previous, installedCommit: commit, installedManifestHash: targetHash };
      });
      return { ok: true };
    } catch (error) {
      return failure(error);
    }
  }

  /**
   * 가장 최근 스냅샷의 파일 묶음과 장부 항목으로 되돌린다. 화면이 본 뒤에 로컬 파일이 바뀌었으면 중단한다.
   * 지금 파일도 스냅샷으로 남는다: 되돌린 것을 다시 되돌릴 수 있다.
   */
  async rollback(rawIdentity: unknown, localHash: unknown, confirm: SkillConfirm): Promise<SkillActionResult> {
    try {
      const identity = this.writableIdentity(rawIdentity);
      const key = skillKey(identity);
      const snapshot = this.store.latestSnapshot(key);
      if (!snapshot) return { ok: false, message: "되돌릴 스냅샷이 없습니다." };
      if (typeof localHash !== "string" || this.localHash(identity.installPath) !== localHash) return { ok: false, message: "화면을 읽은 뒤에 로컬 파일이 바뀌었습니다. 목록을 새로 읽고 다시 하세요." };
      const files = readSkillFiles(snapshot.filesPath);
      if (manifestHash(files) !== snapshot.manifestHash) return { ok: false, message: "스냅샷이 기록과 다릅니다. 되돌리지 않았습니다." };
      const entry = this.entryOf(key);
      const modified = entry.installedManifestHash !== null && entry.installedManifestHash !== localHash;
      const detail = [
        `${identity.installPath}`,
        `스냅샷: ${snapshot.takenAt} (커밋 ${snapshot.entry.installedCommit?.slice(0, 12) ?? "미확인"})`,
        "",
        modified ? "설치 뒤에 바뀐 로컬 파일이 있습니다. 지금 파일도 스냅샷으로 남깁니다." : "지금 파일도 스냅샷으로 남깁니다.",
        "파일 묶음과 출처 기록을 함께 되돌립니다.",
      ].join("\n");
      if (!(await confirm("이 스킬을 이전 스냅샷으로 되돌립니까?", detail))) return CANCELLED;
      this.writableIdentity(identity);
      this.store.replace({ key, installPath: identity.installPath, files, expectedLocalHash: localHash, at: this.now().toISOString() }, () => snapshot.entry);
      return { ok: true };
    } catch (error) {
      return failure(error);
    }
  }
}

/** 링크나 읽지 못한 Claude Code 설치의 범위이다: 홈의 `.claude/skills` 아래이면 사용자, 아니면 프로젝트이다. */
function installScopeOfClaude(installPath: string, home: string): SkillInstallScope {
  return dirname(dirname(dirname(installPath))) === resolve(home) ? "user" : "project";
}
