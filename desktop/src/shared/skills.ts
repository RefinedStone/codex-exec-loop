// 설치된 스킬의 출처 추적 계약이다 (#2383). main 이 디스크와 장부를 읽고, renderer 는 결과만 그린다.

export type SkillRuntime = "codex" | "claude";
export type SkillInstallScope = "user" | "project" | "system" | "admin";

/**
 * 행의 식별자이다: 런타임 + 호스트 + 설치 범위 + 실제 설치 경로. 같은 이름이 다른 범위나 다른 런타임에 있어도
 * 다른 행이다. 호스트는 `local` 뿐이다: 원격 호스트의 경로는 읽지도 쓰지도 않는다.
 */
export interface SkillIdentity {
  runtime: SkillRuntime;
  host: "local";
  scope: SkillInstallScope;
  /** `SKILL.md` 가 있는 폴더의 절대 경로이다. */
  installPath: string;
}

/**
 * 설치를 누가 관리하는지이다. `copy` 만 Akra 가 바꿀 수 있다. 나머지는 읽기 전용이다:
 * `plugin`(마켓플레이스·플러그인), `system`·`admin`(번들·관리자), `symlink`(폴더가 링크이다), `external`(아는 스킬 폴더 밖).
 */
export type SkillManagement = "copy" | "plugin" | "system" | "admin" | "symlink" | "external";

/** 추적 정책이다. `release` 는 prerelease 와 draft 를 뺀 최신 릴리스이다. `pinned` 는 올리지 않는다. */
export type SkillTrackingPolicy = { kind: "release" } | { kind: "branch"; branch: string } | { kind: "pinned"; ref: string };

export interface SkillSource {
  /** 소문자 `owner/name` 이다. 공개 GitHub 저장소만 받는다. */
  repository: string;
  /** 저장소 안 스킬 폴더이다 (`/` 구분). 빈 글이면 저장소의 루트이다. */
  subPath: string;
  policy: SkillTrackingPolicy;
}

/** 사용자가 출처 연결 화면에 쓴 값이다. main 이 정규화하고 검증한다. */
export interface SkillSourceInput {
  repository: string;
  subPath: string;
  policy: SkillTrackingPolicy;
}

export type SkillCheckFailureKind = "offline" | "rateLimited" | "moved" | "missing" | "noRelease" | "invalid";

export interface SkillCheckSuccess {
  /** ISO 시각이다. */
  at: string;
  /** 정책이 가리킨 ref 이다 (릴리스 태그, 브랜치, 고정 ref). */
  ref: string;
  /** ref 를 해소한 커밋이다 (40자). */
  commit: string;
  /** 그 커밋의 스킬 파일 묶음의 매니페스트 해시이다. */
  manifestHash: string;
}

export interface SkillCheckFailure {
  at: string;
  kind: SkillCheckFailureKind;
  message: string;
}

/** 출처 장부의 항목이다. 앱 데이터에 둔다. 설치 폴더에는 아무것도 쓰지 않는다. */
export interface SkillLedgerEntry {
  identity: SkillIdentity;
  source: SkillSource;
  /** 매니페스트 해시 규칙의 버전이다. */
  manifestVersion: 1;
  /** 설치된 파일 묶음과 일치한다고 확인한 커밋이다. 확인한 적이 없으면 null 이다. */
  installedCommit: string | null;
  installedManifestHash: string | null;
  /** 마지막으로 성공한 확인이다. 실패해도 지우지 않는다. */
  lastSuccess: SkillCheckSuccess | null;
  /** 가장 최근 확인이 실패했으면 그 실패이다. 성공하면 null 이다. */
  lastFailure: SkillCheckFailure | null;
}

export type SkillStatus = "sourceUnknown" | "notChecked" | "checkFailed" | "baselineUnknown" | "pinned" | "latest" | "updateAvailable";

export interface SkillState {
  status: SkillStatus;
  /** 설치 뒤에 로컬 파일이 바뀌었다. 다른 상태와 함께 나올 수 있다. */
  localModified: boolean;
  /** 마지막으로 확인한 커밋의 묶음으로 바꿀 수 있다 (미리보기와 적용). */
  canReplace: boolean;
}

/**
 * 장부와 로컬 해시로 상태를 정한다. 이름이나 파일 날짜는 보지 않는다. 가장 최근 확인이 실패했으면 마지막 성공이
 * "최신"이었어도 `checkFailed` 이다.
 */
