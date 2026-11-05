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
} as const;

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
}
