// 접속이 맺어질 때마다 initialize 를 다시 하고 Gateway 정보와 작업 폴더 목록을 읽는다 (P2 §4.5).
// 스레드를 열면 `thread/resume` 으로 기록을 받고, 그 스레드의 알림을 화면 프레임마다 묶어 반영한다 (P3 §6.1, §7).
// 다시 연결되면 열려 있던 스레드를 다시 resume 한다 (P3 §6.5).
// 입력(새 턴, steer, 중단, 새 스레드)과 승인 요청 응답도 여기서 보낸다 (P3 §6.2, §6.4). 턴을 만드는 요청은
// 사용자가 보낸 메시지에서만 나간다.
import type { AkraBridge, RemoteHostBridge } from "../../shared/bridge";
import type { InitializeParams } from "../../shared/protocol/generated/InitializeParams";
import type { Thread } from "../../shared/protocol/generated/v2/Thread";
import type { ThreadListResponse } from "../../shared/protocol/generated/v2/ThreadListResponse";
import type { ThreadReadResponse } from "../../shared/protocol/generated/v2/ThreadReadResponse";
import type { ThreadResumeResponse } from "../../shared/protocol/generated/v2/ThreadResumeResponse";
import type { ThreadStartResponse } from "../../shared/protocol/generated/v2/ThreadStartResponse";
import type { ThreadStatus } from "../../shared/protocol/generated/v2/ThreadStatus";
import type { ThreadTurnsListResponse } from "../../shared/protocol/generated/v2/ThreadTurnsListResponse";
import type { UserInput } from "../../shared/protocol/generated/v2/UserInput";
import { GENERATED_CODEX_VERSION } from "../../shared/protocol/version";
import { type ApprovalDecision, approvalKey, toPendingApproval } from "../conversation/approvals";
import { FrameBatcher, type FrameScheduler } from "../conversation/frameBatcher";
import {
  applyNotifications,
  isTurnInProgress,
  notificationThreadId,
  prependOlderTurns,
  transcriptFromThread,
} from "../conversation/transcript";
import { KernelConnection } from "../kernel/kernelConnection";
import { KERNEL_METHOD } from "../kernel/protocol";
import { JsonRpcClient, JsonRpcError, type JsonRpcId, NO_RESPONSE } from "../rpc/jsonRpcClient";
import { checkCodexVersion } from "./codexVersion";
import { type DotSummary, type GatewayInfo, type SessionStore, sortFolders, type WorkspaceFolder, type WorkspaceThread } from "./desktopStore";

const THREAD_LIST_LIMIT = 100;
/** 스레드를 열 때와 "이전 턴 더 보기" 한 번에 읽는 턴 수이다. 가장 큰 실측 스레드에서 10턴은 약 11 MB, 270ms 였다. */
const TURN_PAGE_SIZE = 10;
/** 첫 턴 전의 새 스레드에 대한 app-server 오류이다 (2026-11-05 실측: `no rollout found for thread id …`). */
const NO_ROLLOUT_ERROR = /no rollout found/i;
/** shortcut: 최대 50 페이지(5,000 스레드)까지만 읽는다. 더 많으면 사이드바를 필요할 때 더 읽는 방식으로 바꾼다. */
const THREAD_LIST_MAX_PAGES = 50;

interface ResumeLoading {
  threadId: string;
  buffer: QueuedNotification[];
}
const UNTITLED_THREAD = "(제목 없음)";

interface QueuedNotification {
  method: string;
  params: unknown;
}

export function initializeParams(clientVersion: string): InitializeParams {
  return {
    clientInfo: { name: "akra-desktop", title: "Akra Desktop", version: clientVersion },
    capabilities: { experimentalApi: false, requestAttestation: false },
  };
}

/**
 * 클라우드 에이전트(dot)가 만든 스레드인지이다. 2026-11-14 실계정에서 dot 의 작업 스레드는 `aeon_child`, 사용자가
 * 만든 클라우드 스레드는 `user` 였다.
 */
export function isAgentThreadSource(threadSource: string | null | undefined): boolean {
  return typeof threadSource === "string" && threadSource.startsWith("aeon");
}

