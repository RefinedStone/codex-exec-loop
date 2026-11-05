// 접속이 맺어질 때마다 initialize 를 다시 하고 Gateway 정보와 작업 폴더 목록을 읽는다 (P2 §4.5).
// 스레드를 열면 `thread/resume` 으로 기록을 받고, 그 스레드의 알림을 화면 프레임마다 묶어 반영한다 (P3 §6.1, §7).
// 다시 연결되면 열려 있던 스레드를 다시 resume 한다 (P3 §6.5).
// 입력(새 턴, steer, 중단, 새 스레드)과 승인 요청 응답도 여기서 보낸다 (P3 §6.2, §6.4). 턴을 만드는 요청은
// 사용자가 보낸 메시지에서만 나간다.
import type { AkraBridge } from "../../shared/bridge";
import type { InitializeParams } from "../../shared/protocol/generated/InitializeParams";
import type { Thread } from "../../shared/protocol/generated/v2/Thread";
import type { ThreadListResponse } from "../../shared/protocol/generated/v2/ThreadListResponse";
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
  transcriptFromResume,
  transcriptFromThread,
} from "../conversation/transcript";
import { JsonRpcClient, JsonRpcError, type JsonRpcId, NO_RESPONSE } from "../rpc/jsonRpcClient";
import { checkCodexVersion } from "./codexVersion";
import type { GatewayInfo, SessionStore, WorkspaceFolder } from "./desktopStore";

const THREAD_LIST_LIMIT = 100;
/** 스레드를 열 때와 "이전 턴 더 보기" 한 번에 읽는 턴 수이다. 가장 큰 실측 스레드에서 10턴은 약 11 MB, 270ms 였다. */
const TURN_PAGE_SIZE = 10;
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

