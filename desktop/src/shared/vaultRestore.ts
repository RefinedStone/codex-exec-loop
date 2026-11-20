// 로컬 복원본 창의 계약이다 (#2427). 이 창만 쓰는 채널과 상태이다. 일반 메시지 버스(`bridge.ts` 의 `window.akra`)와
// 대화 IPC 에 넣지 않는다. 복원 평문은 main 에서 이 창의 renderer 로만 간다. Gateway 와 Kernel 프로토콜로 가지 않는다.

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
  // 이 앱에 로컬 복원 경로가 연결되지 않았다.
  | "source_unavailable"
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
  | "record_unavailable";

/** 다시 잠근 이유이다. */
export type VaultRelockCause =
  | "user"
  | "window_closed"
  | "window_hidden"
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
  | { phase: "held"; reason: VaultRestoreHold }
  | { phase: "restored"; title: string; body: string; expiresAt: number }
  | { phase: "review"; review: VaultReconnectReview };

/** 로컬 복원본 창의 preload 가 노출하는 함수이다. 이것만 있다: `window.akra` 는 이 창에 없다. */
export interface VaultRestoreBridge {
  /** 수신 함수를 등록했다고 알린다. 처음이면 main 이 확인을 시작한다. 다시 오면 (화면을 다시 불러옴) 잠근다. */
  ready(): void;
  onState(listener: (state: VaultRestoreViewState) => void): () => void;
  /** 다시 사용자 확인을 거쳐 연다. */
  unlock(): void;
  lock(): void;
  /** 재연결 검토의 값을 읽는다. */
  review(): void;
  reconnect(mappings: VaultApprovedMapping[]): void;
}

/** 일반 창이 쓰는 함수이다. 창을 열고 닫게 할 뿐이다. 복원본의 상태와 글은 받지 못한다. */
export interface VaultRestoreOpenerBridge {
  /** 이슈의 로컬 복원본 창을 연다. 열람은 그 창에서 사용자가 확인한 뒤에만 된다. */
  open(issueUrl: string): Promise<void>;
  /** 이 창의 프로젝트(열린 스레드의 폴더)가 바뀌었다. 이 창이 연 복원본을 다시 잠근다. */
  projectChanged(): void;
}

/** 로컬 복원본을 여는 대상이다: github.com 의 이슈 주소 하나이다. */
export function isVaultRestoreIssueUrl(url: unknown): url is string {
  return typeof url === "string" && /^https:\/\/github\.com\/[\w.-]+\/[\w.-]+\/issues\/\d+$/.test(url);
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
  source_unavailable: "이 앱에 로컬 복원 경로가 연결되지 않았습니다. 공유본만 볼 수 있습니다.",
  confirmation_cancelled: "확인을 취소했습니다. 복원본을 열지 않았습니다.",
  locked: "Vault 잠금을 해제하지 못했습니다. 공유본만 볼 수 있습니다.",
  latest_read_failed: "최신 게시본을 읽지 못했습니다. 이전에 읽은 글로 복원하지 않습니다.",
  not_bound: "이 이슈에 연결된 복원 기록이 없습니다.",
  published_copy_changed: "게시본이 연결한 뒤에 바뀌었습니다. 재연결 검토가 필요합니다.",
  marker_moved: "가림 표식이 다른 자리로 옮겨졌습니다. 재연결 검토가 필요합니다.",
  marker_duplicated: "같은 가림 표식이 두 번 이상 있습니다. 재연결 검토가 필요합니다.",
  marker_missing: "가림 표식이 게시본에 없습니다. 재연결 검토가 필요합니다.",
  changed_during_review: "검토하는 동안 게시본이 다시 바뀌었습니다. 재연결을 확정하지 않았습니다.",
  mapping_rejected: "승인한 연결이 지금 게시본과 맞지 않습니다. 재연결을 확정하지 않았습니다.",
  record_unavailable: "복원 기록을 읽거나 쓰지 못했습니다.",
};

const RELOCK_TEXT: Record<VaultRelockCause, string> = {
  user: "잠갔습니다.",
  window_closed: "창을 닫아서 잠갔습니다.",
  window_hidden: "창을 숨겨서 잠갔습니다.",
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
    case "held":
      return {
        heading: "열람 보류",
        detail: HOLD_TEXT[state.reason],
        actions: isReconnectableHold(state.reason) ? ["review", "lock"] : ["unlock"],
      };
    case "restored":
      return { heading: "로컬 복원본 · 읽기 전용", detail: "이 글은 이 창에만 있습니다. 창을 닫거나 시간이 끝나면 다시 잠깁니다.", actions: ["lock"] };
    case "review":
      return { heading: "재연결 검토", detail: "표식마다 지금 공유본의 자리를 고릅니다. 고르지 않은 표식은 표식으로 남습니다.", actions: ["lock"] };
  }
}