export function classifySkill(entry: SkillLedgerEntry | null, localHash: string | null): SkillState {
  if (!entry) return { status: "sourceUnknown", localModified: false, canReplace: false };
  const localModified = entry.installedManifestHash !== null && localHash !== null && localHash !== entry.installedManifestHash;
  const checked = entry.lastSuccess;
  if (entry.lastFailure) return { status: "checkFailed", localModified, canReplace: false };
  if (!checked) return { status: "notChecked", localModified, canReplace: false };
  const differs = localHash !== null && checked.manifestHash !== localHash;
  if (entry.installedManifestHash === null) return { status: "baselineUnknown", localModified, canReplace: differs };
  if (entry.source.policy.kind === "pinned") {
    // 고정 ref 가 다른 커밋으로 옮겨져도 올리지 않는다. 같은 커밋이면 로컬 수정을 되돌릴 수 있다.
    return { status: "pinned", localModified, canReplace: differs && checked.commit === entry.installedCommit };
  }
  if (checked.manifestHash === entry.installedManifestHash) return { status: "latest", localModified, canReplace: differs };
  return { status: "updateAvailable", localModified, canReplace: differs };
}

export interface SkillRow {
  identity: SkillIdentity;
  /** 장부와 스냅샷의 키이다. */
  key: string;
  name: string;
  management: SkillManagement;
  /** Codex 의 활성 설정이다. Claude Code 는 런타임 활성 여부를 추정하지 않으므로 null 이다. */
  enabled: boolean | null;
  /** 설치 파일 묶음의 매니페스트 해시이다. 읽지 못했으면 null 이다. */
  localHash: string | null;
  /** 파일 묶음을 읽지 못한 이유이다 (링크, 비정상 파일, 한도 초과). 있으면 쓰지 않는다. */
  scanError: string | null;
  entry: SkillLedgerEntry | null;
  state: SkillState;
  /** 되돌릴 스냅샷이 있다. */
  hasSnapshot: boolean;
}

/** renderer 가 `skills/list` 에서 읽어 넘기는 Codex 스킬이다. main 은 경로를 다시 검증한다. */
export interface CodexSkillInput {
  name: string;
  /** `SKILL.md` 의 절대 경로이다. */
  path: string;
  scope: "user" | "repo" | "system" | "admin";
  enabled: boolean;
  pluginId: string | null;
}

export interface SkillListRequest {
  /** 프로젝트 범위를 읽을 작업 폴더이다. */
  cwd: string | null;
  codex: CodexSkillInput[];
}

export type SkillFileChange = "add" | "modify" | "delete";

export interface SkillPreviewFile {
  path: string;
  change: SkillFileChange;
  /** 동작이 바뀔 수 있다: 스크립트·실행 파일이거나 `SKILL.md` 의 머리(도구 선언 등)가 바뀌었다. */
  behavior: boolean;
  /** 글로 보일 수 없으면 null 이다 (바이너리, 큰 파일). */
  oldText: string | null;
  newText: string | null;
}

export interface SkillPreview {
  identity: SkillIdentity;
  source: SkillSource;
  currentCommit: string | null;
  targetCommit: string;
  /** 대상 묶음의 매니페스트 해시이다. 적용은 이 해시와 같을 때만 한다. */
  targetHash: string;
  /** 미리보기 때의 로컬 해시이다. 적용 때 다르면 중단한다. */
  localHash: string;
  files: SkillPreviewFile[];
}

/** 사용자가 미리보기에서 승인한 값이다. main 은 이 커밋과 해시 그대로만 적용한다. */
export interface SkillApproval {
  commit: string;
  targetHash: string;
  localHash: string;
}

export type SkillActionResult = { ok: true } | { ok: false; message: string; cancelled?: boolean };
export type SkillPreviewResult = { ok: true; preview: SkillPreview } | { ok: false; message: string };

/** 설치된 스킬의 출처와 업데이트이다 (#2383). 파일 쓰기는 main 만 한다. */
export interface SkillsBridge {
  list(request: SkillListRequest): Promise<SkillRow[]>;
  /** 출처를 연결하고 한 번 확인한다. 파일은 바꾸지 않는다. */
  link(identity: SkillIdentity, source: SkillSourceInput): Promise<SkillActionResult>;
  /** 지금 확인한다. 실패하면 마지막 성공 결과를 남긴다. */
  check(identity: SkillIdentity): Promise<SkillActionResult>;
  preview(identity: SkillIdentity): Promise<SkillPreviewResult>;
  /** main 이 사용자에게 확인한 뒤에 승인한 묶음으로 바꾼다. 이전 파일과 장부는 스냅샷으로 남는다. */
  apply(identity: SkillIdentity, approval: SkillApproval): Promise<SkillActionResult>;
  /** 가장 최근 스냅샷의 파일과 장부로 되돌린다. `localHash` 는 화면이 본 로컬 해시이다. */
  rollback(identity: SkillIdentity, localHash: string): Promise<SkillActionResult>;
}