/** 사이드바에 필요한 값만 고른다. */
function workspaceThread(thread: Thread, title: string): WorkspaceThread {
  return {
    id: thread.id,
    title,
    updatedAt: thread.updatedAt,
    branch: thread.gitInfo?.branch ?? null,
    failed: thread.status?.type === "systemError",
    ...(isAgentThreadSource(thread.threadSource) ? { agentThread: true } : {}),
  };
}

export function groupThreadsByCwd(threads: readonly Thread[]): WorkspaceFolder[] {
  const folders = new Map<string, WorkspaceFolder>();
  // rollout 전체 검사는 페이지 사이에 같은 스레드를 다시 줄 수 있다 (2026-11-06 실측: 306행 중 고유 292개).
  const seen = new Set<string>();
  for (const thread of threads) {
    if (seen.has(thread.id)) continue;
    seen.add(thread.id);
    const folder = folders.get(thread.cwd) ?? { cwd: thread.cwd, threads: [] };
    folder.threads.push(workspaceThread(thread, thread.name ?? (thread.preview.trim() || UNTITLED_THREAD)));
    folders.set(thread.cwd, folder);
  }
  return sortFolders([...folders.values()]);
}

/** 목록의 진행 중 스레드에, 목록을 읽는 동안 받은 상태 변경을 덮어 최종 목록을 만든다. */
export function activeThreadIds(threads: readonly Thread[], changes: ReadonlyMap<string, boolean>): string[] {
  const active = new Set(threads.filter((thread) => thread.status?.type === "active").map((thread) => thread.id));
  for (const [threadId, isActive] of changes) {
    if (isActive) {
      active.add(threadId);
    } else {
      active.delete(threadId);
    }
  }
  return [...active];
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** 패널이 직접 부를 수 있는 Gateway 메서드이다. app-server 메서드와 커널 명령은 이 길로 보내지 않는다. */
const GATEWAY_SERVICE_PREFIXES = ["akra/mcp/", "akra/context/", "akra/dots/"];
/** 원격 호스트의 dot 목록이다 (D-19). dots 가 꺼진 Gateway 는 모르는 method 로 답한다. */
const DOTS_LIST_METHOD = "akra/dots/list";

/** 세션이 쓰는 메시지 버스이다. `local` 은 `window.akra` 이고, 원격 호스트는 `remoteHostBridge` 이다. */
export type SessionBridge = Pick<AkraBridge, "send" | "onMessage" | "onConnectionState" | "ready">;

/** 원격 호스트 하나의 메시지 버스이다 (plan 25). `ready` 가 그 호스트의 접속을 연다. */
export function remoteHostBridge(remote: RemoteHostBridge, host: string): SessionBridge {
  return {
    send: (text) => remote.send(host, text),
    onMessage: (listener) => remote.onMessage((from, text) => from === host && listener(text)),
    onConnectionState: (listener) => remote.onConnectionState((from, state) => from === host && listener(state)),
    ready: () => remote.open(host),
  };
}

const READ_ONLY_ERROR = "원격 호스트의 대화는 읽기 전용입니다.";

export class DesktopSession {
  private readonly client: JsonRpcClient;
  private readonly batcher: FrameBatcher<QueuedNotification>;
  private readonly unsubscribers: Array<() => void> = [];
  /** resume 응답을 기다리는 동안 그 스레드에 온 알림이다. 응답을 반영한 뒤 이어서 반영한다. */
  private loading: ResumeLoading | null = null;
  /** initialize 가 thread/list 를 읽는 동안 받은 진행 상태 변경이다 (스레드 id → 진행 중). */
  private statusChangesWhileListing: Map<string, boolean> | null = null;
  private connected = false;
  /**
   * 원격 세션에서 사용자가 보내기를 허락했는지이다 (plan 25 G2, "세션 한 번 허용", D-19). 허락 전에는 읽기만
   * 한다. 허락하면 열린 스레드를 thread/resume 으로 다시 구독해 실시간으로 받고, 턴을 보낼 수 있다.
   */
  private sendingAllowed = false;
  /** 접속마다 늘어난다. 이전 접속에서 늦게 끝난 전체 검사는 반영하지 않는다. */
  private listGeneration = 0;
  /** 사용자의 답을 기다리는 승인 요청의 응답 함수이다. 키는 요청 id 이다. */
  private readonly approvalAnswers = new Map<string, (answer: unknown) => void>();
  /** 커널 패널의 연결이다 (plan 18 K3-6). 같은 Gateway 접속을 쓴다. */
  private readonly kernel: KernelConnection;

  constructor(
    private readonly bridge: SessionBridge,
    private readonly store: SessionStore,
    private readonly clientVersion: string,
    scheduler?: FrameScheduler,
    /** 원격 호스트의 세션이다 (plan 25). 기본은 읽기만 한다: 턴, 새 스레드, 승인 응답을 보내지 않는다. */
    private readonly readOnly = false,
  ) {
    this.client = new JsonRpcClient((text) => bridge.send(text));
    this.batcher = new FrameBatcher((batch) => this.store.updateTranscript((transcript) => applyNotifications(transcript, batch)), scheduler);
    this.client.setServerRequestHandler((method, params, id) => this.handleServerRequest(method, params, id));
    this.kernel = new KernelConnection(this.client, store);
  }

  start(): void {
    this.unsubscribers.push(this.bridge.onMessage((text) => this.client.handleText(text)));
    this.unsubscribers.push(this.client.onNotification((method, params) => this.handleNotification(method, params)));
    this.unsubscribers.push(
      this.bridge.onConnectionState((state) => {
        this.store.setConnection(state);
        this.connected = state.phase === "connected";
        if (this.connected) {
          void this.initialize();
        } else {
          this.client.reset(state.phase);
          this.kernel.connectionChanged(false);
          // 끊긴 접속의 승인 요청에는 답할 수 없다. 다시 구독하면 app-server 가 다시 보낸다 (replay).
          this.dropPendingApprovals();
        }
      }),
    );
    // 수신 함수를 모두 등록한 뒤에 접속을 시작하게 한다 (빠른 접속의 connected 알림을 놓치지 않는다).
    this.bridge.ready();
  }

  stop(): void {
    for (const unsubscribe of this.unsubscribers.splice(0)) {
      unsubscribe();
    }
    this.batcher.clear();
    this.client.reset("closed");
    this.dropPendingApprovals();
  }

  /** 커널 패널이 보여 줄 폴더를 정한다. 그 폴더의 저장소 커널에 붙고, null 이면 뗀다. */
  showKernel(workspace: string | null): Promise<void> {
    return this.kernel.show(workspace);
  }

  /** 커널 패널의 상태를 다시 읽는다. */
  refreshKernel(): Promise<void> {
    return this.kernel.refresh();
  }

  /** 업무 패널이 같은 Gateway 접속으로 보내는 요청이다: MCP hub(plan 21)와 작업 컨텍스트(plan 22). */
  requestGatewayService<T>(method: string, params: unknown): Promise<T> {
    if (!GATEWAY_SERVICE_PREFIXES.some((prefix) => method.startsWith(prefix))) {
      return Promise.reject(new Error(`${method} 는 패널이 쓸 수 있는 Gateway 요청이 아닙니다.`));
    }
    if (!this.connected) {
      return Promise.reject(new Error("Gateway 에 연결되어 있지 않습니다."));
    }
    return this.client.request<T>(method, params);
  }

  /** 메시지를 보낸다. 진행 중인 턴이 있으면 steer, 없으면 새 턴이다 (P3 §6.2). */
  async sendMessage(text: string): Promise<void> {
    this.assertWritable();
    const threadId = this.store.getOpenThreadId();
    const trimmed = text.trim();
    if (!threadId || trimmed.length === 0) {
      return;
    }
    const input: UserInput[] = [{ type: "text", text: trimmed, text_elements: [] }];
    const transcript = this.store.getTranscript();
    const activeTurn = isTurnInProgress(transcript) ? transcript?.turns.at(-1) : undefined;
    if (activeTurn) {
      await this.client.request("turn/steer", { threadId, input, expectedTurnId: activeTurn.id });
    } else {
      await this.client.request("turn/start", { threadId, input });
    }
  }

  /** 진행 중인 턴을 중단한다. */
  async interrupt(): Promise<void> {
    this.assertWritable();
    const threadId = this.store.getOpenThreadId();
    const transcript = this.store.getTranscript();
    const turn = transcript?.turns.at(-1);
    if (!threadId || !turn || turn.status !== "inProgress") {
      return;
    }
    await this.client.request("turn/interrupt", { threadId, turnId: turn.id });
  }

  /** 작업 폴더에서 새 스레드를 시작하고 연다. 첫 메시지는 열린 스레드에 보낸다 (P3 §6.2). */
  async startThread(cwd: string): Promise<string> {
    this.assertWritable();
    const response = await this.client.request<ThreadStartResponse>("thread/start", { cwd });
    const transcript = transcriptFromThread(response.thread);
    this.batcher.clear();
    this.store.setOpenThread(response.thread.id);
    this.store.setTranscript(transcript);
    this.store.addThreadToFolder(response.thread.cwd, workspaceThread(response.thread, transcript.title));
    return response.thread.id;
  }

  /** 승인 요청에 답한다. 다른 창이 먼저 답했으면 이미 닫혀 있으므로 아무것도 하지 않는다. */
  answerApproval(requestId: JsonRpcId, decision: ApprovalDecision): void {
    const key = approvalKey(requestId);
    const answer = this.approvalAnswers.get(key);
    this.approvalAnswers.delete(key);
    this.store.removeApproval(requestId);
    answer?.({ decision });
  }

  /** 이 세션에서 지금 보낼 수 있는지이다. 로컬은 늘 보낼 수 있고, 원격은 사용자가 허락한 뒤에만 보낸다. */
  canSend(): boolean {
    return !this.readOnly || this.sendingAllowed;
  }

  /**
   * 원격 세션에서 보내기를 허락한다 (plan 25 "세션 한 번 허용"). 열린 스레드를 thread/resume 으로 다시 구독해
   * 실시간으로 받는다. 로컬 세션이나 이미 허락한 세션에서는 아무것도 하지 않는다.
   */
  async allowSending(): Promise<void> {
    if (!this.readOnly || this.sendingAllowed) return;
    this.sendingAllowed = true;
    await this.resumeOpenThread();
  }

  private assertWritable(): void {
    if (!this.canSend()) {
      throw new Error(READ_ONLY_ERROR);
    }
  }

  private handleServerRequest(method: string, params: unknown, id: JsonRpcId): Promise<unknown> {
    const approval = this.canSend() ? toPendingApproval(method, params, id) : null;
    if (!approval) {
      return Promise.reject(new JsonRpcError(-32601, `Akra Desktop does not handle ${method} yet`));
    }
    this.store.addApproval(approval);
    return new Promise((resolve) => this.approvalAnswers.set(approvalKey(id), resolve));
  }

  /** 승인 요청이 다른 곳에서 닫혔다. 화면에서 지우고 응답은 보내지 않는다 (Q-03). */
  private resolveApprovalElsewhere(requestId: JsonRpcId): void {
    const key = approvalKey(requestId);
    const answer = this.approvalAnswers.get(key);
    this.approvalAnswers.delete(key);
    this.store.removeApproval(requestId);
    answer?.(NO_RESPONSE);
  }

  private dropPendingApprovals(): void {
    for (const answer of this.approvalAnswers.values()) {
      answer(NO_RESPONSE);
    }
    this.approvalAnswers.clear();
    this.store.clearApprovals();
  }

  /** 스레드를 연다. 이전 스레드는 진행 중인 턴이 없으면 구독을 끝낸다. */
  async openThread(threadId: string): Promise<void> {
    const previousId = this.store.getOpenThreadId();
    if (this.canSend() && previousId && previousId !== threadId && !isTurnInProgress(this.store.getTranscript())) {
      void this.client.request("thread/unsubscribe", { threadId: previousId }).catch(() => undefined);
    }
    this.batcher.clear();
    this.store.setOpenThread(threadId);
    await this.resumeOpenThread();
  }

  private async initialize(): Promise<void> {
    this.client.reset("reconnected");
    // 커널 붙음은 접속마다 따로이다. 대기 중 요청을 정리한 뒤 다시 붙는다.
    this.kernel.connectionChanged(true);
    try {
      await this.client.request("initialize", initializeParams(this.clientVersion));
      this.client.notify("initialized");
      const gateway = await this.client.request<GatewayInfo>("akra/gateway/info");
      // 목록을 읽는 동안 온 상태 변경은 목록의 상태보다 새롭다. 따로 모았다가 목록 결과 위에 덮는다.
      const statusChanges = new Map<string, boolean>();
      this.statusChangesWhileListing = statusChanges;
      const generation = ++this.listGeneration;
      const threads = await this.listAllThreads(true).finally(() => {
        this.statusChangesWhileListing = null;
      });
      this.store.setSession({
        gateway,
        versionCheck: checkCodexVersion(GENERATED_CODEX_VERSION, gateway.codexVersion),
        folders: groupThreadsByCwd(threads),
      });
      // 이 창이 아직 구독하지 않은 스레드도 목록의 상태로 진행 중 표시를 채운다.
      this.store.setActiveThreads(activeThreadIds(threads, statusChanges));
      // 원격 호스트는 목록을 한 번만 읽는다 (plan 25 §8).
      if (!this.readOnly) {
        void this.addThreadsFromRolloutScan(generation);
      } else {
        void this.loadDots();
      }
      if (this.store.getOpenThreadId()) {
        await this.resumeOpenThread();
      }
    } catch (error) {
      this.store.setSessionError(errorMessage(error));
    }
  }

  /** 원격 호스트의 dot 목록을 한 번 읽는다. 실패하면(dots 가 없는 호스트 등) 목록을 비운다. 대화 목록은 그대로 보인다. */
  private async loadDots(): Promise<void> {
    try {
      const response = await this.client.request<{ dots: DotSummary[] }>(DOTS_LIST_METHOD);
      this.store.setDots(response.dots);
    } catch {
      this.store.setDots([]);
    }
  }

  /**
   * 상태 DB 에 아직 반영되지 않은 스레드를 rollout 전체 검사로 찾아 사이드바에 더한다. 화면은 상태 DB 목록으로
   * 먼저 보여 준다. 2026-11-06 실측 (rollout 1,465개, 18 GB): 상태 DB 67ms, 전체 검사 27s. 검사 중에는 스레드
   * 열기가 0.8s 에서 3.4s 로 느려진다.
   */
  private async addThreadsFromRolloutScan(generation: number): Promise<void> {
    try {
      const threads = await this.listAllThreads(false);
      if (generation === this.listGeneration) {
        this.store.mergeFolders(groupThreadsByCwd(threads));
      }
    } catch {
      // 상태 DB 목록은 이미 보인다. 접속이 끊겨서 실패했으면 다시 연결할 때 다시 검사한다.
    }
  }

  /**
   * `nextCursor` 를 따라 모든 페이지를 읽는다. 첫 페이지만 쓰면 오래된 작업 폴더가 목록에서 빠진다.
   * stateDbOnly 이면 rollout 파일을 검사하지 않고 상태 DB 만 읽는다.
   */
  private async listAllThreads(stateDbOnly: boolean): Promise<Thread[]> {
    const threads: Thread[] = [];
    let cursor: string | null = null;
    for (let page = 0; page < THREAD_LIST_MAX_PAGES; page += 1) {
      const response: ThreadListResponse = await this.client.request<ThreadListResponse>("thread/list", {
        limit: THREAD_LIST_LIMIT,
        useStateDbOnly: stateDbOnly,
        ...(cursor ? { cursor } : {}),
      });
      threads.push(...response.data);
      cursor = response.nextCursor;
      if (!cursor) {
        break;
      }
    }
    return threads;
  }

  private async resumeOpenThread(): Promise<void> {
    const threadId = this.store.getOpenThreadId();
    if (!threadId || !this.connected) {
      return;
    }
    // 요청마다 버퍼를 따로 둔다. 빠르게 스레드를 바꿔 resume 이 겹쳐도 먼저 끝난 요청이 다음 요청의 버퍼를 지우지 않는다.
    const loading: ResumeLoading = { threadId, buffer: [] };
    this.loading = loading;
    try {
      // 긴 스레드의 전체 resume 은 수십 MB 가 된다. 턴 없이 구독만 하고 최근 턴부터 페이지로 읽는다.
      // 허락 전 원격 호스트는 구독하지 않고 thread/read 로 한 번만 읽는다. 허락하면 로컬과 같이 thread/resume 으로
      // 구독해 실시간으로 받는다 (plan 25 "세션 한 번 허용").
      const response = this.canSend()
        ? await this.client.request<ThreadResumeResponse>("thread/resume", { threadId, excludeTurns: true })
        : await this.client.request<ThreadReadResponse>("thread/read", { threadId, includeTurns: false });
      if (this.store.getOpenThreadId() !== threadId) {
        return;
      }
      let transcript = transcriptFromThread(response.thread);
      // 서버가 턴을 함께 보냈으면(예: 녹화 재생) 그대로 쓴다.
      if (transcript.turns.length === 0) {
        const page = await this.readTurnPage(threadId, null);
        if (this.store.getOpenThreadId() !== threadId) {
          return;
        }
        transcript = prependOlderTurns(transcript, page.data, page.nextCursor);
      }
      this.store.setTranscript(applyNotifications(transcript, loading.buffer));
    } catch (error) {
      if (this.store.getOpenThreadId() === threadId) {
        this.store.setTranscript(null, errorMessage(error));
      }
    } finally {
      if (this.loading === loading) {
        this.loading = null;
      }
    }
  }

  /** 열린 스레드의 이전 턴을 한 페이지 더 읽어 앞에 붙인다. */
  async loadOlderTurns(): Promise<void> {
    const transcript = this.store.getTranscript();
    const cursor = transcript?.olderTurnsCursor;
    if (!transcript || !cursor) {
      return;
    }
    const page = await this.readTurnPage(transcript.threadId, cursor);
    if (this.store.getOpenThreadId() === transcript.threadId) {
      this.store.updateTranscript((current) => prependOlderTurns(current, page.data, page.nextCursor));
    }
  }

  /** 최신순으로 턴 한 페이지를 읽는다. 아직 기록이 없는 새 스레드는 빈 페이지로 본다. */
  private async readTurnPage(threadId: string, cursor: string | null): Promise<ThreadTurnsListResponse> {
    try {
      return await this.client.request<ThreadTurnsListResponse>("thread/turns/list", {
        threadId,
        limit: TURN_PAGE_SIZE,
        sortDirection: "desc",
        itemsView: "full",
        ...(cursor ? { cursor } : {}),
      });
    } catch (error) {
      // 첫 턴 전의 새 스레드에는 기록(rollout)이 없다. 이 오류만 빈 기록으로 보고, 나머지는 화면에 보여 준다.
      if (cursor === null && error instanceof JsonRpcError && NO_ROLLOUT_ERROR.test(error.message)) {
        return { data: [], nextCursor: null, backwardsCursor: null };
      }
      throw error;
    }
  }

  private handleNotification(method: string, params: unknown): void {
    if (method === KERNEL_METHOD.event) {
      this.kernel.handleEvent(params);
      return;
    }
    const threadId = notificationThreadId(params);
    if (!threadId) {
      return;
    }
    if (method === "thread/status/changed") {
      const status = (params as { status: ThreadStatus }).status.type;
      const active = status === "active";
      this.statusChangesWhileListing?.set(threadId, active);
      this.store.setThreadActive(threadId, active);
      this.store.setThreadFailed(threadId, status === "systemError");
    }
    if (method === "serverRequest/resolved") {
      this.resolveApprovalElsewhere((params as { requestId: JsonRpcId }).requestId);
    }
    if (threadId !== this.store.getOpenThreadId()) {
      return;
    }
    if (this.loading?.threadId === threadId) {
      this.loading.buffer.push({ method, params });
    } else {
      this.batcher.push({ method, params });
    }
  }
}

/** 메시지 버스와 세션을 잇는다. 반환한 함수로 연결을 끊는다. */
export function startDesktopSession(bridge: AkraBridge, store: SessionStore, clientVersion: string): () => void {
  const session = new DesktopSession(bridge, store, clientVersion);
  session.start();
  return () => session.stop();
}
