import type { IssueEventReference } from "./kernel-protocol/generated";

// 로컬 복원본 창의 상태·표시와 채널 값이다 (#2427). window.akra 권한 계약은 bridge.ts 에 있다.
// 복원 평문은 main 에서 복원 창의 renderer 로만 간다. Gateway 와 Kernel 프로토콜로 가지 않는다.

/** 로컬 복원본 창의 preload 만 쓰는 채널이다. main 은 보낸 창이 복원본 창인지 확인한다. */
export const VAULT_RESTORE_CHANNELS = {
  ready: "akra-vault-restore:ready",
  state: "akra-vault-restore:state",
  unlock: "akra-vault-restore:unlock",
  lock: "akra-vault-restore:lock",
  review: "akra-vault-restore:review",
  reconnect: "akra-vault-restore:reconnect",
} as const;

/** 열람을 보류한 이유이다. 값에 평문이나 게시본의 글은 없다. */
export type VaultRestoreHold =
  // 실행 프로필: 신뢰된 경계를 강제할 수 없다.
  | "profile_automation"
  | "profile_dev_renderer"
  | "profile_hidden_window"
  | "agent_working"
  | "os_locked"
  | "os_lock_unavailable"
  | "os_lock_unsupported"
  | "system_suspended"
  // 로컬 helper를 열 수 없거나 이 플랫폼이 native Vault 열람을 지원하지 않는다.
  | "source_unavailable"
  | "platform_unsupported"
  | "confirmation_cancelled"
  // Vault 복원 서비스의 보류 상태이다 (`VaultRestoreHold`, #2425).
  | "locked"
  | "latest_read_failed"
  | "not_bound"
  | "published_copy_changed"
  | "marker_moved"
  | "marker_duplicated"
  | "marker_missing"
  | "changed_during_review"
  | "mapping_rejected"
  | "record_unavailable"
  | "write_outcome_unknown";

/** 다시 잠근 이유이다. */
export type VaultRelockCause =
  | "user"
  | "window_closed"
  | "window_hidden"
  | "renderer_changed"
  | "session_expired"
  | "os_lock"
  | "suspend"
  | "project_switch"
  | "agent_working";

export type VaultField = "title" | "body";

/** 게시본에서 표식이 있는 자리이다. `position` 은 필드의 글에서 표식이 시작하는 바이트 위치이다. */
export interface VaultMarkerPlace {
  field: VaultField;
  position: number;
}

/** 지금 게시본에서 표식이 있는 자리와 그 앞뒤의 글이다. */
export interface VaultMarkerCandidate extends VaultMarkerPlace {
  context: string;
}

export interface VaultReconnectMarker {
  marker: string;
  /** 연결한 판(기존 공유본)에서의 자리이다. */
  previous: VaultMarkerPlace;
  /** 지금 공유본에서 이 표식이 있는 모든 자리이다. 없으면 비어 있다. */
  candidates: VaultMarkerCandidate[];
}

/** 재연결 검토의 값이다. 평문은 없다. 게시본의 글은 다른 사람이 고친 원시 값이다: 글로만 보인다. */
export interface VaultReconnectReview {
  /** 검토한 게시본의 필드 해시이다. 승인은 이 값에 묶인다. */
  reviewedHash: string;
  publishedTitle: string;
  publishedBody: string;
  markers: VaultReconnectMarker[];
}

/** 사용자가 승인한 연결 하나이다. 승인하지 않은 표식은 복원본에 표식으로 남는다. */
export interface VaultApprovedMapping extends VaultMarkerPlace {
  marker: string;
}

export type VaultRestoreViewState =
  /** `cause` 가 null 이면 아직 열지 않았다. */
  | { phase: "locked"; cause: VaultRelockCause | null }
  | { phase: "opening" }
  | { phase: "reviewing" }
  | { phase: "reconnecting" }
  | { phase: "held"; reason: VaultRestoreHold; canReconnect?: boolean }
  | { phase: "restored"; title: string; body: string; expiresAt: number }
  | { phase: "review"; review: VaultReconnectReview };

/** 로컬 복원본을 여는 대상이다: github.com 의 이슈 주소 하나이다. */
export function isVaultRestoreIssueUrl(url: unknown): url is string {
  if (typeof url !== "string") return false;
  const match = /^https:\/\/github\.com\/([\w.-]{1,100})\/([\w.-]{1,100})\/issues\/([1-9]\d{0,18})$/.exec(url);
  return match !== null && ![".", ".."].includes(match[1]!) && ![".", ".."].includes(match[2]!) &&
    BigInt(match[3]!) <= 9_223_372_036_854_775_807n;
}

/** main 이 직접 확인한 실행 프로필의 사실이다. renderer 가 준 값은 쓰지 않는다. */
export interface VaultRestoreProfile {
  /** 원격 디버깅이나 inspector 로 앱을 조종할 수 있다 (화면 시험, 자동화 도구). */
  automation: boolean;
  /** renderer 를 개발 서버(http)에서 읽는다. 검증한 로컬 파일이 아니다. */
  devRenderer: boolean;
  /** 창을 숨기는 시험 방식이다. */
  hiddenWindows: boolean;
  /** 이 앱의 창에 실행 중인 턴이 있다. 기본 실행 프로필의 에이전트는 범용 shell 권한을 가진다. */
  agentWorking: boolean;
}

