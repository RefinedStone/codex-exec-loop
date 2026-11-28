import { IsolatedWorkspaceStarter, type PreparedWorkspace } from "./isolatedWorkspace";
import { samePath } from "../git/gitGraph";
// 접속이 맺어질 때마다 initialize 를 다시 하고 Gateway 정보와 작업 폴더 목록을 읽는다 (P2 §4.5).
// 스레드를 열면 `thread/resume` 으로 기록을 받고, 그 스레드의 알림을 화면 프레임마다 묶어 반영한다 (P3 §6.1, §7).
// 다시 연결되면 열려 있던 스레드를 다시 resume 한다 (P3 §6.5).
// 다른 Codex 프로세스가 그 스레드의 쓰기 잠금을 가지고 있으면 resume 이 거절된다. 그때는 thread/read 로 읽기만 한다.
// 입력(새 턴, steer, 중단, 새 스레드)과 승인 요청 응답도 여기서 보낸다 (P3 §6.2, §6.4). 턴을 만드는 요청은
// 사용자가 보낸 메시지에서만 나간다.
import type { AkraBridge, RemoteHostBridge } from "../../shared/bridge";
import { BROWSER_TOOL_NAMES, type BrowserAgentBridge } from "../../shared/browserAgent";
import type { ListMcpServerStatusResponse } from "../../shared/protocol/generated/v2/ListMcpServerStatusResponse";
import type { ListMcpServerStatusParams } from "../../shared/protocol/generated/v2/ListMcpServerStatusParams";
import type { FuzzyFileSearchResponse } from "../../shared/protocol/generated/FuzzyFileSearchResponse";
import type { InitializeParams } from "../../shared/protocol/generated/InitializeParams";
import type { SkillsListParams } from "../../shared/protocol/generated/v2/SkillsListParams";
import type { SkillsListResponse } from "../../shared/protocol/generated/v2/SkillsListResponse";
import type { Thread } from "../../shared/protocol/generated/v2/Thread";
import type { ThreadForkParams } from "../../shared/protocol/generated/v2/ThreadForkParams";
import type { ThreadForkResponse } from "../../shared/protocol/generated/v2/ThreadForkResponse";
import type { ThreadListResponse } from "../../shared/protocol/generated/v2/ThreadListResponse";
import type { ThreadReadResponse } from "../../shared/protocol/generated/v2/ThreadReadResponse";
import type { ThreadResumeResponse } from "../../shared/protocol/generated/v2/ThreadResumeResponse";
import type { ThreadStartResponse } from "../../shared/protocol/generated/v2/ThreadStartResponse";
import type { TurnStartResponse } from "../../shared/protocol/generated/v2/TurnStartResponse";
import type { ThreadStatus } from "../../shared/protocol/generated/v2/ThreadStatus";
import type { ThreadTurnsListResponse } from "../../shared/protocol/generated/v2/ThreadTurnsListResponse";
import type { ThreadUnarchiveResponse } from "../../shared/protocol/generated/v2/ThreadUnarchiveResponse";
import type { UserInput } from "../../shared/protocol/generated/v2/UserInput";
import { GENERATED_CODEX_VERSION } from "../../shared/protocol/version";
import { type ApprovalDecision, approvalKey, type PendingApproval, permissionsResponse, toPendingApproval } from "../conversation/approvals";
import { FrameBatcher, type FrameScheduler } from "../conversation/frameBatcher";
import {
  applyNotifications,
  isTurnInProgress,
  notificationThreadId,
  prependOlderTurns,
  transcriptFromThread,
  type Transcript,
} from "../conversation/transcript";
import { KernelConnection, type IssueDecisionAttention, type KernelInterest } from "../kernel/kernelConnection";
import { TaskAuthoringController } from "../kernel/taskAuthoring";
import type { IssueAutonomyLevel, IssueCommandOutcome, IssueConversationTarget, IssueInterventionName } from "../kernel/issueInteractions";
import { KERNEL_METHOD, type IssueAutomationModeRecord, type IssueEventReference } from "../kernel/protocol";
import { JsonRpcClient, JsonRpcError, type JsonRpcId, NO_RESPONSE } from "../rpc/jsonRpcClient";
import { checkCodexVersion } from "./codexVersion";
import { ChatHistory } from "./chatHistory";
import { ACCOUNT_LIMITS_READ, applyAccountLimitsNotification, applyAccountLimitsRead, setAccountLimitsStatus, useAccountLimits } from "./accountLimits";
import { applyMcpWriteApprovalNotification, clearMcpWriteApprovals, MCP_WRITE_APPROVAL_ANSWER, MCP_WRITE_APPROVAL_SUBSCRIBE, removeMcpWriteApproval } from "./mcpWriteApprovals";
import { NextTurnQueue } from "./nextTurnQueue";
import { NextTurnSettingsController } from "./nextTurnSettings";
import { ThreadCompactionController } from "./threadCompaction";
import { EMPTY_FORK_SNAPSHOT, ForkOutcomeJournal } from "./forkOutcomeJournal";
import { ProvisionalThreadTitles, threadDisplayTitle } from "./threadTitles";
import { type DotLiveState, type DotSummary, type GatewayInfo, type SessionStore, sortFolders, type WorkspaceFolder, type WorkspaceThread } from "./desktopStore";

const THREAD_LIST_LIMIT = 100;
/** 스레드를 열 때와 "이전 턴 더 보기" 한 번에 읽는 턴 수이다. 가장 큰 실측 스레드에서 10턴은 약 11 MB, 270ms 였다. */
const TURN_PAGE_SIZE = 10;
/** 첫 턴 전의 새 스레드에 대한 app-server 오류이다 (2026-11-05 실측: `no rollout found for thread id …`). */
const NO_ROLLOUT_ERROR = /no rollout found/i;
/**
 * 다른 Codex 프로세스(예: Codex 앱)가 스레드의 쓰기 잠금을 가지고 있을 때의 resume 오류이다 (2026-11-18 실측,
 * codex 0.167.2: `thread … already has an active writer`). 이때도 `thread/read` 와 `thread/turns/list` 는 된다.
 */
const WRITER_ELSEWHERE_ERROR = /already has an active writer/i;
const WRITER_ELSEWHERE_SEND_ERROR = "다른 Codex 앱이 이 스레드를 쓰고 있어서 보낼 수 없습니다. 다시 연결한 뒤에 보내세요.";
/** 대화 분기의 경계를 고를 때 읽는 가장 새 턴 수이다. 가장 새 턴이 진행 중이면 그 앞 턴이 경계이다. */
const FORK_BOUNDARY_PAGE_SIZE = 2;
const BROWSER_MCP_SERVER = "akra_browser";
const FORK_NO_FINISHED_TURN = "끝난 턴이 없는 대화는 분기할 수 없습니다.";
const FORK_UNKNOWN = "분기 결과를 확인하지 못했습니다. 사이드바에서 '분기' 표시가 있는 대화를 먼저 확인하세요. 다시 분기하면 대화가 하나 더 생길 수 있습니다.";
/** `thread/fork` 응답을 기다리는 시간이다 (taskAuthoring 의 저장 대기와 같은 값). 지나면 결과를 모른다. */
export const FORK_RESPONSE_TIMEOUT_MS = 20_000;
const FORK_TIMED_OUT = Symbol("fork timed out");
/** shortcut: 최대 50 페이지(5,000 스레드)까지만 읽는다. 더 많으면 사이드바를 필요할 때 더 읽는 방식으로 바꾼다. */
const THREAD_LIST_MAX_PAGES = 50;

interface ResumeLoading {
  threadId: string;
  buffer: QueuedNotification[];
}


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
    ...(thread.forkedFromId ? { forkedFromId: thread.forkedFromId } : {}),
    ...(isAgentThreadSource(thread.threadSource) ? { agentThread: true } : {}),
  };
}

/**
 * 대화 분기의 결과이다 (#2625). `refused` 는 서버에 대화를 만들지 않았다. `unknown` 은 접속이 끊기거나 응답이
 * `FORK_RESPONSE_TIMEOUT_MS` 안에 오지 않아서 서버가 만들었는지 모른다: 자동으로 다시 보내지 않는다. 늦게 온 성공은
 * 목록에만 넣는다. `created` 는 사용자가 다른 화면으로 가서 목록에만 넣었다.
 */
export type ForkThreadOutcome =
  | { kind: "opened"; threadId: string }
  | { kind: "created"; threadId: string }
  | { kind: "refused"; reason: string }
  | { kind: "unknown"; reason: string };

/**
 * 1차 범위(일반 로컬 사용자 대화)가 아닌 원본이면 이유를 돌려준다. 브라우저 대화는 읽은 턴에 브라우저 도구 호출이
 * 있는지로 판정한다. shortcut: 읽지 않은 이전 턴은 보지 않는다. 그래도 일반 fork 는 thread/start 의 MCP 설정을
 * 옮기지 않으므로(codex 0.172.0 `load_for_cwd`) 브라우저 세션 파일을 나눠 쓰지 않는다.
 */
