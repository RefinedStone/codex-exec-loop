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
import type { InitializeParams } from "../../shared/protocol/generated/InitializeParams";
import type { SkillsListParams } from "../../shared/protocol/generated/v2/SkillsListParams";
import type { SkillsListResponse } from "../../shared/protocol/generated/v2/SkillsListResponse";
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
import { KernelConnection, type IssueDecisionAttention, type KernelInterest } from "../kernel/kernelConnection";
import { TaskAuthoringController } from "../kernel/taskAuthoring";
import type { IssueAutonomyLevel, IssueCommandOutcome, IssueConversationTarget, IssueInterventionName } from "../kernel/issueInteractions";
import { KERNEL_METHOD, type IssueAutomationModeRecord, type IssueEventReference } from "../kernel/protocol";
import { JsonRpcClient, JsonRpcError, type JsonRpcId, NO_RESPONSE } from "../rpc/jsonRpcClient";
import { checkCodexVersion } from "./codexVersion";
import { ChatHistory } from "./chatHistory";
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
const GATEWAY_SERVICE_PREFIXES = ["akra/mcp/", "akra/context/", "akra/dots/", "akra/git/", "akra/issue/", "akra/providerCredentials/", "akra/providers/"];
/** 원격 호스트의 dot 목록이다 (D-19). dots 가 꺼진 Gateway 는 모르는 method 로 답한다. */
const DOTS_LIST_METHOD = "akra/dots/list";
/** 방의 실시간 상태를 보기 시작한다. 그 뒤 `akra/dots/live` 알림이 온다. */
const DOTS_WATCH_METHOD = "akra/dots/watch";
const DOTS_LIVE_NOTIFICATION = "akra/dots/live";
/** 세션이 쓰는 메시지 버스이다. `local` 은 `window.akra` 이고, 원격 호스트는 `remoteHostBridge` 이다. */
export type SessionBridge = Pick<AkraBridge, "send" | "onMessage" | "onConnectionState" | "ready"> & Partial<Pick<AkraBridge, "confirmIssueAutomationMode" | "confirmIssueDecision" | "confirmIssueAutonomy" | "confirmIssueBindingResume">> & {
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
}

export class DesktopSession {
  private readonly versionProviderListeners = new Set<() => void>();