/** 신뢰된 경계를 강제할 수 없는 실행 프로필이면 보류 이유를 준다. 열람할 수 있으면 null 이다. */
export function vaultRestoreProfileHold(profile: VaultRestoreProfile): VaultRestoreHold | null {
  if (profile.automation) return "profile_automation";
  if (profile.devRenderer) return "profile_dev_renderer";
  if (profile.hiddenWindows) return "profile_hidden_window";
  if (profile.agentWorking) return "agent_working";
  return null;
}

/** 게시본의 변경이 확인된 보류이다. 이때 재연결 검토를 할 수 있다. */
export function isReconnectableHold(reason: VaultRestoreHold): boolean {
  return reason === "published_copy_changed" || reason === "marker_moved" || reason === "marker_duplicated" || reason === "marker_missing";
}

const HOLD_TEXT: Record<VaultRestoreHold, string> = {
  profile_automation: "이 앱은 자동화 또는 디버깅 연결로 실행 중입니다. 이 실행 방식에서는 열람을 보류합니다.",
  profile_dev_renderer: "화면을 개발 서버에서 읽었습니다. 이 실행 방식에서는 열람을 보류합니다.",
  profile_hidden_window: "창을 숨기는 시험 방식으로 실행 중입니다. 이 실행 방식에서는 열람을 보류합니다.",
  agent_working: "이 앱에서 에이전트 턴이 실행 중입니다. 턴이 끝난 뒤에 다시 여세요.",
  os_locked: "운영체제 화면이 잠겨 있습니다. 화면을 잠금 해제한 뒤 다시 확인하세요.",
  os_lock_unavailable: "운영체제 화면 잠금 상태를 확인할 수 없습니다. 공유본만 볼 수 있으며, 상태 확인이 가능해진 뒤 다시 시도하세요.",
  os_lock_unsupported: "이 운영체제에서는 전용 복원 창의 화면 잠금 감지가 지원되지 않아 열람을 보류합니다. 공유본만 볼 수 있습니다.",
  system_suspended: "절전 상태입니다. 절전에서 돌아온 뒤 다시 확인하세요.",
  source_unavailable: "로컬 복원 경로를 열 수 없습니다. 공유본만 볼 수 있습니다.",
  platform_unsupported: "현재 환경에서는 신뢰할 수 있는 로컬 Vault 열람 경로를 사용할 수 없습니다. 공유본만 볼 수 있습니다.",
  confirmation_cancelled: "확인을 취소했습니다. 복원본을 열지 않았습니다.",
  locked: "Vault 잠금을 해제하지 못했습니다. 공유본만 볼 수 있습니다.",
  latest_read_failed: "최신 게시본을 읽지 못했습니다. 이전에 읽은 글로 복원하지 않습니다.",
  not_bound: "이 이슈에 연결된 복원 기록이 없습니다.",
  published_copy_changed: "게시본이 연결한 뒤에 바뀌었습니다. 재연결 검토가 필요합니다.",
  marker_moved: "가림 표식이 다른 자리로 옮겨졌습니다. 재연결 검토가 필요합니다.",
  marker_duplicated: "같은 가림 표식이 두 번 이상 있습니다. 재연결 검토가 필요합니다.",
  marker_missing: "가림 표식이 게시본에 없습니다. 재연결 검토가 필요합니다.",
  changed_during_review: "검토하는 동안 게시본이나 로컬 연결이 바뀌었습니다. 재연결을 확정하지 않았습니다.",
  mapping_rejected: "승인한 연결이 지금 게시본과 맞지 않습니다. 재연결을 확정하지 않았습니다.",
  record_unavailable: "복원 기록을 읽거나 쓰지 못했습니다.",
  write_outcome_unknown: "연결 저장이 완료됐을 수 있습니다. 다시 열어 결과를 확인하세요.",
};

const RELOCK_TEXT: Record<VaultRelockCause, string> = {
  user: "잠갔습니다.",
  window_closed: "창을 닫아서 잠갔습니다.",
  window_hidden: "창을 숨겨서 잠갔습니다.",
  renderer_changed: "화면 연결이 바뀌어 잠갔습니다. 현재 이슈에서 다시 여세요.",
  session_expired: "열람 시간이 끝나서 잠갔습니다.",
  os_lock: "화면이 잠겨서 잠갔습니다.",
  suspend: "절전으로 들어가서 잠갔습니다.",
  project_switch: "프로젝트를 바꿔서 잠갔습니다.",
  agent_working: "에이전트 턴이 시작되어 잠갔습니다.",
};

export type VaultRestoreAction = "unlock" | "lock" | "review";

/** 화면이 그리는 값이다. 상태마다 제목, 설명, 누를 수 있는 동작을 정한다. */
export interface VaultRestoreView {
  heading: string;
  detail: string;
  actions: VaultRestoreAction[];
}