export function forkBlockReason(thread: Pick<Thread, "id" | "ephemeral" | "parentThreadId" | "threadSource">, transcript: Transcript | null): string | null {
  if (thread.ephemeral || thread.parentThreadId || isAgentThreadSource(thread.threadSource)) {
    return "임시 대화, 하위 에이전트 대화, dot 작업 대화는 분기하지 않습니다.";
  }
  const browser = transcript?.threadId === thread.id
    && transcript.turns.some((turn) => turn.items.some((item) => item.type === "mcpToolCall" && item.server === BROWSER_MCP_SERVER));
  return browser ? "브라우저 대화는 아직 분기하지 않습니다. 브라우저 연결은 대화마다 따로 등록해야 합니다." : null;
}

// Generic server/internal errors do not prove that a non-idempotent request
// had no side effect. Only protocol-level pre-execution refusals are definitive.
function definitiveForkRejection(error: unknown): error is JsonRpcError {
  return error instanceof JsonRpcError && [-32700, -32600, -32601, -32602].includes(error.code);
}

/** `thread/fork` 응답의 새 대화이다. id 가 없거나 원본과 같으면 null 이다. */
function forkedThreadOf(response: ThreadForkResponse | undefined, sourceThreadId: string, sourceCwd: string): Thread | null {
  const forked = response?.thread;
  return typeof forked?.id === "string" && forked.id.length > 0 && forked.id !== sourceThreadId
    && forked.forkedFromId === sourceThreadId && forked.cwd === sourceCwd ? forked : null;
}