export function groupThreadsByCwd(threads: readonly Thread[]): WorkspaceFolder[] {
  const folders = new Map<string, WorkspaceFolder>();
  for (const thread of threads) {
    const folder = folders.get(thread.cwd) ?? { cwd: thread.cwd, threads: [] };
    folder.threads.push({
      id: thread.id,
      title: thread.name ?? (thread.preview.trim() || UNTITLED_THREAD),
      updatedAt: thread.updatedAt,
    });
    folders.set(thread.cwd, folder);
  }
  const sorted = [...folders.values()];
  for (const folder of sorted) {
    folder.threads.sort((left, right) => right.updatedAt - left.updatedAt);
  }
  const latest = (folder: WorkspaceFolder) => folder.threads[0]?.updatedAt ?? 0;
  return sorted.sort((left, right) => latest(right) - latest(left));
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export class DesktopSession {
  private readonly client: JsonRpcClient;
  private readonly batcher: FrameBatcher<QueuedNotification>;
  private readonly unsubscribers: Array<() => void> = [];
  /** resume 응답을 기다리는 동안 그 스레드에 온 알림이다. 응답을 반영한 뒤 이어서 반영한다. */
  private loading: ResumeLoading | null = null;
  private connected = false;
  /** 사용자의 답을 기다리는 승인 요청의 응답 함수이다. 키는 요청 id 이다. */
  private readonly approvalAnswers = new Map<string, (answer: unknown) => void>();

  constructor(
    private readonly bridge: AkraBridge,
    private readonly store: SessionStore,
    private readonly clientVersion: string,
    scheduler?: FrameScheduler,
  ) {
    this.client = new JsonRpcClient((text) => bridge.send(text));
    this.batcher = new FrameBatcher((batch) => this.store.updateTranscript((transcript) => applyNotifications(transcript, batch)), scheduler);
    this.client.setServerRequestHandler((method, params, id) => this.handleServerRequest(method, params, id));
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

  /** 메시지를 보낸다. 진행 중인 턴이 있으면 steer, 없으면 새 턴이다 (P3 §6.2). */
  async sendMessage(text: string): Promise<void> {
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
    const threadId = this.store.getOpenThreadId();
    const transcript = this.store.getTranscript();
    const turn = transcript?.turns.at(-1);
    if (!threadId || !turn || turn.status !== "inProgress") {
      return;
    }
    await this.client.request("turn/interrupt", { threadId, turnId: turn.id });
  }

  /** 작업 폴더에서 새 스레드를 시작하고 연다. 첫 메시지는 열린 스레드에 보낸다 (P3 §6.2). */
  async startThread(cwd: string): Promise<void> {
    const response = await this.client.request<ThreadStartResponse>("thread/start", { cwd });
    const transcript = transcriptFromThread(response.thread);
    this.batcher.clear();
    this.store.setOpenThread(response.thread.id);
    this.store.setTranscript(transcript);
    this.store.addThreadToFolder(response.thread.cwd, {
      id: response.thread.id,
      title: transcript.title,
      updatedAt: response.thread.updatedAt,
    });
  }

  /** 승인 요청에 답한다. 다른 창이 먼저 답했으면 이미 닫혀 있으므로 아무것도 하지 않는다. */
  answerApproval(requestId: JsonRpcId, decision: ApprovalDecision): void {
    const key = approvalKey(requestId);
    const answer = this.approvalAnswers.get(key);
    this.approvalAnswers.delete(key);
    this.store.removeApproval(requestId);
    answer?.({ decision });
  }

  private handleServerRequest(method: string, params: unknown, id: JsonRpcId): Promise<unknown> {
    const approval = toPendingApproval(method, params, id);
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
    if (previousId && previousId !== threadId && !isTurnInProgress(this.store.getTranscript())) {
      void this.client.request("thread/unsubscribe", { threadId: previousId }).catch(() => undefined);
    }
    this.batcher.clear();
    this.store.setOpenThread(threadId);
    await this.resumeOpenThread();
  }

  private async initialize(): Promise<void> {
    this.client.reset("reconnected");
    try {
      await this.client.request("initialize", initializeParams(this.clientVersion));
      this.client.notify("initialized");
      const gateway = await this.client.request<GatewayInfo>("akra/gateway/info");
      const threads = await this.listAllThreads();
      this.store.setSession({
        gateway,
        versionCheck: checkCodexVersion(GENERATED_CODEX_VERSION, gateway.codexVersion),
        folders: groupThreadsByCwd(threads),
      });
      // 이 창이 아직 구독하지 않은 스레드도 목록의 상태로 진행 중 표시를 채운다.
      this.store.setActiveThreads(threads.filter((thread) => thread.status.type === "active").map((thread) => thread.id));
      if (this.store.getOpenThreadId()) {
        await this.resumeOpenThread();
      }
    } catch (error) {
      this.store.setSessionError(errorMessage(error));
    }
  }

  /** `nextCursor` 를 따라 모든 페이지를 읽는다. 첫 페이지만 쓰면 오래된 작업 폴더가 목록에서 빠진다. */
  private async listAllThreads(): Promise<Thread[]> {
    const threads: Thread[] = [];
    let cursor: string | null = null;
    for (let page = 0; page < THREAD_LIST_MAX_PAGES; page += 1) {
      const response: ThreadListResponse = await this.client.request<ThreadListResponse>("thread/list", {
        limit: THREAD_LIST_LIMIT,
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
      const response = await this.client.request<ThreadResumeResponse>("thread/resume", { threadId, excludeTurns: true });
      if (this.store.getOpenThreadId() !== threadId) {
        return;
      }
      let transcript = transcriptFromResume(response);
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
      if (cursor === null && error instanceof JsonRpcError) {
        return { data: [], nextCursor: null, backwardsCursor: null };
      }
      throw error;
    }
  }

  private handleNotification(method: string, params: unknown): void {
    const threadId = notificationThreadId(params);
    if (!threadId) {
      return;
    }
    if (method === "thread/status/changed") {
      this.store.setThreadActive(threadId, (params as { status: ThreadStatus }).status.type === "active");
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
