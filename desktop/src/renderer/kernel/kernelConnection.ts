// 같은 Gateway 접속에서 저장소 커널에 한 번 붙고, 열린 패널의 관심 영역만 읽는다.
// 개요와 이슈 목록은 서로 다른 조회 계약을 쓰며, 이슈를 읽는 동안 모델 턴을 만들지 않는다.
import { JsonRpcError, type JsonRpcClient } from "../rpc/jsonRpcClient";
import {
  type AttachResponse,
  type AutoFollowSettings,
  type IssueAutomationAnswer,
  type IssueAutomationModeRecord,
  type IssuesPageAnswer,
  KERNEL_METHOD,
  KERNEL_UNAVAILABLE,
  KERNEL_VIEW_QUERIES,
  type KernelCommand,
  type KernelEvent,
  type KernelViewAnswer,
  type ParallelState,
  type PlanningState,
  type ThreadActivity,
} from "./protocol";
import { sendKernelCommand } from "./sendKernelCommand";

export type KernelPhase = "idle" | "attaching" | "attached" | "failed";
export type KernelInterest = "overview" | "issues";

export interface KernelIssuesView {
  automation: IssueAutomationAnswer | null;
  /** 서버가 정한 순서와 묶음을 가진 현재 한 페이지다. 이전 페이지를 합치지 않는다. */
  page: IssuesPageAnswer | null;
  loading: boolean;
  pageNumber: number;
  canGoBack: boolean;
  commandPending: "mode" | "sync" | null;
  error: string | null;
  /** 보이는 페이지가 새 revision보다 오래되었거나 다시 읽기에 실패했다. */
  stale: boolean;
}

export const EMPTY_KERNEL_ISSUES: KernelIssuesView = {
  automation: null,
  page: null,
  loading: false,
  pageNumber: 1,
  canGoBack: false,
  commandPending: null,
  error: null,
  stale: false,
};

export interface KernelView {
  phase: KernelPhase;
  workspace: string | null;
  /** 붙은 커널의 저장소 신원이다. */
  kernel: string | null;
  message: string | null;
  threads: ThreadActivity[];
  autoFollow: AutoFollowSettings | null;
  planning: PlanningState | null;
  parallel: ParallelState | null;
  issues: KernelIssuesView;
}

export const EMPTY_KERNEL_VIEW: KernelView = {
  phase: "idle",
  workspace: null,
  kernel: null,
  message: null,
  threads: [],
  autoFollow: null,
  planning: null,
  parallel: null,
  issues: EMPTY_KERNEL_ISSUES,
};

export interface KernelStore {
  getKernel(): KernelView;
  setKernel(view: KernelView): void;
}

interface IssueRequestToken {
  connection: number;
  interest: number;
  request: number;
}

interface IssuePageRequest {
  token: IssueRequestToken;
  after: string | null;
}

const CLIENT_NAME = "akra-desktop";
const ISSUE_PAGE_SIZE = 50;