export function groupThreadsByCwd(threads: readonly Thread[]): WorkspaceFolder[] {
  const folders = new Map<string, WorkspaceFolder>();
  // rollout 전체 검사는 페이지 사이에 같은 스레드를 다시 줄 수 있다 (2026-11-06 실측: 306행 중 고유 292개).
  const seen = new Set<string>();
  for (const thread of threads) {
    if (seen.has(thread.id)) continue;
    seen.add(thread.id);
    const folder = folders.get(thread.cwd) ?? { cwd: thread.cwd, threads: [] };
    folder.threads.push(workspaceThread(thread, threadDisplayTitle(thread.name, thread.preview)));
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
const GATEWAY_SERVICE_PREFIXES = ["akra/mcp/", "akra/context/", "akra/dots/", "akra/git/", "akra/issue/", "akra/providerCredentials/", "akra/providers/"];
/** 원격 호스트의 dot 목록이다 (D-19). dots 가 꺼진 Gateway 는 모르는 method 로 답한다. */
const DOTS_LIST_METHOD = "akra/dots/list";
/** 방의 실시간 상태를 보기 시작한다. 그 뒤 `akra/dots/live` 알림이 온다. */
const DOTS_WATCH_METHOD = "akra/dots/watch";
const DOTS_LIVE_NOTIFICATION = "akra/dots/live";
/** 세션이 쓰는 메시지 버스이다. `local` 은 `window.akra` 이고, 원격 호스트는 `remoteHostBridge` 이다. */
export type SessionBridge = Pick<AkraBridge, "send" | "onMessage" | "onConnectionState" | "ready"> & Partial<Pick<AkraBridge, "confirmIssueAutomationMode" | "confirmIssueDecision" | "confirmIssueAutonomy" | "confirmIssueBindingResume" | "confirmTaskCriterionWaiver">> & {
  readonly chatHistory?: true;
};

/** 원격 호스트 하나의 메시지 버스이다 (plan 25). `ready` 가 그 호스트의 접속을 연다. */
export function remoteHostBridge(remote: RemoteHostBridge, host: string, role: "main" | "dot-window"): SessionBridge {
  return {
    ...(role === "main" ? { chatHistory: true } : {}),
    send: (text) => remote.send(host, text),
    onMessage: (listener) => remote.onMessage((from, text) => from === host && listener(text)),
    onConnectionState: (listener) => remote.onConnectionState((from, state) => from === host && listener(state)),
    ready: () => remote.open(host),
  };
}

const READ_ONLY_ERROR = "원격 호스트의 대화는 읽기 전용입니다.";

/** 열린 스레드의 기록과, 다른 프로세스가 쓰기 잠금을 가져서 구독하지 못하고 읽기만 했는지이다. */
interface OpenedThread {
  thread: Thread;
  writerElsewhere: boolean;
  model?: string | null;
  effort?: string | null;
  approvalPolicy?: unknown;
  sandbox?: unknown;
}

export class DesktopSession {
  readonly nextTurnSettings: NextTurnSettingsController;
  readonly nextTurnQueue: NextTurnQueue;
  readonly compaction: ThreadCompactionController;
  private readonly pendingTurnStarts = new Set<string>();
  private readonly acknowledgedTurnStarts = new Map<string, string>();
  private readonly versionProviderListeners = new Set<() => void>();

  onVersionProviderReady(listener: () => void): () => void {
    this.versionProviderListeners.add(listener);
    return () => this.versionProviderListeners.delete(listener);
  }
  private readonly client: JsonRpcClient;
  private readonly isolatedWorkspace: IsolatedWorkspaceStarter;
  private readonly batcher: FrameBatcher<QueuedNotification>;
  private readonly unsubscribers: Array<() => void> = [];
  private readonly skillsChangedListeners = new Set<() => void>();
  /** resume 응답을 기다리는 동안 그 스레드에 온 알림이다. 응답을 반영한 뒤 이어서 반영한다. */
  private loading: ResumeLoading | null = null;
  /** 탐색/resume 세대는 스트리밍으로 바뀌는 transcript 객체와 분리한다. */
  private conversationGeneration = 0;
  /** initialize 가 thread/list 를 읽는 동안 받은 진행 상태 변경이다 (스레드 id → 진행 중). */
  private statusChangesWhileListing: Map<string, boolean> | null = null;
  private connected = false;
  private forkConnectionEpoch = 0;
  private providerVersionScope = "";
  /**
   * 원격 세션에서 사용자가 보내기를 허락했는지이다 (plan 25 G2, "세션 한 번 허용", D-19). 허락 전에는 읽기만
   * 한다. 허락하면 열린 스레드를 thread/resume 으로 다시 구독해 실시간으로 받고, 턴을 보낼 수 있다.
   */
  private sendingAllowed = false;
  /** 다른 Codex 프로세스가 쓰기 잠금을 가져서 구독하지 못하고 읽기만 한 스레드이다. 이 스레드에는 보내지 않는다. */
  private writerElsewhereThreadId: string | null = null;
  /** 접속마다 늘어난다. 이전 접속에서 늦게 끝난 전체 검사는 반영하지 않는다. */
  private listGeneration = 0;
  private readonly chatHistory: ChatHistory | undefined;
  /**
   * 이 접속에서 받은 보관·삭제와 제목 변경이다. 목록 요청보다 늦게 온 알림이 더 새롭다. 그래서 늦게 끝난 목록
   * 결과(첫 목록, rollout 전체 검사)에 덮어 쓴다. 접속마다 비운다.
   */
  private removedThreadIds = new Set<string>();
  private renamedThreads = new Map<string, string>();
  // List/resume responses cannot overwrite names learned or edited after the request started.
  private titleRevision = 0;
  private readonly titleRevisions = new Map<string, number>();
  private readonly pendingThreadRenames = new Map<string, number>();
  private readonly provisionalTitles = new ProvisionalThreadTitles();
  /** 이 접속에서 생기거나 보관을 푼 스레드이다. 그 전에 읽기 시작한 목록에는 없을 수 있다. */
  private addedThreads = new Map<string, Thread>();
  /** 원본 id → 진행 중인 분기이다. 같은 원본의 두 번째 요청은 같은 결과를 기다린다. */
  private readonly forksInFlight = new Map<string, Promise<ForkThreadOutcome>>();
  /** 요청 전 기록하여 renderer 재시작 뒤에도 결과 불명 경고를 유지한다. */
  private readonly forkOutcomes: ForkOutcomeJournal;
  /** 사용자의 답을 기다리는 승인 요청의 응답 함수이다. 키는 요청 id 이다. */
  private readonly approvalAnswers = new Map<string, { approval: PendingApproval; answer: (answer: unknown) => void }>();
  /** 커널 패널의 연결이다 (plan 18 K3-6). 같은 Gateway 접속을 쓴다. */
  private readonly kernel: KernelConnection;
  readonly taskAuthoring: TaskAuthoringController;
  private taskAuthoringEpoch = 0;
  private taskAuthoringWorkspace: string | null = null;

  constructor(
    private readonly bridge: SessionBridge,
    private readonly store: SessionStore,
    private readonly clientVersion: string,
    scheduler?: FrameScheduler,
    /** 원격 호스트의 세션이다 (plan 25). 기본은 읽기만 한다: 턴, 새 스레드, 승인 응답을 보내지 않는다. */
    private readonly readOnly = false,
    forkOutcomes = new ForkOutcomeJournal(),
  ) {
    this.forkOutcomes = forkOutcomes;
    this.client = new JsonRpcClient((text) => bridge.send(text));
    this.isolatedWorkspace = new IsolatedWorkspaceStarter((method, params) => this.client.request(method, params));
    this.nextTurnSettings = new NextTurnSettingsController((params) => this.client.request("model/list", params));
    this.nextTurnQueue = new NextTurnQueue(async (threadId, text) => {
      this.assertWritable();
      this.assertSubscribed(threadId);
      this.assertTurnSubmissionAvailable(threadId, true);
      if (this.store.getOpenThreadId() !== threadId || this.loading) throw new Error("예약한 대화가 현재 열려 있지 않습니다.");
      const response = await this.startTextTurn(threadId, text);
      return response?.turn?.id ?? "";
    }, (threadId) => this.connected && this.canSend() && !this.loading && !this.pendingTurnStarts.has(threadId) && !this.compaction.isBlocking(threadId)
      && this.store.getOpenThreadId() === threadId && this.writerElsewhereThreadId !== threadId,
      (threadId) => this.nextTurnSettings.validationReason(threadId));
    this.compaction = new ThreadCompactionController({
      request: (method, params) => this.client.request(method, params),
      reload: async (threadId, turnId, status) => {
        if (!this.connected || this.store.getOpenThreadId() !== threadId) return false;
        const generation = this.conversationGeneration + 1;
        await this.reopenThread();
        if (!this.connected || this.conversationGeneration !== generation || this.loading || this.store.getOpenThreadId() !== threadId) return false;
        const transcript = this.store.getTranscript();
        const turn = transcript?.threadId === threadId ? transcript.turns.find(turn => turn.id === turnId) : undefined;
        return turn?.status === status && (status !== "completed" || turn.items.some(item => item.type === "contextCompaction"));
      },
      context: () => {
        const threadId = this.store.getOpenThreadId();
        const transcript = this.store.getTranscript();
        const queued = threadId ? this.nextTurnQueue.snapshot(threadId) : null;
        return {
          threadId, connected: this.connected, writable: this.canSend() && this.writerElsewhereThreadId !== threadId,
          loaded: !this.loading && !!threadId && transcript?.threadId === threadId,
          // status/active 알림은 controller가 즉시 추적하고 terminal로 해제한다.
          turnInProgress: isTurnInProgress(transcript),
          submissionInFlight: !!threadId && (this.isTurnSubmissionPending(threadId) || this.acknowledgedTurnStarts.has(threadId) || !!queued?.items.some(item => item.state === "unknown")),
          knownTurnIds: transcript?.turns.map(turn => turn.id) ?? [],
        };
      },
    });
    this.taskAuthoring = new TaskAuthoringController(this.client, () => {
      const kernel = this.store.getKernel();
      return { workspace: kernel.workspace, kernel: kernel.kernel,
        attached: !this.readOnly && this.connected && kernel.phase === "attached",
        epoch: this.taskAuthoringEpoch };
    });
    this.chatHistory = readOnly && bridge.chatHistory ? new ChatHistory(this.client, store) : undefined;
    this.batcher = new FrameBatcher((batch) => this.store.updateTranscript((transcript) => applyNotifications(transcript, batch)), scheduler);
    this.client.setServerRequestHandler((method, params, id) => this.handleServerRequest(method, params, id));
    this.kernel = new KernelConnection(this.client, store, readOnly ? undefined : bridge.confirmIssueAutomationMode, readOnly ? {} : {
      confirmIssueDecision: bridge.confirmIssueDecision,
      confirmIssueAutonomy: bridge.confirmIssueAutonomy,
      confirmIssueBindingResume: bridge.confirmIssueBindingResume,
      confirmTaskCriterionWaiver: bridge.confirmTaskCriterionWaiver,
    });
  }

  start(): void {
    this.unsubscribers.push(this.bridge.onMessage((text) => this.client.handleText(text)));
    this.unsubscribers.push(this.client.onNotification((method, params) => this.handleNotification(method, params)));
    this.unsubscribers.push(
      this.bridge.onConnectionState((state) => {
        this.providerVersionScope = "";
        this.store.setConnection(state);
        this.connected = state.phase === "connected";
        this.forkConnectionEpoch++;
        if (!this.connected) this.provisionalTitles.clear();
        this.compaction.connectionChanged(this.connected);
        if (this.connected) {
          void this.initialize();
        } else {
          this.nextTurnSettings.connectionChanged(false);
          this.chatHistory?.disconnected();
          this.invalidateConversation();
          this.taskAuthoringEpoch++;
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
    this.forkConnectionEpoch++;
    this.compaction.dispose();
    this.nextTurnSettings.dispose();
    this.nextTurnQueue.dispose();
    this.providerVersionScope = "";
    for (const unsubscribe of this.unsubscribers.splice(0)) {
      unsubscribe();
    }
    this.invalidateConversation();
    this.batcher.clear();
    this.chatHistory?.invalidate(null);
    this.kernel.connectionChanged(false);
    this.taskAuthoringEpoch++;
    this.client.reset("closed");
    this.dropPendingApprovals();
  }

  /** 커널 패널이 보여 줄 폴더를 정한다. 그 폴더의 저장소 커널에 붙고, null 이면 뗀다. */
  showKernel(workspace: string | null, interests: readonly KernelInterest[] = ["overview"]): Promise<void> {
    if (this.taskAuthoringWorkspace !== workspace) {
      this.taskAuthoringWorkspace = workspace;
      this.taskAuthoringEpoch++;
    }
    return this.kernel.show(workspace, interests);
  }

  requestDelegation<T>(method: typeof KERNEL_METHOD.query | typeof KERNEL_METHOD.command, params: unknown): Promise<T> {
    if (!this.connected) return Promise.reject(new Error("Gateway 에 연결되어 있지 않습니다."));
    return this.client.request<T>(method, params);
  }

  /** 커널 패널의 상태를 다시 읽는다. */
  refreshKernel(): Promise<void> {
    return this.kernel.refresh();
  }

  waiveTaskCriterion(target: import("../kernel/taskCriterionWaiver").TaskCriterionWaiverTarget, reason: string): Promise<import("../kernel/taskCriterionWaiver").TaskCriterionWaiverOutcome> {
    return this.kernel.waiveTaskCriterion(target, reason);
  }

  requestTaskVerification(target: import("../kernel/taskVerificationActions").TaskVerificationTarget): Promise<void> {
    return this.kernel.requestTaskVerification(target);
  }

  setParallelEnabled(enabled: boolean): Promise<void> {
    return this.kernel.setParallelEnabled(enabled);
  }

  refreshIssues(): Promise<void> {
    return this.kernel.refreshIssues();
  }

  nextIssuesPage(): Promise<void> {
    return this.kernel.nextIssuesPage();
  }

  previousIssuesPage(): Promise<void> {
    return this.kernel.previousIssuesPage();
  }

  selectIssue(issue: IssueEventReference): Promise<void> {
    return this.kernel.selectIssue(issue);
  }

  selectIssueFromAttention(attention: IssueDecisionAttention): Promise<void> {
    return this.kernel.selectIssueFromAttention(attention);
  }

  clearIssueSelection(): void {
    this.kernel.clearIssueSelection();
  }

  refreshIssueDetail(): Promise<void> {
    return this.kernel.refreshIssueDetail();
  }

  refreshIssueAutomationPolicy(): Promise<void> {
    return this.kernel.refreshIssueAutomationPolicy();
  }

  setIssueAutomationMode(mode: IssueAutomationModeRecord): Promise<void> {
    return this.kernel.setIssueAutomationMode(mode);
  }

  syncIssues(): Promise<void> {
    return this.kernel.syncIssues();
  }

  setIssueBindingPaused(paused: boolean): Promise<void> {
    return this.kernel.setIssueBindingPaused(paused);
  }

  setIssueAutonomy(level: IssueAutonomyLevel): Promise<void> {
    return this.kernel.setIssueAutonomy(level);
  }

  answerIssueDecision(decisionId: string, option: string, note: string, criteria?: import("../kernel/protocol").TaskAcceptanceCriterion[]): Promise<void> {
    return this.kernel.answerIssueDecision(decisionId, option, note, criteria);
  }

  interveneIssue(action: IssueInterventionName, message?: string, confirm = false): Promise<IssueCommandOutcome> {
    return this.kernel.interveneIssue(action, message, confirm);
  }

  readIssueConversation(): Promise<IssueConversationTarget | null> {
    return this.kernel.readIssueConversation();
  }

  /** Codex 가 찾은 스킬을 읽는다 (#2383). `forceReload` 이면 캐시 대신 디스크를 다시 읽는다. 턴을 쓰지 않는다. */
  listSkills(cwd: string | null, forceReload: boolean): Promise<SkillsListResponse> {
    if (!this.connected) {
      return Promise.reject(new Error("Gateway 에 연결되어 있지 않습니다."));
    }
    const params: SkillsListParams = { cwds: cwd ? [cwd] : [], forceReload };
    return this.client.request<SkillsListResponse>("skills/list", params);
  }

  /** `skills/changed` 알림을 듣는다. 로컬 스킬 파일이 바뀌었다는 신호이다: 목록을 다시 읽는다. */
  onSkillsChanged(listener: () => void): () => void {
    this.skillsChangedListeners.add(listener);
    return () => this.skillsChangedListeners.delete(listener);
  }

  /** 업무 패널이 같은 Gateway 접속으로 보내는 요청이다: MCP hub(plan 21)와 작업 컨텍스트(plan 22). */
  onExtensionChanged(event: string, listener: (kernel: string, change?: import("../extensions/extensionState").ExtensionChange) => void): () => void {
    return this.client.onNotification((method, params) => {
      if (method === event && params && typeof params === "object" && "kernel" in params && typeof params.kernel === "string") listener(params.kernel, params as import("../extensions/extensionState").ExtensionChange);
    });
  }

  requestGatewayService<T>(method: string, params: unknown): Promise<T> {
    if (this.readOnly && (method.startsWith("akra/providerCredentials/") || method.startsWith("akra/providers/"))) {
      return Promise.reject(new Error("원격 호스트에서는 로컬 업무 계정에 접근할 수 없습니다."));
    }
    if (!["akra/kernel/extensions/list", "akra/kernel/extensions/call", "akra/kernel/extensions/attach", "akra/kernel/extensions/detach"].includes(method) && !GATEWAY_SERVICE_PREFIXES.some((prefix) => method.startsWith(prefix))) {
      return Promise.reject(new Error(`${method} 는 패널이 쓸 수 있는 Gateway 요청이 아닙니다.`));
    }
    if (!this.connected) {
      return Promise.reject(new Error("Gateway 에 연결되어 있지 않습니다."));
    }
    return this.client.request<T>(method, params);
  }

  /** 메시지를 보낸다. 진행 중인 턴이 있으면 steer, 없으면 새 턴이다 (P3 §6.2). */
  async sendMessage(text: string, images: readonly string[] = []): Promise<void> {
    this.assertWritable();
    const threadId = this.store.getOpenThreadId();
    this.assertSubscribed(threadId);
    const trimmed = text.trim();
    if (!threadId || (trimmed.length === 0 && images.length === 0)) {
      return;
    }
    this.assertTurnSubmissionAvailable(threadId);
    const input = userInput(trimmed, images);
    const transcript = this.store.getTranscript();
    const activeTurn = isTurnInProgress(transcript) ? transcript?.turns.at(-1) : undefined;
    const expectedTurnId = activeTurn?.id ?? this.acknowledgedTurnStarts.get(threadId);
    if (expectedTurnId) {
      await this.client.request("turn/steer", { threadId, input, expectedTurnId });
      this.displayAcceptedTitle(threadId, trimmed);
    } else {
      await this.startTextTurn(threadId, trimmed, images);
    }
  }

  titleCandidate(threadId: string): string | undefined { return this.provisionalTitles.candidate(threadId); }

  private displayAcceptedTitle(threadId: string, text: string): void {
    const title = this.provisionalTitles.accepted(threadId, text);
    if (title === null) return;
    this.store.renameThread(threadId, title);
    this.store.updateTranscript(current => current.threadId === threadId ? { ...current, title } : current);
  }

  queueMessage(text: string): void {
    this.assertWritable();
    const threadId = this.store.getOpenThreadId();
    this.assertSubscribed(threadId);
    if (!threadId || !this.connected) throw new Error("연결된 대화에서 입력을 예약하세요.");
    if (this.compaction.isBlocking(threadId)) throw new Error("문맥 압축의 결과를 확인한 뒤 입력을 예약하세요.");
    if (this.pendingTurnStarts.has(threadId) && !this.nextTurnQueue.hasInFlight(threadId)) throw new Error("새 턴의 접수 확인 후 예약하세요.");
    const transcript = this.store.getTranscript();
    const turnId = isTurnInProgress(transcript) ? transcript?.turns.at(-1)?.id ?? null : null;
    this.nextTurnQueue.enqueue(threadId, text, turnId);
  }

  resumeQueuedMessages(): void {
    this.assertWritable();
    const threadId = this.store.getOpenThreadId();
    this.assertSubscribed(threadId);
    if (!threadId) return;
    const transcript = this.store.getTranscript();
    this.nextTurnQueue.resume(threadId, isTurnInProgress(transcript)
      ? transcript?.turns.at(-1)?.id ?? null : this.acknowledgedTurnStarts.get(threadId) ?? null);
  }

  private assertTurnSubmissionAvailable(threadId: string, fromQueue = false): void {
    if (this.compaction.isBlocking(threadId)) throw new Error("문맥 압축의 결과를 확인한 뒤 입력을 보내세요.");
    if (this.pendingTurnStarts.has(threadId) || (!fromQueue && this.nextTurnQueue.hasInFlight(threadId))) {
      throw new Error("새 턴의 접수를 확인하는 중입니다. 확인 후 다시 보내세요.");
    }
    if (this.nextTurnQueue.snapshot(threadId).items.some((item) => item.state === "unknown")) {
      throw new Error("결과 불명인 예약 입력의 대화 기록을 확인하고 목록에서 지운 뒤 보내세요.");
    }
  }

  isTurnSubmissionPending(threadId: string): boolean {
    return this.pendingTurnStarts.has(threadId) || this.nextTurnQueue.hasInFlight(threadId);
  }

  private async startTextTurn(threadId: string, text: string, images: readonly string[] = []): Promise<TurnStartResponse> {
    const generation = this.conversationGeneration;
    const settings = this.nextTurnSettings.prepare(threadId);
    this.pendingTurnStarts.add(threadId);
    try {
      const input = userInput(text, images);
      const response = await this.client.request<TurnStartResponse>("turn/start", { threadId, input, ...settings.params });
      if (typeof response?.turn?.id === "string" && response.turn.id.length > 0) {
        this.nextTurnSettings.accepted(settings);
        this.displayAcceptedTitle(threadId, text);
        this.nextTurnQueue.started(threadId, response.turn.id);
        if (generation === this.conversationGeneration && this.nextTurnQueue.snapshot(threadId).waitingTurnId === response.turn.id) {
          this.acknowledgedTurnStarts.set(threadId, response.turn.id);
        }
      }
      return response;
    } finally {
      this.pendingTurnStarts.delete(threadId);
    }
  }

  /** 진행 중인 턴을 중단한다. */
  async interrupt(): Promise<void> {
    this.assertWritable();
    const threadId = this.store.getOpenThreadId();
    this.assertSubscribed(threadId);
    if (threadId) this.nextTurnQueue.pause(threadId, "사용자가 중단을 요청해 예약을 멈췄습니다. 확인 후 계속하세요.");
    const transcript = this.store.getTranscript();
    const turn = transcript?.turns.at(-1);
    if (!threadId || !turn || turn.status !== "inProgress") {
      return;
    }
    await this.client.request("turn/interrupt", { threadId, turnId: turn.id });
  }

  /** 작업 폴더에서 새 스레드를 시작하고 연다. 첫 메시지는 열린 스레드에 보낸다 (P3 §6.2). */
  async startIsolatedThread(cwd: string): Promise<string> {
    if (this.readOnly) throw new Error("격리 공간은 로컬 호스트에서만 만들 수 있습니다.");
    this.assertWritable();
    const generation = this.conversationGeneration;
    return this.isolatedWorkspace.start(cwd, async (workspace, markStartRequested) => {
      if (generation !== this.conversationGeneration) throw new Error("대화 위치가 바뀌었습니다. 준비된 공간은 보존됩니다.");
      return this.startThread(workspace.workspaceDir, workspace, generation, markStartRequested);
    });
  }

  async startThread(cwd: string, expected?: PreparedWorkspace, generation?: number, markStartRequested?: () => void): Promise<string> {
    this.assertWritable();
    markStartRequested?.();
    const response = await this.client.request<ThreadStartResponse>("thread/start", { cwd });
    if (expected && (!response.thread.id || !samePath(response.thread.cwd, expected.workspaceDir)
      || (response.thread.gitInfo?.branch != null && response.thread.gitInfo.branch !== expected.branch)
      || generation !== this.conversationGeneration)) {
      throw new Error("격리 공간과 스레드의 연결을 확인하지 못했습니다. 준비된 공간은 보존됩니다.");
    }
    this.nextTurnSettings.observeSnapshot(response.thread.id, response.model ?? response.thread.model, response.reasoningEffort ?? response.thread.reasoningEffort,
      undefined, { approvalPolicy: response.approvalPolicy, sandbox: response.sandbox });
    const transcript = transcriptFromThread(response.thread);
    this.invalidateConversation();
    this.batcher.clear();
    this.store.setOpenThread(response.thread.id);
    this.store.setTranscript(transcript);
    this.store.addThreadToFolder(response.thread.cwd, workspaceThread(response.thread, transcript.title));
    if (!this.readOnly && !response.thread.ephemeral && !response.thread.parentThreadId && !isAgentThreadSource(response.thread.threadSource)
        && !this.renamedThreads.has(response.thread.id)) {
      this.provisionalTitles.register(response.thread.id, response.thread.name, response.thread.preview);
    }
    return response.thread.id;
  }

  async startBrowserThread(cwd: string, agent: BrowserAgentBridge): Promise<string> {
    if (this.readOnly) throw new Error("브라우저 대화는 로컬 호스트에서만 만들 수 있습니다.");
    const registration = await agent.prepare();
    try {
      const response = await this.client.request<ThreadStartResponse>("thread/start", {
        cwd,
        config: {
          "mcp_servers.akra_browser.command": registration.command,
          "mcp_servers.akra_browser.args": [...registration.args],
          "mcp_servers.akra_browser.env.ELECTRON_RUN_AS_NODE": "1",
          "mcp_servers.akra_browser.env.AKRA_BROWSER_SESSION_FILE": registration.sessionFile,
        },
      });
      const threadId = response.thread.id;
      this.nextTurnSettings.observeSnapshot(threadId, response.model ?? response.thread.model, response.reasoningEffort ?? response.thread.reasoningEffort,
        undefined, { approvalPolicy: response.approvalPolicy, sandbox: response.sandbox });
      await agent.setActiveThread(threadId);
      await agent.bind(registration.registrationId, threadId);
      let cursor: string | null = null;
      let verified = false;
      do {
        const status: ListMcpServerStatusResponse = await this.client.request("mcpServerStatus/list", {
          threadId, detail: "toolsAndAuthOnly", ...(cursor ? { cursor } : {}),
        } satisfies ListMcpServerStatusParams);
        const browser = status.data.find((server) => server.name === BROWSER_MCP_SERVER);
        if (browser) {
          verified = browser.runtimeStatus === "connected" && !browser.toolsError
            && BROWSER_TOOL_NAMES.every((name) => browser.tools[name] !== undefined);
          break;
        }
        cursor = status.nextCursor;
      } while (cursor);
      if (!verified) throw new Error("브라우저 도구를 확인할 수 없습니다. 새 브라우저 대화를 만드세요.");
      const transcript = transcriptFromThread(response.thread);
      this.invalidateConversation();
    this.batcher.clear();
      this.store.setOpenThread(threadId);
      this.store.setTranscript(transcript);
      this.store.addThreadToFolder(response.thread.cwd, workspaceThread(response.thread, transcript.title));
      return threadId;
    } catch (error) {
      await agent.release(registration.registrationId);
      await agent.setActiveThread(this.store.getOpenThreadId());
      throw error;
    }
  }

  /** 승인 시점의 불명 시도 집합을 전송 직전에 다시 확인한다. */
  forkOutcomeSnapshot(sourceThreadId: string): string | null {
    try { return this.forkOutcomes.snapshot(sourceThreadId); }
    catch { return null; }
  }

  /** 이 원본의 지난 분기 결과를 모른다. 다시 분기하면 대화가 하나 더 생길 수 있다. */
  forkOutcomeUnknown(sourceThreadId: string): boolean {
    return this.forkOutcomes.has(sourceThreadId);
  }

  /**
   * 로컬 스레드를 공식 `thread/fork` 로 한 번 분기한다 (#2625). 원본에는 resume, 턴, unsubscribe 를 보내지 않는다.
   * 경계는 지금 읽은 가장 새 끝난 턴(completed, interrupted, failed)이고 `lastTurnId` 로 고정한다. 그 뒤에 다른 앱이
   * 시작한 턴은 새 대화에 들어가지 않는다. 응답이 왔을 때 원본이 아직 열려 있으면 `claimSelection` 을 부른다. 그것이
   * 화면을 새 대화로 옮기고 true 를 돌려주면 새 대화를 연다. 아니면 목록에만 넣고 fork 가 건 구독을 끝낸다.
   */
  forkThread(sourceThreadId: string, claimSelection: (threadId: string) => boolean, approvedSnapshot = EMPTY_FORK_SNAPSHOT): Promise<ForkThreadOutcome> {
    const inFlight = this.forksInFlight.get(sourceThreadId);
    if (inFlight) return inFlight;
    const fork = this.runFork(sourceThreadId, claimSelection, approvedSnapshot).finally(() => this.forksInFlight.delete(sourceThreadId));
    this.forksInFlight.set(sourceThreadId, fork);
    return fork;
  }

  private async runFork(sourceThreadId: string, claimSelection: (threadId: string) => boolean, approvedSnapshot: string): Promise<ForkThreadOutcome> {
    if (this.readOnly) return { kind: "refused", reason: "원격 호스트의 대화는 분기하지 않습니다." };
    if (!this.connected) return { kind: "refused", reason: "Gateway 에 연결되어 있지 않습니다." };
    const connectionEpoch = this.forkConnectionEpoch;
    const sameConnection = () => this.connected && connectionEpoch === this.forkConnectionEpoch;
    const connectionChanged = { kind: "refused", reason: "분기 요청 전 Gateway 연결이 바뀌었습니다." } as const;
    let lastTurnId: string;
    let sourceCwd: string;
    try {
      const source = await this.readThread(sourceThreadId);
      if (!sameConnection()) return connectionChanged;
      if (source.id !== sourceThreadId || typeof source.cwd !== "string" || !source.cwd) {
        return { kind: "refused", reason: "분기 원본의 ID와 작업 폴더를 확인할 수 없습니다." };
      }
      sourceCwd = source.cwd;
      const blocked = forkBlockReason(source, this.store.getTranscript());
      if (blocked) return { kind: "refused", reason: blocked };
      const page = await this.client.request<ThreadTurnsListResponse>("thread/turns/list", {
        threadId: sourceThreadId, limit: FORK_BOUNDARY_PAGE_SIZE, sortDirection: "desc", itemsView: "notLoaded",
      });
      if (!sameConnection()) return connectionChanged;
      const boundary = page.data.find((turn) => turn.status !== "inProgress");
      if (!boundary) return { kind: "refused", reason: FORK_NO_FINISHED_TURN };
      lastTurnId = boundary.id;
    } catch (error) {
      if (error instanceof JsonRpcError && NO_ROLLOUT_ERROR.test(error.message)) return { kind: "refused", reason: FORK_NO_FINISHED_TURN };
      return { kind: "refused", reason: `분기하지 않았습니다: ${errorMessage(error)}` };
    }
    // 여기부터 서버가 대화를 만들 수 있다. 결과를 모르면 다시 보내지 않는다.
    let attemptId: string;
    try {
      const admitted = await this.forkOutcomes.begin(sourceThreadId, approvedSnapshot);
      if (admitted === null) return { kind: "refused", reason: "다른 분기 결과가 바뀌었습니다. 사이드바를 확인하고 다시 분기 여부를 확인하세요." };
      attemptId = admitted;
    } catch {
      return { kind: "refused", reason: "분기 결과를 안전하게 기록할 수 없어 요청을 보내지 않았습니다." };
    }
    if (!sameConnection() || this.removedThreadIds.has(sourceThreadId)) {
      this.forkOutcomes.resolve(sourceThreadId, attemptId);
      return !sameConnection() ? connectionChanged : { kind: "refused", reason: "분기 원본이 보관되거나 삭제되었습니다." };
    }
    const params: ThreadForkParams = { threadId: sourceThreadId, lastTurnId, excludeTurns: true, ephemeral: false };
    const request = this.client.request<ThreadForkResponse>("thread/fork", params);
    let response: ThreadForkResponse | typeof FORK_TIMED_OUT;
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      response = await Promise.race([request, new Promise<typeof FORK_TIMED_OUT>((resolve) => { timer = setTimeout(() => resolve(FORK_TIMED_OUT), FORK_RESPONSE_TIMEOUT_MS); })]);
    } catch (error) {
      if (definitiveForkRejection(error)) {
        this.forkOutcomes.resolve(sourceThreadId, attemptId);
        return { kind: "refused", reason: error.code === -32601 ? "이 Codex app-server 는 대화 분기(thread/fork)를 지원하지 않습니다." : `분기하지 못했습니다: ${error.message}` };
      }
      return this.forkOutcomeLost();
    } finally {
      clearTimeout(timer);
    }
    if (response === FORK_TIMED_OUT) {
      // 늦게 온 성공은 목록에만 넣는다. 그동안 사용자가 본 화면을 덮지 않는다.
      void request.then((late) => {
        if (!sameConnection()) return;
        const forked = forkedThreadOf(late, sourceThreadId, sourceCwd);
        if (forked) {
          this.forkOutcomes.resolve(sourceThreadId, attemptId);
          if (this.listFork(forked)) this.releaseUnopenedFork(forked.id);
        }
      }, error => {
        if (definitiveForkRejection(error)) this.forkOutcomes.resolve(sourceThreadId, attemptId);
      });
      return this.forkOutcomeLost();
    }
    if (!sameConnection()) return this.forkOutcomeLost();
    const forked = forkedThreadOf(response, sourceThreadId, sourceCwd);
    if (!forked) return this.forkOutcomeLost();
    this.forkOutcomes.resolve(sourceThreadId, attemptId);
    if (!this.listFork(forked)) return { kind: "created", threadId: forked.id };
    if (this.store.getOpenThreadId() === sourceThreadId && claimSelection(forked.id)) {
      this.leaveOpenThread(forked.id);
      this.batcher.clear();
      this.store.setOpenThread(forked.id);
      // fork 가 이미 구독했다. 다시 resume 하지 않고 기록만 읽는다.
      await this.resumeOpenThread({ thread: forked, writerElsewhere: false });
      return { kind: "opened", threadId: forked.id };
    }
    this.releaseUnopenedFork(forked.id);
    return { kind: "created", threadId: forked.id };
  }

  /** 서버가 대화를 만들었는지 모른다. 이 원본을 다시 분기하기 전에 사용자에게 묻는다. */
  private forkOutcomeLost(): ForkThreadOutcome {
    return { kind: "unknown", reason: FORK_UNKNOWN };
  }

  /** `thread/started` 알림이 먼저 왔어도 목록에는 한 번만 들어간다. */
  private listFork(forked: Thread): boolean {
    if (this.removedThreadIds.has(forked.id)) return false;
    this.addedThreads.set(forked.id, forked);
    this.store.mergeFolders(groupThreadsByCwd([forked]));
    return true;
  }

  /** 사용자가 열지 않은 포크는 fork 가 건 구독을 끝낸다. */
  private releaseUnopenedFork(forkThreadId: string): void {
    if (this.store.getOpenThreadId() !== forkThreadId) {
      void this.client.request("thread/unsubscribe", { threadId: forkThreadId }).catch(() => undefined);
    }
  }

  /** 스레드 이름을 바꾼다 (#2841). app-server 가 모든 창에 `thread/name/updated` 를 보낸다. */
  async renameThread(threadId: string, name: string): Promise<void> {
    this.assertWritable();
    const trimmed = name.trim();
    if (!trimmed) throw new Error("이름이 비어 있습니다.");
    this.provisionalTitles.stopGenerating(threadId);
    const revision = this.advanceTitleRevision(threadId);
    this.pendingThreadRenames.set(threadId, revision);
    try {
      await this.client.request("thread/name/set", { threadId, name: trimmed });
      if (this.pendingThreadRenames.get(threadId) === revision
          && this.titleRevisions.get(threadId) === revision) this.recordThreadName(threadId, trimmed);
    } finally {
      if (this.pendingThreadRenames.get(threadId) === revision) {
        this.pendingThreadRenames.delete(threadId);
        this.advanceTitleRevision(threadId);
      }
    }
  }

  /** 스레드를 보관한다. 목록에서 빠지고 다른 창에는 `thread/archived` 로 알려진다. */
  async archiveThread(threadId: string): Promise<void> {
    this.assertWritable();
    if (this.store.getActiveThreadIds().includes(threadId)) throw new Error("진행 중인 턴이 있어 보관할 수 없습니다.");
    await this.client.request("thread/archive", { threadId });
    this.nextTurnQueue.pause(threadId, "대화가 보관되거나 삭제되어 예약을 멈췄습니다.");
    this.removedThreadIds.add(threadId);
    this.provisionalTitles.forget(threadId);
    this.addedThreads.delete(threadId);
    this.store.removeThread(threadId);
  }

  /** 보관을 되돌린다. 응답의 스레드를 목록에 다시 넣는다. */
  async unarchiveThread(threadId: string): Promise<void> {
    this.assertWritable();
    const response = await this.client.request<ThreadUnarchiveResponse>("thread/unarchive", { threadId });
    this.removedThreadIds.delete(threadId);
    this.addedThreads.set(threadId, response.thread);
    this.store.mergeFolders(groupThreadsByCwd([response.thread]));
  }

  /** 대화 폴더의 파일을 찾는다 (#2837). 상위 20개의 폴더 기준 경로이다. */
  async searchFiles(cwd: string, query: string): Promise<string[]> {
    const response = await this.client.request<FuzzyFileSearchResponse>("fuzzyFileSearch", { query, roots: [cwd], cancellationToken: null });
    return response.files.slice(0, FILE_SEARCH_LIMIT).map((file) => file.path);
  }

  /** 승인 요청에 답한다. 다른 창이 먼저 답했으면 이미 닫혀 있으므로 아무것도 하지 않는다. */
  answerApproval(requestId: JsonRpcId, decision: ApprovalDecision): void {
    const pending = this.approvalAnswers.get(approvalKey(requestId));
    const permissions = pending?.approval.permissions;
    this.answerServerRequest(requestId, permissions ? permissionsResponse(permissions, decision) : { decision });
  }

  /** 서버 요청에 그 요청의 응답 형식으로 답한다 (#2836). 이미 닫힌 요청이면 아무것도 하지 않는다. */
  answerServerRequest(requestId: JsonRpcId, result: unknown): void {
    const key = approvalKey(requestId);
    const pending = this.approvalAnswers.get(key);
    this.approvalAnswers.delete(key);
    this.store.removeApproval(requestId);
    pending?.answer(result);
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

  private assertSubscribed(threadId: string | null): void {
    if (threadId !== null && threadId === this.writerElsewhereThreadId) {
      throw new Error(WRITER_ELSEWHERE_SEND_ERROR);
    }
  }

  /**
   * 열린 스레드를 다시 연다. 다른 Codex 앱이 쓰기 잠금을 놓았으면 구독해서 보낼 수 있게 되고, 아직 가지고
   * 있으면 최신 기록을 다시 읽는다.
   */
  reopenThread(): Promise<void> {
    return this.resumeOpenThread();
  }

  private handleServerRequest(method: string, params: unknown, id: JsonRpcId): Promise<unknown> {
    const approval = this.canSend() ? toPendingApproval(method, params, id) : null;
    if (!approval) {
      // 조용히 실패하지 않는다: 거절한 요청과 이유를 사용자에게 알린다.
      this.store.notice?.(this.canSend()
        ? `Akra Desktop 이 처리하지 않는 서버 요청을 거절했습니다: ${method}`
        : `보내기를 허락하지 않은 원격 세션이라 서버 요청을 거절했습니다: ${method}`);
      return Promise.reject(new JsonRpcError(-32601, `Akra Desktop does not handle ${method} yet`));
    }
    this.store.addApproval(approval);
    return new Promise((resolve) => this.approvalAnswers.set(approvalKey(id), { approval, answer: resolve }));
  }

  /** 승인 요청이 다른 곳에서 닫혔다. 화면에서 지우고 응답은 보내지 않는다 (Q-03). */
  private resolveApprovalElsewhere(requestId: JsonRpcId): void {
    const key = approvalKey(requestId);
    const pending = this.approvalAnswers.get(key);
    this.approvalAnswers.delete(key);
    this.store.removeApproval(requestId);
    pending?.answer(NO_RESPONSE);
  }

  /** MCP hub 쓰기 도구 호출에 답한다 (#2830). 다른 창이 먼저 답했으면 Gateway 가 받지 않는다. */
  async answerMcpWrite(id: string, approve: boolean): Promise<void> {
    removeMcpWriteApproval(id);
    await this.client.request(MCP_WRITE_APPROVAL_ANSWER, { id, approve });
  }

  private dropPendingApprovals(): void {
    if (!this.readOnly) clearMcpWriteApprovals();
    for (const pending of this.approvalAnswers.values()) {
      pending.answer(NO_RESPONSE);
    }
    this.approvalAnswers.clear();
    this.store.clearApprovals();
  }

  /** 스레드를 연다. 이전 스레드는 진행 중인 턴이 없으면 구독을 끝낸다. */
  async openThread(threadId: string): Promise<void> {
    this.leaveOpenThread(threadId);
    this.invalidateConversation();
    this.batcher.clear();
    this.store.setOpenThread(threadId);
    await this.resumeOpenThread();
  }

  /** 다른 스레드로 옮기기 전에 이전 스레드의 구독을 끝낸다. 진행 중인 턴이 있거나 읽기만 한 스레드는 그대로 둔다. */
  private leaveOpenThread(nextThreadId: string): void {
    const previousId = this.store.getOpenThreadId();
    const previousSubscribed = previousId !== this.writerElsewhereThreadId;
    if (this.canSend() && previousId && previousSubscribed && previousId !== nextThreadId && !isTurnInProgress(this.store.getTranscript())) {
      void this.client.request("thread/unsubscribe", { threadId: previousId }).catch(() => undefined);
    }
  }

  private async initialize(): Promise<void> {
    const settingsEpoch = this.nextTurnSettings.connectionChanged(false);
    this.client.reset("reconnected");
    // 커널 붙음은 접속마다 따로이다. 대기 중 요청을 정리한 뒤 다시 붙는다.
    this.kernel.connectionChanged(true);
    try {
      await this.client.request("initialize", initializeParams(this.clientVersion));
      this.client.notify("initialized");
      // 승인 카드를 띄우는 이 창만 MCP hub 쓰기 승인을 받겠다고 밝힌다 (#2830). 이전 Gateway 는 모르는 method 로 답한다.
      if (!this.readOnly) void this.client.request(MCP_WRITE_APPROVAL_SUBSCRIBE).catch(() => undefined);
      this.nextTurnSettings.connectionChanged(true, settingsEpoch);
      const gateway = await this.client.request<GatewayInfo>("akra/gateway/info");
      // 목록을 읽는 동안 온 상태 변경은 목록의 상태보다 새롭다. 따로 모았다가 목록 결과 위에 덮는다.
      const statusChanges = new Map<string, boolean>();
      this.statusChangesWhileListing = statusChanges;
      const generation = ++this.listGeneration;
      this.removedThreadIds.clear();
      this.renamedThreads.clear();
      this.titleRevisions.clear();
      this.pendingThreadRenames.clear();
      this.titleRevision += 1;
      this.provisionalTitles.clear();
      this.addedThreads.clear();
      const titleReadRevision = this.titleRevision;
      const threads = await this.listAllThreads(true).finally(() => {
        this.statusChangesWhileListing = null;
      });
      this.store.setSession({
        gateway,
        versionCheck: checkCodexVersion(GENERATED_CODEX_VERSION, gateway.codexVersion),
        folders: groupThreadsByCwd(this.withLifecycleChanges(threads, titleReadRevision)),
      });
      // 이 창이 아직 구독하지 않은 스레드도 목록의 상태로 진행 중 표시를 채운다.
      this.store.setActiveThreads(activeThreadIds(threads, statusChanges));
      // 원격 호스트는 목록을 한 번만 읽는다 (plan 25 §8).
      if (!this.readOnly) {
        void this.addThreadsFromRolloutScan(generation, titleReadRevision);
        void this.readAccountLimits();
      } else {
        void this.loadDots();
        this.chatHistory?.connected();
      }
      if (this.store.getOpenThreadId()) {
        await this.resumeOpenThread();
      }
    } catch (error) {
      this.store.setSessionError(errorMessage(error));
    }
  }

  private accountLimitsReading = false;

  /** 계정 사용 한도를 읽는다 (#2821). 지원하지 않는 app-server 이면 표시하지 않는다. 읽기 실패는 "알 수 없음" 이다. */
  private async readAccountLimits(): Promise<void> {
    this.accountLimitsReading = true;
    try {
      applyAccountLimitsRead(await this.client.request(ACCOUNT_LIMITS_READ, { excludeResetCreditDetails: true }));
    } catch (error) {
      setAccountLimitsStatus(error instanceof JsonRpcError && error.code === -32601 ? "unsupported" : "unavailable");
    } finally {
      this.accountLimitsReading = false;
    }
  }

  /** 원격 호스트의 dot 목록을 한 번 읽는다. 실패하면(dots 가 없는 호스트 등) 목록을 비운다. 대화 목록은 그대로 보인다. */
  private async loadDots(): Promise<void> {
    try {
      const response = await this.client.request<{ dots: DotSummary[] }>(DOTS_LIST_METHOD);
      this.store.setDots(response.dots);
      // dot 마다 방의 실시간 상태를 본다. 보는 화면이 없어도 본다: 답이 오면 알림과 안 읽음 표시를 만든다.
      // 접속마다 다시 요청한다. 실패해도(이전 Gateway 등) 대화 읽기는 된다.
      for (const dot of response.dots) {
        if (dot.messagingRoomId !== null) {
          void this.client.request(DOTS_WATCH_METHOD, { roomId: dot.messagingRoomId }).catch(() => undefined);
        }
      }
    } catch {
      this.store.setDots([]);
    }
  }

  async reloadChats(): Promise<void> {
    if (this.connected) await this.chatHistory?.reload();
  }

  async readChat(id: string): Promise<void> {
    if (!this.connected) {
      this.chatHistory?.invalidate("Gateway 에 연결되어 있지 않습니다.");
      return;
    }
    await this.chatHistory?.read(id);
  }

  /**
   * 상태 DB 에 아직 반영되지 않은 스레드를 rollout 전체 검사로 찾아 사이드바에 더한다. 화면은 상태 DB 목록으로
   * 먼저 보여 준다. 2026-11-06 실측 (rollout 1,465개, 18 GB): 상태 DB 67ms, 전체 검사 27s. 검사 중에는 스레드
   * 열기가 0.8s 에서 3.4s 로 느려진다.
   */
  private async addThreadsFromRolloutScan(generation: number, titleReadRevision: number): Promise<void> {
    try {
      const threads = await this.listAllThreads(false);
      if (generation === this.listGeneration && this.connected) {
        this.store.mergeFolders(groupThreadsByCwd(this.withLifecycleChanges(threads, titleReadRevision)));
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

  private invalidateConversation(): number {
    this.acknowledgedTurnStarts.clear();
    this.nextTurnQueue.pauseAll("대화 또는 접속이 바뀌어 예약을 멈췄습니다. 해당 대화에서 계속하세요.");
    this.loading = null;
    return ++this.conversationGeneration;
  }

  /** `alreadyOpened` 는 이미 구독한 스레드(예: 방금 분기한 대화)이다. 그때는 resume 하지 않고 턴만 읽는다. */
  private async resumeOpenThread(alreadyOpened?: OpenedThread): Promise<void> {
    const generation = this.invalidateConversation();
    const threadId = this.store.getOpenThreadId();
    if (!threadId || !this.connected) {
      return;
    }
    const settingsRevision = this.nextTurnSettings.snapshot(threadId).currentRevision;
    const titleReadRevision = this.titleRevision;
    // 요청마다 버퍼를 따로 둔다. 빠르게 스레드를 바꿔 resume 이 겹쳐도 먼저 끝난 요청이 다음 요청의 버퍼를 지우지 않는다.
    const loading: ResumeLoading = { threadId, buffer: [] };
    this.loading = loading;
    // Thread ids can repeat in an A → B → A navigation, so compare this request's identity too.
    const isCurrent = () => this.conversationGeneration === generation && this.loading === loading && this.store.getOpenThreadId() === threadId;
    try {
      // 긴 스레드의 전체 resume 은 수십 MB 가 된다. 턴 없이 구독만 하고 최근 턴부터 페이지로 읽는다.
      // 허락 전 원격 호스트는 구독하지 않고 thread/read 로 한 번만 읽는다. 허락하면 로컬과 같이 thread/resume 으로
      // 구독해 실시간으로 받는다 (plan 25 "세션 한 번 허용").
      const opened = alreadyOpened ?? (this.canSend() ? await this.subscribeOrRead(threadId) : { thread: await this.readThread(threadId), writerElsewhere: false });
      if (!isCurrent()) {
        return;
      }
      // thread/list 는 분기 계보를 생략할 수 있다. 재연결/재시작 뒤 실제로 연 대화의 서버 응답으로 복원한다.
      if (opened.thread.id === threadId && opened.thread.forkedFromId && !this.removedThreadIds.has(threadId)) {
        this.store.mergeFolders(groupThreadsByCwd([opened.thread]));
      }
      this.writerElsewhereThreadId = opened.writerElsewhere ? threadId : null;
      this.store.setWriterElsewhere(opened.writerElsewhere);
      this.nextTurnSettings.observeSnapshot(threadId, opened.model ?? opened.thread.model,
        opened.effort ?? opened.thread.reasoningEffort, settingsRevision, { approvalPolicy: opened.approvalPolicy, sandbox: opened.sandbox });
      if (opened.writerElsewhere) this.provisionalTitles.forget(threadId);
      let transcript = transcriptFromThread({ ...opened.thread, preview: this.provisionalTitles.preview(threadId, null, opened.thread.preview) });
      // 서버가 턴을 함께 보냈으면(예: 녹화 재생) 그대로 쓴다.
      if (transcript.turns.length === 0) {
        const page = await this.readTurnPage(threadId, null);
        if (!isCurrent()) {
          return;
        }
        transcript = prependOlderTurns(transcript, page.data, page.nextCursor);
      }
      // Reconcile the authoritative name only after the complete resume is still current.
      const summary = this.store.getWorkspaceThread(threadId);
      // Recovery of a missing fork origin must not overwrite a newer sidebar title
      // with metadata from an older thread/read or thread/resume response.
      const olderThanSummary = summary !== undefined && summary.updatedAt > opened.thread.updatedAt;
      if (opened.thread.id === threadId && !olderThanSummary) this.observeThreadName(opened.thread, titleReadRevision);
      const title = this.renamedThreads.get(threadId) ?? (olderThanSummary ? summary.title : undefined);
      const loaded = applyNotifications(transcript, loading.buffer);
      this.store.setTranscript(title === undefined ? loaded : { ...loaded, title });
    } catch (error) {
      if (isCurrent()) {
        this.store.setTranscript(null, errorMessage(error));
      }
    } finally {
      if (this.loading === loading) {
        this.loading = null;
      }
    }
  }

  /** 구독해서 연다. 다른 Codex 프로세스가 쓰기 잠금을 가지고 있으면 구독하지 않고 읽기만 한다. */
  private async subscribeOrRead(threadId: string): Promise<OpenedThread> {
    try {
      const response = await this.client.request<ThreadResumeResponse>("thread/resume", { threadId, excludeTurns: true });
      return { thread: response.thread, writerElsewhere: false, model: response.model, effort: response.reasoningEffort,
        approvalPolicy: response.approvalPolicy, sandbox: response.sandbox };
    } catch (error) {
      if (!(error instanceof JsonRpcError && WRITER_ELSEWHERE_ERROR.test(error.message))) {
        throw error;
      }
      return { thread: await this.readThread(threadId), writerElsewhere: true };
    }
  }

  /** 목록 결과에 이 접속 중 생기거나 보관을 푼 스레드를 더하고, 보관·삭제된 스레드를 빼고, 바뀐 제목을 넣는다. */
  private withLifecycleChanges(threads: readonly Thread[], titleReadRevision: number): Thread[] {
    for (const thread of threads) this.observeThreadName(thread, titleReadRevision);
    return [...threads, ...this.addedThreads.values()]
      .filter((thread) => !this.removedThreadIds.has(thread.id))
      .map((thread) => {
        const name = this.renamedThreads.get(thread.id);
        const observed = name ?? thread.name;
        return { ...thread, name: observed, preview: this.provisionalTitles.preview(thread.id, observed, thread.preview) };
      });
  }

  private advanceTitleRevision(threadId: string): number {
    const revision = ++this.titleRevision;
    this.titleRevisions.set(threadId, revision);
    return revision;
  }

  private recordThreadName(threadId: string, name: string): void {
    this.advanceTitleRevision(threadId);
    this.provisionalTitles.forget(threadId);
    this.renamedThreads.set(threadId, name);
    this.store.renameThread(threadId, name);
    this.store.updateTranscript(current => current.threadId === threadId ? { ...current, title: name } : current);
  }

  private observeThreadName(thread: Thread, readRevision: number): void {
    if (!thread.name?.trim() || this.removedThreadIds.has(thread.id)
        || this.pendingThreadRenames.has(thread.id)
        || (this.titleRevisions.get(thread.id) ?? 0) > readRevision) return;
    this.recordThreadName(thread.id, thread.name);
  }

  /** 다른 곳에서 보관을 푼 스레드를 읽어 사이드바에 다시 넣는다. 알림에는 스레드 id 만 있다. */
  private async addUnarchivedThread(threadId: string): Promise<void> {
    try {
      const thread = await this.readThread(threadId);
      // 읽는 동안 다시 보관했으면 넣지 않는다.
      if (this.removedThreadIds.has(threadId)) return;
      this.addedThreads.set(threadId, thread);
      this.store.mergeFolders(groupThreadsByCwd([thread]));
    } catch {
      // 다시 연결할 때 읽는 목록이 채운다.
    }
  }

  private async readThread(threadId: string): Promise<Thread> {
    const response = await this.client.request<ThreadReadResponse>("thread/read", { threadId, includeTurns: false });
    return response.thread;
  }

  /** 열린 스레드의 이전 턴을 한 페이지 더 읽어 앞에 붙인다. */
  async loadOlderTurns(): Promise<void> {
    const transcript = this.store.getTranscript();
    const cursor = transcript?.olderTurnsCursor;
    const generation = this.conversationGeneration;
    if (!transcript || !cursor || this.loading !== null) {
      return;
    }
    const page = await this.readTurnPage(transcript.threadId, cursor);
    // 같은 세대의 현재 transcript에 병합한다. 같은 커서의 늦은 중복 응답은 적용하지 않는다.
    if (this.conversationGeneration === generation && this.store.getOpenThreadId() === transcript.threadId) {
      this.store.updateTranscript((current) =>
        current.threadId === transcript.threadId && current.olderTurnsCursor === cursor
          ? prependOlderTurns(current, page.data, page.nextCursor)
          : current);
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
    // 압축 턴의 최종 결과를 먼저 반영한 뒤 예약 입력의 다음 dispatch 여부를 판단한다.
    this.compaction.observe(method, params);
    if (!this.readOnly && applyAccountLimitsNotification(method, params)) {
      // 처음 읽기가 실패했으면 알림이 올 때 기본 버킷을 다시 읽는다. 알림의 버킷을 임의로 보이지 않는다.
      if (useAccountLimits.getState().status === "unavailable" && !this.accountLimitsReading) void this.readAccountLimits();
      return;
    }
    // MCP hub 쓰기 승인은 이 기기의 Gateway(local 접속)만 보낸다 (#2830).
    if (!this.readOnly && applyMcpWriteApprovalNotification(method, params)) return;
    if (method === "akra/providerCredentials/changed") {
      if (!this.readOnly && params && typeof params === "object") {
        const change = params as { provider?: unknown; profileId?: unknown; revision?: unknown; lagged?: unknown };
        if (change.provider === "jira" && typeof change.profileId === "string" && Number.isSafeInteger(change.revision)) {
          const scope = change.profileId + ":" + String(change.revision);
          if (scope !== this.providerVersionScope || change.lagged === true) {
            this.providerVersionScope = scope;
            for (const listener of this.versionProviderListeners) listener();
          }
        }
      }
      return;
    }
    if (method === KERNEL_METHOD.event) {
      this.kernel.handleEvent(params);
      return;
    }
    if (method === "skills/changed") {
      for (const listener of this.skillsChangedListeners) listener();
      return;
    }
    if (method === DOTS_LIVE_NOTIFICATION) {
      const state = params as Partial<DotLiveState> | null;
      if (typeof state?.roomId === "string") {
        this.store.applyDotLive({ roomId: state.roomId, connected: state.connected === true, working: state.working === true, activityLabel: state.activityLabel ?? null, usingComputer: state.usingComputer === true });
      }
      return;
    }
    if (method === "thread/started") {
      const thread = (params as { thread?: Thread } | null)?.thread;
      if (typeof thread?.id === "string") this.nextTurnSettings.observeSnapshot(thread.id, thread.model, thread.reasoningEffort);
      // thread/list 처럼 하위 에이전트와 디스크에 남지 않는 스레드는 목록에 넣지 않는다. 이미 있는 스레드는 그대로 둔다.
      if (thread && !thread.ephemeral && !thread.parentThreadId) {
        this.addedThreads.set(thread.id, thread);
        this.store.mergeFolders(groupThreadsByCwd([thread]));
      }
      return;
    }
    const threadId = notificationThreadId(params);
    if (!threadId) {
      return;
    }
    if (method === "thread/settings/updated") {
      const settings = (params as { threadSettings?: { model?: unknown; effort?: unknown; approvalPolicy?: unknown; sandboxPolicy?: unknown } } | null)?.threadSettings;
      if (settings && typeof settings === "object") this.nextTurnSettings.observeNotification(threadId, settings.model, settings.effort,
        { approvalPolicy: settings.approvalPolicy, sandbox: settings.sandboxPolicy });
      return;
    }
    if (method === "turn/completed") {
      const turn = (params as { turn?: { id?: unknown; status?: unknown } }).turn;
      if (typeof turn?.id === "string" && (turn.status === "completed" || turn.status === "failed" || turn.status === "interrupted")) {
        if (this.acknowledgedTurnStarts.get(threadId) === turn.id) this.acknowledgedTurnStarts.delete(threadId);
        this.nextTurnQueue.terminal(threadId, turn.id, turn.status);
      }
    }
    if (method === "thread/name/updated") {
      const name = (params as { threadName?: string }).threadName;
      if (name) this.recordThreadName(threadId, name);
      // The name was applied immediately to both projections. Do not batch it again:
      // a delayed frame could otherwise overwrite a newer manual rename.
      return;
    }
    if (method === "thread/archived" || method === "thread/deleted") {
      this.nextTurnQueue.pause(threadId, "대화가 보관되거나 삭제되어 예약을 멈췄습니다.");
      this.removedThreadIds.add(threadId);
      this.provisionalTitles.forget(threadId);
      this.addedThreads.delete(threadId);
      this.store.removeThread(threadId);
    }
    if (method === "thread/unarchived") {
      this.removedThreadIds.delete(threadId);
      void this.addUnarchivedThread(threadId);
    }
    if (method === "thread/status/changed") {
      const status = (params as { status: ThreadStatus }).status.type;
      const active = status === "active";
      this.statusChangesWhileListing?.set(threadId, active);
      // 실패를 먼저 적는다: "진행 중"이 끝나는 순간에 화면과 알림이 실패인지 안다.
      this.store.setThreadFailed(threadId, status === "systemError");
      this.store.setThreadActive(threadId, active);
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

const FILE_SEARCH_LIMIT = 20;

/** 글과 첨부 이미지(#2838)를 턴 입력으로 만든다. 이미지는 글 뒤에 붙는다. 원본은 로그에 남기지 않는다(Gateway 녹화도 가린다). */
export function userInput(text: string, images: readonly string[] = []): UserInput[] {
  return [
    ...(text ? [{ type: "text" as const, text, text_elements: [] }] : []),
    ...images.map((url) => ({ type: "image" as const, url })),
  ];
}

/** 메시지 버스와 세션을 잇는다. 반환한 함수로 연결을 끊는다. */
export function startDesktopSession(bridge: AkraBridge, store: SessionStore, clientVersion: string): () => void {
  const session = new DesktopSession(bridge, store, clientVersion);
  session.start();
  return () => session.stop();
}
