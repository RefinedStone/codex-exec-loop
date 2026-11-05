// main 과 renderer 가 공유하는 메시지 버스 계약이다. main 은 메시지를 해석하지 않고 문자열 그대로 전달한다.

export type ConnectionPhase = "connecting" | "connected" | "disconnected";

export interface ConnectionState {
  phase: ConnectionPhase;
  /** Gateway 접속 주소 (unix 소켓 경로 또는 Windows named pipe 이름) */
  endpoint: string;
  /** 다음 재접속까지 남은 시간 (밀리초). 연결 중이거나 연결되었으면 없다. */
  retryInMs?: number;
  /** 마지막 실패 이유 */
  detail?: string;
}

export const IPC_CHANNELS = {
  send: "akra:send",
  message: "akra:message",
  connectionState: "akra:connection-state",
  ready: "akra:ready",
  chooseFolder: "akra:choose-folder",
  gitStatus: "akra:git-status",
  gitFileDiff: "akra:git-file-diff",
} as const;

/** 작업 트리의 바뀐 파일 하나이다 (`git status --porcelain=v1`). */
export interface GitChangedFile {
  /** 작업 폴더 기준 경로이다. untracked 폴더는 `/` 로 끝난다. */
  path: string;
  /** 이름을 바꾼 파일의 이전 경로이다. */
  originalPath: string | null;
  /** porcelain 의 두 글자 상태 (예: ` M`, `A `, `??`) */
  status: string;
  untracked: boolean;
  directory: boolean;
  /** HEAD 대비 줄 수이다. 새 파일, 바이너리, 첫 커밋 전에는 null 이다. */
  added: number | null;
  removed: number | null;
}

export type GitStatusResult =
  | { ok: true; branch: string | null; files: GitChangedFile[]; truncated: boolean }
  | { ok: false; message: string };

export type GitFileDiff =
  | { kind: "gitDiff"; text: string; truncated: boolean }
  | { kind: "content"; text: string; truncated: boolean }
  | { kind: "binary" };

/** preload 가 `window.akra` 로 노출하는 함수 전부이다. */
export interface AkraBridge {
  /** JSON-RPC 메시지 한 개를 Gateway 로 보낸다. */
  send(text: string): void;
  onMessage(listener: (text: string) => void): () => void;
  onConnectionState(listener: (state: ConnectionState) => void): () => void;
  /**
   * 수신 함수를 모두 등록했다고 main 에 알린다. main 은 이때 Gateway 접속을 (다시) 연다. 그래서 빠른 접속의
   * `connected` 알림을 renderer 가 놓치지 않는다.
   */
  ready(): void;
  /** 시스템 폴더 선택 대화상자를 연다. 고르지 않으면 null 이다. */
  chooseFolder(): Promise<string | null>;
  /** 작업 폴더의 바뀐 파일 목록을 읽는다 (R1). */
  gitStatus(cwd: string): Promise<GitStatusResult>;
  /** 바뀐 파일 하나의 diff 를 읽는다. 새 파일은 내용을 돌려준다. */
  gitFileDiff(cwd: string, path: string, untracked: boolean): Promise<GitFileDiff>;
}