function failureMessage(error: unknown): string {
  if (error instanceof JsonRpcError && error.code === KERNEL_UNAVAILABLE) {
    return `커널에 붙을 수 없습니다. 같은 저장소에서 TUI 가 실행 중이면 TUI 를 닫으세요. (${error.message})`;
  }
  return error instanceof Error ? error.message : String(error);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 표시 계약의 모양만 검사한다. 상태·이유·묶음의 의미는 서버가 정한다. */
function isIssuesPageAnswer(value: unknown): value is IssuesPageAnswer {
  if (!isRecord(value) || value.type !== "issues") return false;
  if (value.status === "unavailable") return true;
  if (value.status !== "ready" || !Number.isSafeInteger(value.revision) || Number(value.revision) < 0) return false;
  if (!Array.isArray(value.items) || !Array.isArray(value.groups)) return false;
  if (value.next_after !== undefined && value.next_after !== null && typeof value.next_after !== "string") return false;
  if (!value.items.every((item: unknown) => {
    if (!isRecord(item) || !isRecord(item.presentation)) return false;
    const shown = item.presentation;
    if (shown.number !== null && (!Number.isSafeInteger(shown.number) || Number(shown.number) < 1)) return false;
    if (shown.title !== null && typeof shown.title !== "string") return false;
    if (shown.url !== null && typeof shown.url !== "string") return false;
    return isRecord(shown.status) && typeof shown.status.code === "string" && typeof shown.status.label === "string"
      && isRecord(shown.reason) && typeof shown.reason.code === "string" && typeof shown.reason.summary === "string";
  })) return false;
  const indices = new Set<number>();
  const itemCount = value.items.length;
  for (const group of value.groups as unknown[]) {
    if (!isRecord(group) || typeof group.id !== "string" || typeof group.label !== "string" || !Array.isArray(group.item_indices)) return false;
    for (const index of group.item_indices as unknown[]) {
      if (typeof index !== "number" || !Number.isSafeInteger(index) || index < 0 || index >= itemCount || indices.has(index)) return false;
      indices.add(index);
    }
  }
  return indices.size === itemCount;
}

export class KernelConnection {
  private wanted: string | null = null;
  private interests = new Set<KernelInterest>();
  private connected = false;
  private generation = 0;
  private overviewGeneration = 0;
  private overviewRequest = 0;
  /** resync는 조회 응답을 대체하지만, 명시적인 planning 새로 고침 요청을 취소하지 않는다. */
  private overviewRefreshRequest = 0;
  private readonly eventCounts = new Map<string, number>();

  private issuesGeneration = 0;
  private issueStateRequest = 0;
  private issuePageRequest = 0;
  private issueCommandRequest = 0;
  private activeIssuePage: IssuePageRequest | null = null;
  private latestIssueRevision = 0;
  private currentIssueCursor: string | null = null;
  private previousIssueCursors: Array<string | null> = [];
  private issueStateError: string | null = null;
  private issuePageError: string | null = null;
  private issueCommandError: string | null = null;

  constructor(
    private readonly client: JsonRpcClient,
    private readonly store: KernelStore,
  ) {}

  /** 같은 폴더에서는 관심 영역만 바꾸고, 관심 영역이 없으면 커널에서 뗀다. */
  async show(workspace: string | null, interests: readonly KernelInterest[] = ["overview"]): Promise<void> {
    const next = new Set(interests);
    const wanted = next.size === 0 ? null : workspace;
    const sameWorkspace = wanted === this.wanted;
    const overviewChanged = next.has("overview") !== this.interests.has("overview");
    const issuesChanged = next.has("issues") !== this.interests.has("issues");
    if (sameWorkspace && !overviewChanged && !issuesChanged) return;

    const previous = this.wanted;
    this.wanted = wanted;
    this.interests = next;
    if (overviewChanged || !sameWorkspace) {
      this.overviewGeneration += 1;
      this.overviewRequest += 1;
      this.overviewRefreshRequest += 1;
    }
    if (issuesChanged || !sameWorkspace) this.invalidateIssues();
    if (!sameWorkspace) this.generation += 1;
    this.store.setKernel({ ...this.store.getKernel(), message: null });

    if (wanted === null) {
      this.store.setKernel(EMPTY_KERNEL_VIEW);
      if (previous !== null && this.connected) {
        await this.client.request(KERNEL_METHOD.detach, {}).catch(() => undefined);
      }
      return;
    }
    if (sameWorkspace && this.store.getKernel().phase === "attached") {
      await Promise.all([
        overviewChanged && next.has("overview") ? this.readOverview(true) : Promise.resolve(),
        issuesChanged && next.has("issues") ? this.refreshIssues() : Promise.resolve(),
      ]);
      return;
    }
    if (!sameWorkspace || this.store.getKernel().phase !== "attaching") await this.attach();
  }

  connectionChanged(connected: boolean): void {
    this.connected = connected;
    this.generation += 1;
    this.overviewGeneration += 1;
    this.overviewRequest += 1;
    this.overviewRefreshRequest += 1;
    this.eventCounts.clear();
    this.invalidateIssues();
    void this.attach();
  }

  /** 개요를 요청한 패널에서만 planning 큐를 다시 읽게 한다. */
  async refresh(): Promise<void> {
    if (!this.interests.has("overview")) return;
    if (this.store.getKernel().phase !== "attached") {
      if (this.connected && this.store.getKernel().phase !== "attaching") await this.attach();
      return;
    }
    await this.readOverview(true);
  }

  /** 상태와 첫 페이지를 다시 읽는다. 새로고침은 이전 페이지 이동 요청을 대체한다. */
  async refreshIssues(): Promise<void> {
    if (!this.canRead("issues")) {
      if (this.interests.has("issues") && this.connected && this.store.getKernel().phase !== "attaching") await this.attach();
      return;
    }
    this.issueCommandError = null;
    await Promise.all([this.readIssueAutomation(), this.readIssuesPage(null, [], null)]);
  }

  async nextIssuesPage(): Promise<void> {
    const view = this.store.getKernel().issues;
    if (!this.canNavigateIssues() || view.page?.status !== "ready" || !view.page.next_after) return;
    await this.readIssuesPage(view.page.next_after, [...this.previousIssueCursors, this.currentIssueCursor], view.page.revision);
  }

  async previousIssuesPage(): Promise<void> {
    if (!this.canNavigateIssues() || this.previousIssueCursors.length === 0) return;
    const page = this.store.getKernel().issues.page;
    if (page?.status !== "ready") return;
    const after = this.previousIssueCursors.at(-1) ?? null;
    await this.readIssuesPage(after, this.previousIssueCursors.slice(0, -1), page.revision);
  }

  setIssueAutomationMode(mode: IssueAutomationModeRecord): Promise<void> {
    return this.runIssueCommand({ type: "set_issue_automation_mode", mode }, "mode");
  }

  syncIssues(): Promise<void> {
    return this.runIssueCommand({ type: "sync_issues" }, "sync");
  }

  handleEvent(params: unknown): void {
    const event = (params as { event?: KernelEvent } | undefined)?.event;
    if (!event || this.store.getKernel().phase !== "attached") return;
    this.eventCounts.set(event.type, (this.eventCounts.get(event.type) ?? 0) + 1);
    if (event.type === "resync") {
      if (this.interests.has("overview")) void this.readOverview(false);
      if (this.interests.has("issues")) void this.refreshIssues();
      return;
    }
    if (event.type === "issues") {
      if (this.canRead("issues")) {
        const { type: _type, ...state } = event;
        this.applyIssueAutomation({ ...state, type: "issue_automation" });
      }
      return;
    }
    if (this.interests.has("overview")) this.applyOverview(event);
  }

  private async attach(): Promise<void> {
    const workspace = this.wanted;
    if (workspace === null) return;
    if (!this.connected) {
      this.store.setKernel({ ...EMPTY_KERNEL_VIEW, workspace, message: "Gateway 연결을 기다립니다." });
      return;
    }
    const generation = ++this.generation;
    const previous = this.store.getKernel();
    this.store.setKernel({ ...previous, phase: "attaching", workspace, message: null });
    let attached: AttachResponse;
    try {
      attached = await this.client.request<AttachResponse>(KERNEL_METHOD.attach, { workspace, client_name: CLIENT_NAME });
    } catch (error) {
      if (generation === this.generation) {
        this.store.setKernel({ ...EMPTY_KERNEL_VIEW, phase: "failed", workspace, message: failureMessage(error) });
      }
      return;
    }
    if (generation !== this.generation) return;
    const kept = previous.kernel === attached.kernel ? this.store.getKernel() : EMPTY_KERNEL_VIEW;
    this.store.setKernel({ ...kept, phase: "attached", workspace, kernel: attached.kernel, message: null });
    await Promise.all([
      this.interests.has("overview") ? this.readOverview(true) : Promise.resolve(),
      this.interests.has("issues") ? this.refreshIssues() : Promise.resolve(),
    ]);
  }

  private canRead(interest: KernelInterest): boolean {
    return this.connected && this.wanted !== null && this.interests.has(interest) && this.store.getKernel().phase === "attached";
  }

  private overviewScopeCurrent(generation: number, interest: number): boolean {
    return this.canRead("overview") && generation === this.generation && interest === this.overviewGeneration;
  }

  private overviewCurrent(generation: number, interest: number, request: number): boolean {
    return this.overviewScopeCurrent(generation, interest) && request === this.overviewRequest;
  }

  private overviewRefreshCurrent(generation: number, interest: number, refresh: number): boolean {
    return this.overviewScopeCurrent(generation, interest) && refresh === this.overviewRefreshRequest;
  }

  private async readOverview(refreshPlanning: boolean): Promise<void> {
    if (!this.canRead("overview")) return;
    const generation = this.generation;
    const interest = this.overviewGeneration;
    const request = ++this.overviewRequest;
    const refresh = refreshPlanning ? ++this.overviewRefreshRequest : null;
    this.store.setKernel({ ...this.store.getKernel(), message: null });
    try {
      await Promise.all(KERNEL_VIEW_QUERIES.map(async (type) => {
        const seen = this.eventCounts.get(type) ?? 0;
        const event = await this.client.request<KernelViewAnswer>(KERNEL_METHOD.query, { query: { type } });
        if (this.overviewCurrent(generation, interest, request) && (this.eventCounts.get(type) ?? 0) === seen) {
          this.applyOverview(event);
        }
      }));
      if (refresh !== null) await this.refreshPlanningForOverview(generation, interest, refresh);
    } catch (error) {
      if (this.overviewCurrent(generation, interest, request)) {
        this.store.setKernel({ ...this.store.getKernel(), message: `커널 상태를 읽지 못했습니다: ${failureMessage(error)}` });
      }
    }
  }

  private async refreshPlanningForOverview(generation: number, interest: number, refresh: number): Promise<void> {
    if (!this.overviewRefreshCurrent(generation, interest, refresh)) return;
    try {
      await sendKernelCommand(this.client, { type: "refresh_planning" });
    } catch (error) {
      // 명령 실패는 같은 관심 영역의 resync 조회가 끝났더라도 해당 새로 고침에 남긴다.
      if (this.overviewRefreshCurrent(generation, interest, refresh)) {
        this.store.setKernel({ ...this.store.getKernel(), message: `커널 상태를 읽지 못했습니다: ${failureMessage(error)}` });
      }
    }
  }

  private invalidateIssues(): void {
    this.issuesGeneration += 1;
    this.issueStateRequest += 1;
    this.issuePageRequest += 1;
    this.issueCommandRequest += 1;
    this.activeIssuePage = null;
    this.latestIssueRevision = 0;
    this.currentIssueCursor = null;
    this.previousIssueCursors = [];
    this.issueStateError = null;
    this.issuePageError = null;
    this.issueCommandError = null;
    this.updateIssues(EMPTY_KERNEL_ISSUES);
  }

  private issueToken(request: number): IssueRequestToken {
    return { connection: this.generation, interest: this.issuesGeneration, request };
  }

  private issueCurrent(token: IssueRequestToken, request: number): boolean {
    return this.canRead("issues") && token.connection === this.generation && token.interest === this.issuesGeneration && token.request === request;
  }

  private updateIssues(update: Partial<KernelIssuesView>): void {
    const view = this.store.getKernel();
    this.store.setKernel({
      ...view,
      issues: { ...view.issues, ...update, error: this.issueCommandError ?? this.issuePageError ?? this.issueStateError },
    });
  }

  private async readIssueAutomation(): Promise<void> {
    const token = this.issueToken(++this.issueStateRequest);
    const seen = this.eventCounts.get("issues") ?? 0;
    this.issueStateError = null;
    this.updateIssues({});
    try {
      const state = await this.client.request<IssueAutomationAnswer>(KERNEL_METHOD.query, { query: { type: "issue_automation" } });
      if (this.issueCurrent(token, this.issueStateRequest) && (this.eventCounts.get("issues") ?? 0) === seen) {
        this.applyIssueAutomation(state);
      }
    } catch (error) {
      if (this.issueCurrent(token, this.issueStateRequest) && (this.eventCounts.get("issues") ?? 0) === seen) {
        this.issueStateError = `이슈 수집 상태를 읽지 못했습니다: ${failureMessage(error)}`;
        this.updateIssues({});
      }
    }
  }

  private applyIssueAutomation(automation: IssueAutomationAnswer): void {
    const issues = this.store.getKernel().issues;
    if (automation.status === "ready" && issues.automation?.status === "ready" && automation.revision < issues.automation.revision) return;
    this.issueStateError = null;
    if (automation.status === "ready") this.latestIssueRevision = Math.max(this.latestIssueRevision, automation.revision);
    this.updateIssues({ automation });
    if (automation.status !== "ready") return;
    const pageRevision = issues.page?.status === "ready" ? issues.page.revision : -1;
    if (automation.revision <= pageRevision) return;
    this.updateIssues({ stale: issues.page !== null, canGoBack: false });
    // 첫 페이지 요청은 최신 revision을 확인해 답을 고른다. 이전 커서의 요청은 즉시 대체한다.
    if (this.activeIssuePage?.after === null) return;
    void this.readIssuesPage(null, [], null);
  }

  private canNavigateIssues(): boolean {
    const issues = this.store.getKernel().issues;
    return this.canRead("issues") && !issues.loading && !issues.stale && issues.commandPending === null;
  }

  private async readIssuesPage(
    after: string | null,
    history: Array<string | null>,
    expectedRevision: number | null,
    retried = false,
  ): Promise<void> {
    if (!this.canRead("issues")) return;
    const token = this.issueToken(++this.issuePageRequest);
    const request = { token, after };
    this.activeIssuePage = request;
    this.issuePageError = null;
    this.updateIssues({ loading: true, ...(after === null && history.length === 0 ? { canGoBack: false } : {}) });
    let restart = false;
    try {
      const page = await this.client.request<unknown>(KERNEL_METHOD.query, { query: { type: "issues", after, limit: ISSUE_PAGE_SIZE } });
      if (!this.issueCurrent(token, this.issuePageRequest)) return;
      if (!isIssuesPageAnswer(page)) throw new Error("이슈 표시 계약을 읽을 수 없습니다. Gateway를 업데이트하거나 다시 시작해 주세요.");
      const outdated = page.status === "ready" && (page.revision < this.latestIssueRevision || (expectedRevision !== null && page.revision !== expectedRevision));
      if (page.status === "ready") this.latestIssueRevision = Math.max(this.latestIssueRevision, page.revision);
      if (outdated) {
        restart = true;
      } else {
        this.currentIssueCursor = page.status === "ready" ? after : null;
        this.previousIssueCursors = page.status === "ready" ? history : [];
        this.updateIssues({ page, pageNumber: this.previousIssueCursors.length + 1, canGoBack: this.previousIssueCursors.length > 0, stale: false });
      }
    } catch (error) {
      if (this.issueCurrent(token, this.issuePageRequest)) {
        this.issuePageError = `이슈 목록을 읽지 못했습니다: ${failureMessage(error)}`;
        this.updateIssues({ stale: this.store.getKernel().issues.page !== null, canGoBack: false });
      }
    } finally {
      if (this.issueCurrent(token, this.issuePageRequest)) {
        this.activeIssuePage = null;
        this.updateIssues({ loading: false });
      }
    }
    if (restart && this.issueCurrent(token, this.issuePageRequest)) {
      this.updateIssues({ stale: this.store.getKernel().issues.page !== null, canGoBack: false });
      if (retried) {
        this.issuePageError = "이슈 목록이 조회 중 바뀌었습니다. 첫 페이지를 다시 읽어 주세요.";
        this.updateIssues({});
      } else {
        await this.readIssuesPage(null, [], null, true);
      }
    }
  }

  private async runIssueCommand(command: KernelCommand, pending: "mode" | "sync"): Promise<void> {
    if (!this.canRead("issues") || this.store.getKernel().issues.commandPending !== null) return;
    const token = this.issueToken(++this.issueCommandRequest);
    this.issueCommandError = null;
    this.updateIssues({ commandPending: pending });
    try {
      const response = await sendKernelCommand(this.client, command);
      if (!this.issueCurrent(token, this.issueCommandRequest)) return;
      if (response.rejection) {
        throw new Error(response.rejection.reason === "issue_automation_off"
          ? "이슈 수집이 꺼져 있습니다. 관찰로 바꾼 뒤 다시 동기화하세요."
          : `커널이 요청을 거절했습니다: ${response.rejection.message}`);
      }
      if (response.failure) throw new Error(response.failure);
      if (!response.processed) throw new Error("커널이 요청을 처리하지 못했습니다. 상태를 다시 읽어 주세요.");
      await this.refreshIssues();
    } catch (error) {
      if (this.issueCurrent(token, this.issueCommandRequest)) {
        this.issueCommandError = `이슈 요청을 완료하지 못했습니다: ${failureMessage(error)}`;
        this.updateIssues({});
      }
    } finally {
      if (this.issueCurrent(token, this.issueCommandRequest)) this.updateIssues({ commandPending: null });
    }
  }

  private applyOverview(event: KernelEvent): void {
    const view = this.store.getKernel();
    switch (event.type) {
      case "thread_activity":
        this.store.setKernel({ ...view, threads: event.threads });
        break;
      case "auto_follow":
        this.store.setKernel({ ...view, autoFollow: { max_turns: event.max_turns, paused: event.paused } });
        break;
      case "planning": {
        const { type: _type, ...planning } = event;
        this.store.setKernel({ ...view, planning });
        break;
      }
      case "parallel": {
        const { type: _type, ...parallel } = event;
        this.store.setKernel({ ...view, parallel });
        break;
      }
      default:
        break;
    }
  }
}