  onVersionProviderReady(listener: () => void): () => void {
    this.versionProviderListeners.add(listener);
    return () => this.versionProviderListeners.delete(listener);
  }
  private readonly client: JsonRpcClient;
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
  /** 이 접속에서 생기거나 보관을 푼 스레드이다. 그 전에 읽기 시작한 목록에는 없을 수 있다. */
  private addedThreads = new Map<string, Thread>();
  /** 사용자의 답을 기다리는 승인 요청의 응답 함수이다. 키는 요청 id 이다. */
  private readonly approvalAnswers = new Map<string, (answer: unknown) => void>();
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
  ) {
    this.client = new JsonRpcClient((text) => bridge.send(text));
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
        if (this.connected) {
          void this.initialize();
        } else {
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
  async sendMessage(text: string): Promise<void> {
    this.assertWritable();
    const threadId = this.store.getOpenThreadId();
    this.assertSubscribed(threadId);
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
    this.assertSubscribed(threadId);
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
    this.invalidateConversation();
    this.batcher.clear();
    this.store.setOpenThread(response.thread.id);
    this.store.setTranscript(transcript);
    this.store.addThreadToFolder(response.thread.cwd, workspaceThread(response.thread, transcript.title));
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
      await agent.setActiveThread(threadId);
      await agent.bind(registration.registrationId, threadId);
      let cursor: string | null = null;
      let verified = false;
      do {
        const status: ListMcpServerStatusResponse = await this.client.request("mcpServerStatus/list", {
          threadId, detail: "toolsAndAuthOnly", ...(cursor ? { cursor } : {}),
        } satisfies ListMcpServerStatusParams);
        const browser = status.data.find((server) => server.name === "akra_browser");
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
    const previousSubscribed = previousId !== this.writerElsewhereThreadId;
    if (this.canSend() && previousId && previousSubscribed && previousId !== threadId && !isTurnInProgress(this.store.getTranscript())) {
      void this.client.request("thread/unsubscribe", { threadId: previousId }).catch(() => undefined);
    }
    this.invalidateConversation();
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
      this.removedThreadIds.clear();
      this.renamedThreads.clear();
      this.addedThreads.clear();
      const threads = await this.listAllThreads(true).finally(() => {
        this.statusChangesWhileListing = null;
      });
      this.store.setSession({
        gateway,
        versionCheck: checkCodexVersion(GENERATED_CODEX_VERSION, gateway.codexVersion),
        folders: groupThreadsByCwd(this.withLifecycleChanges(threads)),
      });
      // 이 창이 아직 구독하지 않은 스레드도 목록의 상태로 진행 중 표시를 채운다.
      this.store.setActiveThreads(activeThreadIds(threads, statusChanges));
      // 원격 호스트는 목록을 한 번만 읽는다 (plan 25 §8).
      if (!this.readOnly) {
        void this.addThreadsFromRolloutScan(generation);
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
  private async addThreadsFromRolloutScan(generation: number): Promise<void> {
    try {
      const threads = await this.listAllThreads(false);
      if (generation === this.listGeneration) {
        this.store.mergeFolders(groupThreadsByCwd(this.withLifecycleChanges(threads)));
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
    this.loading = null;
    return ++this.conversationGeneration;
  }

  private async resumeOpenThread(): Promise<void> {
    const generation = this.invalidateConversation();
    const threadId = this.store.getOpenThreadId();
    if (!threadId || !this.connected) {
      return;
    }
    // 요청마다 버퍼를 따로 둔다. 빠르게 스레드를 바꿔 resume 이 겹쳐도 먼저 끝난 요청이 다음 요청의 버퍼를 지우지 않는다.
    const loading: ResumeLoading = { threadId, buffer: [] };
    this.loading = loading;
    // Thread ids can repeat in an A → B → A navigation, so compare this request's identity too.
    const isCurrent = () => this.conversationGeneration === generation && this.loading === loading && this.store.getOpenThreadId() === threadId;
    try {
      // 긴 스레드의 전체 resume 은 수십 MB 가 된다. 턴 없이 구독만 하고 최근 턴부터 페이지로 읽는다.
      // 허락 전 원격 호스트는 구독하지 않고 thread/read 로 한 번만 읽는다. 허락하면 로컬과 같이 thread/resume 으로
      // 구독해 실시간으로 받는다 (plan 25 "세션 한 번 허용").
      const opened = this.canSend() ? await this.subscribeOrRead(threadId) : { thread: await this.readThread(threadId), writerElsewhere: false };
      if (!isCurrent()) {
        return;
      }
      this.writerElsewhereThreadId = opened.writerElsewhere ? threadId : null;
      this.store.setWriterElsewhere(opened.writerElsewhere);
      let transcript = transcriptFromThread(opened.thread);
      // 서버가 턴을 함께 보냈으면(예: 녹화 재생) 그대로 쓴다.
      if (transcript.turns.length === 0) {
        const page = await this.readTurnPage(threadId, null);
        if (!isCurrent()) {
          return;
        }
        transcript = prependOlderTurns(transcript, page.data, page.nextCursor);
      }
      this.store.setTranscript(applyNotifications(transcript, loading.buffer));
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
      return { thread: response.thread, writerElsewhere: false };
    } catch (error) {
      if (!(error instanceof JsonRpcError && WRITER_ELSEWHERE_ERROR.test(error.message))) {
        throw error;
      }
      return { thread: await this.readThread(threadId), writerElsewhere: true };
    }
  }

  /** 목록 결과에 이 접속 중 생기거나 보관을 푼 스레드를 더하고, 보관·삭제된 스레드를 빼고, 바뀐 제목을 넣는다. */
  private withLifecycleChanges(threads: readonly Thread[]): Thread[] {
    return [...threads, ...this.addedThreads.values()]
      .filter((thread) => !this.removedThreadIds.has(thread.id))
      .map((thread) => {
        const name = this.renamedThreads.get(thread.id);
        return name === undefined ? thread : { ...thread, name };
      });
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
    if (method === "thread/name/updated") {
      const name = (params as { threadName?: string }).threadName;
      if (name) {
        this.renamedThreads.set(threadId, name);
        this.store.renameThread(threadId, name);
      }
    }
    if (method === "thread/archived" || method === "thread/deleted") {
      this.removedThreadIds.add(threadId);
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

/** 메시지 버스와 세션을 잇는다. 반환한 함수로 연결을 끊는다. */
export function startDesktopSession(bridge: AkraBridge, store: SessionStore, clientVersion: string): () => void {
  const session = new DesktopSession(bridge, store, clientVersion);
  session.start();
  return () => session.stop();
}