export function vaultRestoreView(state: VaultRestoreViewState): VaultRestoreView {
  switch (state.phase) {
    case "locked":
      return {
        heading: "잠김",
        detail: state.cause ? `${RELOCK_TEXT[state.cause]} 다시 보려면 확인을 다시 거칩니다.` : "확인한 뒤에 복원본을 엽니다.",
        actions: ["unlock"],
      };
    case "opening":
      return { heading: "여는 중", detail: "사용자 확인과 최신 게시본 확인을 기다립니다.", actions: ["lock"] };
    case "reviewing":
      return { heading: "검토 준비 중", detail: "최신 공유본의 표식 위치를 확인하고 있습니다.", actions: ["lock"] };
    case "reconnecting":
      return { heading: "재연결 확인 중", detail: "운영체제 확인 창에서 고른 표식의 연결 변경을 확인하세요.", actions: ["lock"] };
    case "held":
      return {
        heading: "열람 보류",
        detail: HOLD_TEXT[state.reason] + (isReconnectableHold(state.reason) && state.canReconnect !== true ? " 이 열람 경로에서는 재연결을 지원하지 않습니다." : ""),
        actions: isReconnectableHold(state.reason) && state.canReconnect === true ? ["review", "lock"] : ["unlock"],
      };
    case "restored":
      return { heading: "로컬 복원본 · 읽기 전용", detail: "이 글은 이 창에만 있습니다. 창을 닫거나 시간이 끝나면 다시 잠깁니다.", actions: ["lock"] };
    case "review":
      return { heading: "재연결 검토", detail: "표식마다 지금 공유본의 자리를 고릅니다. 고르지 않은 표식은 표식으로 남습니다.", actions: ["lock"] };
  }
}

/** 같은 URL의 다른 저장소를 구분하기 위해 확인과 복원을 전체 좌표에 묶는다. */
export interface VaultRestoreTarget {
  workspace: string;
  issueUrl: string;
  issue: IssueEventReference;
}

export function isVaultRestoreTarget(value: unknown): value is VaultRestoreTarget {
  if (!record(value) || !fields(value, ["workspace", "issueUrl", "issue"]) ||
    !bounded(value.workspace, 4_096) || !absoluteWorkspace(value.workspace) ||
    !isVaultRestoreIssueUrl(value.issueUrl) || value.issueUrl.length > 4_096 || !record(value.issue)) return false;
  const issue = value.issue;
  return fields(issue, ["provider", "server_id", "repository_id", "issue_id", "project_scope"]) &&
    issue.provider === "github" && issue.server_id === "github.com" &&
    token(issue.repository_id) && token(issue.issue_id) && record(issue.project_scope) &&
    fields(issue.project_scope, ["workspace_dir"]) && issue.project_scope.workspace_dir === value.workspace;
}

export function copyVaultRestoreTarget(value: VaultRestoreTarget): VaultRestoreTarget {
  return {
    workspace: value.workspace,
    issueUrl: value.issueUrl,
    issue: {
      provider: value.issue.provider,
      server_id: value.issue.server_id,
      repository_id: value.issue.repository_id,
      issue_id: value.issue.issue_id,
      project_scope: { workspace_dir: value.workspace },
    },
  };
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function fields(value: Record<string, unknown>, allowed: readonly string[]): boolean {
  return Object.keys(value).every((key) => allowed.includes(key));
}
function bounded(value: unknown, maximum: number): value is string {
  return typeof value === "string" && value.length > 0 && value.trim() === value && new TextEncoder().encode(value).length <= maximum &&
    !/[\u0000-\u001f\u007f-\u009f]/.test(value);
}
function token(value: unknown): value is string {
  return typeof value === "string" && /^[A-Za-z0-9_=-]{1,200}$/.test(value);
}
function absoluteWorkspace(value: string): boolean {
  return value.startsWith("/") || /^[A-Za-z]:[\\/]/.test(value) || /^\\\\[^\\]+\\[^\\]+/.test(value);
}

/** 전용 renderer의 선택만 복사한다. 추가 필드·중복·제시하지 않은 후보를 승인으로 보내지 않는다. */
export function approvedVaultMappings(review: VaultReconnectReview, mappings: unknown): VaultApprovedMapping[] | null {
  if (!Array.isArray(mappings) || mappings.length > review.markers.length) return null;
  const offered = new Map(review.markers.map((marker) => [marker.marker, marker]));
  const selected = new Set<string>();
  const approved: VaultApprovedMapping[] = [];
  for (const mapping of mappings) {
    if (!record(mapping) || Object.keys(mapping).length !== 3 || !fields(mapping, ["marker", "field", "position"]) ||
      typeof mapping.marker !== "string" || (mapping.field !== "title" && mapping.field !== "body") ||
      typeof mapping.position !== "number" || !Number.isSafeInteger(mapping.position) || mapping.position < 0 ||
      selected.has(mapping.marker)) return null;
    const marker = offered.get(mapping.marker);
    if (!marker?.candidates.some((place) => place.field === mapping.field && place.position === mapping.position)) return null;
    selected.add(mapping.marker);
    approved.push({ marker: mapping.marker, field: mapping.field, position: mapping.position });
  }
  return approved;
}
