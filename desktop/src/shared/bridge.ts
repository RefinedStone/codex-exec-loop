// main 과 renderer 가 공유하는 메시지 버스 계약이다. main 은 메시지를 해석하지 않고 문자열 그대로 전달한다.
import type { GitHunkAction, GitHunkDiff } from "./gitHunks";
import type { ProviderConnectionBridge, ProviderCredentialBridge } from "./providerCredentials";
import type { BrowserAgentBridge } from "./browserAgent";
import type { AppSettings, AppSettingsPatch } from "./settings";
import type { SkillsBridge } from "./skills";
import type { VaultApprovedMapping, VaultRestoreTarget, VaultRestoreViewState } from "./vaultRestore";
import type { ZoomDirection, ZoomFactors, ZoomScope } from "./zoom";
import type { ConfirmTaskCriterionWaiverRequest, ConfirmIssueBindingResumeRequest, ConfirmIssueAutonomyRequest, ConfirmIssueAutomationModeRequest, ConfirmIssueDecisionRequest, IssueConfirmationResponse } from "./issueConfirmation";
import type { ConfirmIssueCommentRequest } from "./issueConfirmation";
import type { DiagnosticsBridge } from "./diagnostics";

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
  diagnosticsPreview: "akra:diagnostics-preview",
  diagnosticsSave: "akra:diagnostics-save",
  diagnosticsDiscard: "akra:diagnostics-discard",
  remoteOpen: "akra:remote-open",
  remoteClose: "akra:remote-close",
  remoteSend: "akra:remote-send",
  remoteMessage: "akra:remote-message",
  remoteState: "akra:remote-state",
  chooseFolder: "akra:choose-folder",
  openFolder: "akra:open-folder",
  gitStatus: "akra:git-status",
  gitFileDiff: "akra:git-file-diff",
  gitFileAction: "akra:git-file-action",
  gitHunks: "akra:git-hunks",
  gitHunkAction: "akra:git-hunk-action",
  gitOpenFile: "akra:git-open-file",
  storageReveal: "akra:storage-reveal",
  issueAutomationPolicyOpenFile: "issueAutomationPolicy:openFile",
  confirmIssueDecision: "akra:confirm-issue-decision",
  confirmIssueAutomationMode: "akra:confirm-issue-automation-mode",
  confirmIssueAutonomy: "akra:confirm-issue-autonomy",
  confirmIssueBindingResume: "akra:confirm-issue-binding-resume",
  confirmIssueCommentRequest: "akra:confirm-issue-comment-request",
  confirmTaskCriterionWaiver: "akra:confirm-task-criterion-waiver",
  gitBranchSummary: "akra:git-branch-summary",
  gitCommitDiff: "akra:git-commit-diff",
  gitPush: "akra:git-push",
  gitCreatePullRequest: "akra:git-create-pull-request",
  terminalOpen: "akra:terminal-open",
  terminalWrite: "akra:terminal-write",
  terminalResize: "akra:terminal-resize",
  terminalClose: "akra:terminal-close",
  terminalData: "akra:terminal-data",
  terminalExit: "akra:terminal-exit",
  browserShow: "akra:browser-show",
  browserBounds: "akra:browser-bounds",
  browserHide: "akra:browser-hide",
  browserNavigate: "akra:browser-navigate",
  browserCommand: "akra:browser-command",
  browserState: "akra:browser-state",
  browserLoadSettings: "akra:browser-load-settings",
  browserSaveSettings: "akra:browser-save-settings",
  browserAgentPrepare: "akra:browser-agent-prepare",
  browserAgentBind: "akra:browser-agent-bind",
  browserAgentRelease: "akra:browser-agent-release",
  browserAgentActiveThread: "akra:browser-agent-active-thread",
  browserAgentPause: "akra:browser-agent-pause",
  browserAgentResume: "akra:browser-agent-resume",
  browserAgentStatus: "akra:browser-agent-status",
  browserAgentState: "akra:browser-agent-state",
  delegatedChatEnabled: "akra:delegated-chat-enabled",
  delegatedChatOpen: "akra:delegated-chat-open",
  githubSummary: "akra:github-summary",
  githubReviewPrompt: "akra:github-review-prompt",
  githubProject: "akra:github-project",
  githubRepository: "akra:github-repository",
  openExternal: "akra:open-external",
  providerLegacyStatus: "akra:provider-legacy-status",
  providerMigrateLegacy: "akra:provider-migrate-legacy",
  providerPrepareDisconnect: "akra:provider-prepare-disconnect",
  providerCleanupLegacy: "akra:provider-cleanup-legacy",
  providerOpenExternal: "akra:provider-open-external",
  dotsGetEnabled: "akra:dots-get-enabled",
  dotsSetEnabled: "akra:dots-set-enabled",
  dotsOpenWindow: "akra:dots-open-window",
  dotsHasWindow: "akra:dots-has-window",
  dotsDock: "akra:dots-dock",
  dotsDocked: "akra:dots-docked",
  dotsSendAllowed: "akra:dots-send-allowed",
  dotsAllowSend: "akra:dots-allow-send",
  dotsTakeDraft: "akra:dots-take-draft",
  dotsReportSeen: "akra:dots-report-seen",
  dotsSeen: "akra:dots-seen",
  zoomGet: "akra:zoom-get",
  zoomStep: "akra:zoom-step",
  zoomChanged: "akra:zoom-changed",
  zoomCommand: "akra:zoom-command",
  appFocused: "akra:app-focused",
  settingsGet: "akra:settings-get",
  settingsSet: "akra:settings-set",
  settingsChanged: "akra:settings-changed",
  settingsOpen: "akra:settings-open",
  reportWorking: "akra:report-working",
  vaultRestoreOpen: "akra:vault-restore-open",
  vaultRestoreProjectChanged: "akra:vault-restore-project-changed",
  skillsList: "akra:skills-list",
  skillsLink: "akra:skills-link",
  skillsCheck: "akra:skills-check",
  skillsPreview: "akra:skills-preview",
  skillsApply: "akra:skills-apply",
  skillsRollback: "akra:skills-rollback",
} as const;

export interface GitHubPullRequest {
  number: number;
  title: string;
  url: string;
  /** owner/name */
  repository: string;
  isDraft: boolean;
  updatedAt: string;
  /** 리뷰 요청 목록에서만 채운다. */
  author: string | null;
  /** 마지막 커밋의 CI 상태 (SUCCESS, FAILURE, PENDING 등). 없으면 null 이다. */
  checks: string | null;
  reviewDecision: string | null;
  /** 해결되지 않은 리뷰 스레드 수이다. 내 PR 목록에서만 채운다. */
  unresolvedThreads: number | null;
}

export interface GitHubSummary {
  login: string;
  myPullRequests: GitHubPullRequest[];
  reviewRequests: GitHubPullRequest[];
}

/** GitHub Project 의 항목 하나이다 (R11). 초안(draft issue)은 저장소, 번호, 주소가 없다. */
export interface GitHubProjectItem {
  kind: "issue" | "pullRequest" | "draft";
  title: string;
  /** github.com 주소이다. 초안이면 null 이다. */
  url: string | null;
  /** owner/name 이다. 초안이면 null 이다. */
  repository: string | null;
  number: number | null;
  /** Project 의 Status 필드 값이다. 없으면 null 이다. */
  status: string | null;
}

export interface GitHubProject {
  title: string;
  url: string;
  /** 열린 이슈·PR 과 초안이다. Project 에 들어 있는 순서이다. */
  items: GitHubProjectItem[];
  /** 닫혔거나 머지되어 뺀 항목 수이다. */
  closedCount: number;
  /** 읽는 한도를 넘어서 뒤의 항목을 읽지 않았다. */
  truncated: boolean;
}

/** GitHub 업무 패널이다 (R4). main 이 공식 CLI `gh` 로 읽는다. */
export interface GitHubBridge {
  summary(): Promise<GitHubSummary>;
  /** PR 의 해결되지 않은 리뷰 코멘트로 만든 첫 메시지이다. */
  reviewPrompt(url: string): Promise<string>;
  /** Project 하나의 항목을 읽는다 (R11). `project` 는 `owner/번호` 또는 Project 주소이다. */
  project(project: string): Promise<GitHubProject>;
  /** 폴더의 `origin` 이 가리키는 GitHub `owner/name` 이다. git 저장소가 아니거나 GitHub 가 아니면 null 이다. */
  repository(cwd: string): Promise<string | null>;
  /** github.com 주소를 기본 브라우저로 연다. */
  openExternal(url: string): void;
}

/** cloud: Atlassian Cloud (이메일 + API 토큰). server: Server/Data Center (PAT). */
export type JiraDeployment = "cloud" | "server";

export interface JiraIssue {
  key: string;
  summary: string;
  /** `<사이트>/browse/<키>` */
  url: string;
  status: string | null;
  /** new, indeterminate, done */
  statusCategory: string | null;
  issueType: string | null;
  priority: string | null;
  updatedAt: string;
}

export interface JiraSummary {
  siteUrl: string;
  /** 나에게 배정된, 끝나지 않은 티켓이다. 최근 수정 순이다. */
  issues: JiraIssue[];
}

/** Jira 업무 패널의 renderer facade이다. 인증과 HTTP는 Gateway가 소유한다. */
export interface JiraDraftCoordinate { readonly connection:import("./providerCredentials").ConnectionRef; readonly cloudId:string; readonly projectId:string; readonly issueId:string }
export interface JiraBridge extends ProviderConnectionBridge {
  draftTarget(key:string): Promise<JiraDraftCoordinate>;
  currentVersion(request: { readonly key: string; readonly url: string }): Promise<{ version: string }>;
  summary(): Promise<JiraSummary>;
  issuePrompt(key: string): Promise<string>;
  openExternal(url: string): Promise<void>;
}

/** Notion 업무 패널이다 (plan 21). 데이터는 renderer 가 Gateway 의 MCP hub 로 읽는다. */
export interface NotionBridge {
  /** Notion 페이지 주소를 기본 브라우저로 연다. */
  openExternal(url: string): void;
}

/** 브라우저 보기를 놓을 자리이다. 창 안의 CSS 픽셀이다. */
export interface BrowserBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface BrowserState {
  profileId: string;
  url: string;
  title: string;
  loading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
  /** 마지막 페이지 열기 실패이다. */
  error: string | null;
}

/** 프로필마다 쿠키, localStorage, 캐시가 따로 있다. */
export interface BrowserProfile {
  id: string;
  name: string;
}

/** 자주 여는 사이트와 프로필의 조합이다. */
export interface BrowserPreset {
  name: string;
  url: string;
  profileId: string;
}

export interface BrowserSettings {
  profiles: BrowserProfile[];
  presets: BrowserPreset[];
}

export type BrowserCommand = "back" | "forward" | "reload" | "stop";

/** 앱 창 안의 브라우저 보기이다 (R3). */
export interface BrowserBridge {
  readonly agent: BrowserAgentBridge;
  show(profileId: string, bounds: BrowserBounds, url: string | null): Promise<BrowserState>;
  setBounds(bounds: BrowserBounds): void;
  hide(): void;
  /** 열 수 없는 주소이면 false 이다. */
  navigate(input: string): Promise<boolean>;
  command(command: BrowserCommand): void;
  onState(listener: (state: BrowserState) => void): () => void;
  loadSettings(): Promise<BrowserSettings>;
  saveSettings(settings: BrowserSettings): Promise<BrowserSettings>;
}

export interface TerminalOpenRequest {
  /** 터미널의 작업 폴더이다. 없거나 지워졌으면 홈 폴더이다. */
  cwd: string | null;
  cols: number;
  rows: number;
}

export type TerminalOpenResult = { ok: true; id: number; cwd: string; shell: string } | { ok: false; message: string };

/** 앱 안 터미널이다 (R2). 대화 메시지 버스와 다른 IPC 통로를 쓴다. */
export interface TerminalBridge {
  open(request: TerminalOpenRequest): Promise<TerminalOpenResult>;
  write(id: number, data: string): void;
  resize(id: number, cols: number, rows: number): void;
  close(id: number): void;
  onData(listener: (id: number, data: string) => void): () => void;
  onExit(listener: (id: number, exitCode: number) => void): () => void;
}

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

/** 파일 하나에 하는 git 동작이다. `discard` 는 변경을 버린다 (되돌릴 수 없다). */
export type GitFileAction = "stage" | "unstage" | "discard";

/** `cancelled` 는 사용자가 확인 창에서 취소했다는 뜻이다. */
export type GitActionResult = { ok: true } | { ok: false; message: string; cancelled?: boolean };

export type GitDiffSource = "head_worktree" | "head_index" | "index_worktree" | "untracked";

/** Identifies the captured view, not a promise that the worktree is still unchanged. */
export interface GitDiffSnapshot {
  source: GitDiffSource;
  snapshotId: string;
  headOid: string | null;
  path: string;
  originalPath: string | null;
  contentStatus: "patch_only" | "complete" | "truncated" | "binary";
}

export type GitFileDiff = (
  | { kind: "gitDiff"; text: string; truncated: boolean }
  | { kind: "content"; text: string; truncated: boolean }
  | { kind: "binary" }
) & { snapshot?: GitDiffSnapshot };

/** 브랜치가 기준 브랜치 뒤에 더한 커밋 하나이다. */
export interface GitCommit {
  sha: string;
  shortSha: string;
  subject: string;
  author: string;
  /** Unix 초이다. */
  committedAt: number;
  /** 원격 브랜치에 있다. 원격 브랜치가 없으면 false 이다. 알 수 없으면 null 이다 (머지한 뒤 지운 브랜치, 브랜치 없이 보는 커밋). */
  pushed: boolean | null;
}

export type GitChecksState = "SUCCESS" | "FAILURE" | "PENDING";

/** 현재 브랜치의 GitHub PR 이다. 같은 브랜치의 PR 이 여럿이면 가장 최근 것이다. */
export interface GitPullRequest {
  number: number;
  title: string;
  url: string;
  state: "OPEN" | "MERGED" | "CLOSED";
  isDraft: boolean;
  /** PR 의 대상 브랜치이다. */
  baseBranch: string;
  /** 검사가 없으면 null 이다. */
  checks: GitChecksState | null;
}

export type GitBranchSummary =
  | {
      ok: true;
      /** 현재 브랜치이다. 브랜치 없이 커밋을 보고 있으면 null 이다. */
      branch: string | null;
      /** 이 브랜치를 push 한 원격 브랜치이다 (예: `origin/feat/x`). 없으면 null 이다. */
      remoteBranch: string | null;
      /** `origin` 의 GitHub `owner/name` 이다. GitHub 가 아니면 null 이다. */
      repository: string | null;
      /** 커밋을 비교한 기준이다 (예: `origin/main`). 찾지 못했으면 null 이고 커밋 목록은 비어 있다. */
      base: string | null;
      /** 최근 것부터이다. */
      commits: GitCommit[];
      truncated: boolean;
      pullRequest: GitPullRequest | null;
      /** PR 을 읽지 못한 이유이다 (gh 없음, 로그인 안 함 등). */
      pullRequestError: string | null;
    }
  | { ok: false; message: string };

/** 사용자가 쓴 새 PR 의 내용이다. */
export interface PullRequestDraft {
  /** 한 줄이다. */
  title: string;
  body: string;
  /** 대상 브랜치의 이름이다 (`origin/` 없이). */
  base: string;
  draft: boolean;
}

/** push 와 PR 만들기의 결과이다. `url` 은 만든 PR 의 주소이다. `cancelled` 는 사용자가 확인 창에서 취소했다는 뜻이다. */
export type GitPublishResult = { ok: true; url: string | null } | { ok: false; message: string; cancelled?: boolean };

export interface GitCommitDiff {
  /** git diff 형식이다. */
  text: string;
  truncated: boolean;
}

/** Gateway 의 호스트 id 이다. `local` 은 원격 호스트가 아니다. */
export function isRemoteHostId(host: unknown): host is string {
  return typeof host === "string" && host !== "local" && /^[A-Za-z0-9_-]{1,64}$/.test(host);
}

/**
 * `local` 이 아닌 Gateway 호스트(클라우드 에이전트, plan 25)의 접속이다. 호스트마다 접속이 하나이고, 메시지와 상태에
 * 호스트 id 가 붙는다. main 은 `open` 을 받기 전에는 접속하지 않는다.
 */
export interface RemoteHostBridge {
  open(host: string): void;
  close(host: string): void;
  send(host: string, text: string): void;
  onMessage(listener: (host: string, text: string) => void): () => void;
  onConnectionState(listener: (host: string, state: ConnectionState) => void): () => void;
}

/** preload 가 `window.akra` 로 노출하는 함수 전부이다. */

/** 서버가 반환한 정책 위치는 일치 확인에만 쓴다. 실제 대상은 main이 scope별로 다시 결정한다. */
export interface IssueAutomationPolicyFileRequest {
  workspace: string;
  scope: "user" | "project";
  expectedPath: string;
  host: { kind: "local" } | { kind: "remote"; id: string };
}

export interface AkraBridge {
  readonly diagnostics: DiagnosticsBridge;
  revealStoragePath(request: { path: string; host: "local" }): Promise<void>;
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
  /** 있는 폴더를 운영체제의 파일 관리자로 연다. 열지 못하면 false 이다 (#2841). */
  openFolder(path: string): Promise<boolean>;
  /** 작업 폴더의 바뀐 파일 목록을 읽는다 (R1). */
  gitStatus(cwd: string): Promise<GitStatusResult>;
  /** 바뀐 파일 하나의 diff 를 읽는다. 새 파일은 내용을 돌려준다. */
  gitFileDiff(cwd: string, path: string, untracked: boolean, source?: GitDiffSource): Promise<GitFileDiff>;
  /**
   * 바뀐 파일 하나를 스테이지하거나, 스테이지를 취소하거나, 변경을 버린다. 경로만 보낸다. main 이 그 파일의 상태를
   * git 에서 다시 읽고, 버리기 전에 사용자에게 확인한다.
   */
  gitFileAction(cwd: string, action: GitFileAction, path: string): Promise<GitActionResult>;
  gitHunks(cwd: string, path: string): Promise<GitHunkDiff>;
  gitHunkAction(cwd: string, action: GitHunkAction, path: string, hunkId: string): Promise<GitActionResult>;
  /** 파일을 편집기(VS Code)로 연다. 편집기가 없으면 파일이 있는 폴더를 연다. */
  gitOpenFile(cwd: string, path: string): Promise<void>;
  /** 로컬의 알려진 자동화 정책 파일을 편집기로 연다. 누락된 파일을 여기서 만들지 않는다. */
  openIssueAutomationPolicyFile(request: IssueAutomationPolicyFileRequest): Promise<void>;
  /** main의 별도 시스템 확인 창을 거친다. 확인 증거는 반환하지 않는다. */
  confirmIssueDecision(request: ConfirmIssueDecisionRequest): Promise<IssueConfirmationResponse>;
  confirmIssueAutomationMode(request: ConfirmIssueAutomationModeRequest): Promise<IssueConfirmationResponse>;
  confirmIssueAutonomy(request: ConfirmIssueAutonomyRequest): Promise<IssueConfirmationResponse>;
  confirmIssueBindingResume(request: ConfirmIssueBindingResumeRequest): Promise<IssueConfirmationResponse>;
  confirmIssueCommentRequest(request: ConfirmIssueCommentRequest): Promise<IssueConfirmationResponse>;
  confirmTaskCriterionWaiver(request: ConfirmTaskCriterionWaiverRequest): Promise<IssueConfirmationResponse>;
  /** 브랜치가 기준 브랜치 뒤에 더한 커밋과 그 브랜치의 PR 을 읽는다. `fresh` 이면 잠깐 저장해 둔 PR 을 쓰지 않고 다시 묻는다. */
  gitBranchSummary(cwd: string, fresh: boolean): Promise<GitBranchSummary>;
  /** 커밋 하나의 변경을 읽는다. */
  gitCommitDiff(cwd: string, sha: string): Promise<GitCommitDiff>;
  /** 현재 브랜치를 `origin` 의 같은 이름 브랜치로 push 한다. main 이 먼저 사용자에게 확인한다. 강제 push 는 하지 않는다. */
  gitPush(cwd: string): Promise<GitPublishResult>;
  /** 현재 브랜치의 PR 을 gh 로 만든다. main 이 저장소, 브랜치, gh 계정을 보여 주고 사용자에게 확인한다. */
  gitCreatePullRequest(cwd: string, draft: PullRequestDraft): Promise<GitPublishResult>;
  remote: RemoteHostBridge;
  terminal: TerminalBridge;
  browser: BrowserBridge;
  delegatedChat: {
    enabled(): Promise<boolean>;
    /** Opens the isolated ChatGPT window for manual use. */
    open(conversationUrl?: string): Promise<boolean>;
  };
  github: GitHubBridge;
  providerCredentials: ProviderCredentialBridge;
  notion: NotionBridge;
  dots: DotsBridge;
  zoom: ZoomBridge;
  /** 이 앱의 창 가운데 하나에 포커스가 있는지 묻는다. 브라우저 패널에 포커스가 있어도 true 이다. */
  isAppFocused(): Promise<boolean>;
  settings: SettingsBridge;
  skills: SkillsBridge;
  /** 로컬 복원본 창을 연다 (#2427). 복원한 글은 이 버스로 오지 않는다. */
  vaultRestore: VaultRestoreOpenerBridge;
}

/** 앱 설정이다 (plan 26 U2). 값은 main 이 가지고 저장한다. */
export interface SettingsBridge {
  get(): Promise<AppSettings>;
  /** 설정의 한 부분을 바꾼다. main 이 지금 값에 합치고, 검증하고, 저장한 값을 돌려준다. */
  set(patch: AppSettingsPatch): Promise<AppSettings>;
  /** 설정이 바뀌면 부른다. 다른 창에서 바꾼 것도 온다. */
  onChanged(listener: (settings: AppSettings) => void): () => void;
  /** 앱 메뉴의 "설정…" 을 고르면 부른다. */
  onOpen(listener: () => void): () => void;
  /** 이 창에 실행 중인 턴이 있는지 알린다. main 이 "일할 때만 잠자기 방지"에 쓴다. */
  reportWorking(working: boolean): void;
}

/** 영역별 확대이다 (#2626). 값은 main 이 가지고 저장한다. renderer 가 영역마다 CSS `zoom` 으로 적용한다. */
export interface ZoomBridge {
  get(): Promise<ZoomFactors>;
  /** 한 영역을 한 단계 확대하거나 축소하거나 100% 로 되돌린다. main 이 영역 이름을 검증한다. */
  step(scope: ZoomScope, direction: ZoomDirection): void;
  /** 확대 값이 바뀌면 부른다. 다른 창에서 바꾼 것도 온다. `scope` 는 바뀐 영역, `here` 는 이 창에서 바꿨는지이다. */
  onChanged(listener: (factors: ZoomFactors, scope: ZoomScope, here: boolean) => void): () => void;
  /** 메뉴나 단축키(⌘+ / ⌘− / ⌘0)이다. 어느 영역을 바꿀지는 이 창이 포커스로 정한다. */
  onCommand(listener: (direction: ZoomDirection) => void): () => void;
}

/** dots 클라우드 호스트의 활성화 설정이다 (plan 25 §3.2). Gateway 를 다시 시작해야 적용된다. */
export interface DotsBridge {
  getEnabled(): Promise<boolean>;
  /** 플래그를 바꾸고 새 상태를 돌려준다. Gateway 재시작 전에는 적용되지 않는다. */
  setEnabled(enabled: boolean): Promise<boolean>;
  /**
   * 그 dot 과의 대화만 보이는 작은 창을 연다. 이미 열려 있으면 그 창을 앞으로 가져온다. `draft` 는 옆 칸에서 쓰던
   * 글이다: 새로 여는 창이 `takeDraft` 로 받는다. 답은 창을 새로 열었는지이다. 이미 있던 창은 `draft` 를 받지
   * 않으므로 false 이다: 부른 쪽이 자기 글을 지우면 안 된다.
   */
  openWindow(host: string, dotId: string, draft?: string): Promise<boolean>;
  /** dot 창이 부른다: 이 창을 열 때 넘겨받은 쓰던 글을 한 번 가져간다. 없으면 null 이다. */
  takeDraft(host: string, dotId: string): Promise<string | null>;
  /** 그 dot 의 창이 열려 있는지이다. */
  hasWindow(host: string, dotId: string): Promise<boolean>;
  /** dot 창이 부른다: 대화와 쓰던 글을 그 창을 연 본 창의 옆 칸으로 옮기고 dot 창을 닫는다. */
  dock(host: string, dotId: string, draft: string): void;
  /** 본 창이 듣는다: dot 창이 대화를 옆 칸으로 옮겼다. `draft` 는 그 창에서 쓰던 글이다. */
  onDock(listener: (host: string, dotId: string, draft: string) => void): () => void;
  /** dot 창이 부른다: 사용자가 이 창에서 그 dot 의 새 답을 봤다. 다른 창들이 안 읽음 표시를 지운다. */
  reportSeen(host: string, dotId: string): void;
  /** 본 창이 듣는다: 사용자가 dot 창에서 그 dot 의 새 답을 봤다. */
  onSeen(listener: (host: string, dotId: string) => void): () => void;
  /** 이 앱을 켠 동안 사용자가 그 dot 에 보내기를 허락했는지이다. 본 창과 dot 창이 같은 답을 받는다. */
  isSendAllowed(dotId: string): Promise<boolean>;
  /** 사용자가 그 dot 에 보내기를 허락했다고 적는다. 앱을 끄면 사라진다. */
  allowSend(dotId: string): void;
}

/** dot 창의 주소에 넣는 dot id 이다. 주소와 파일 이름에 안전한 글자만 받는다. */
export function isDotId(dotId: unknown): dotId is string {
  return typeof dotId === "string" && /^[A-Za-z0-9_-]{1,128}$/.test(dotId);
}


/** 복원본 창의 window.akra.vaultRestore 에만 허용하는 함수이다. 일반 창에는 이 권한이 없다. */
export interface VaultRestoreBridge {
  previewBackup(generation: number, archive: Uint8Array, recoveryKey: string): Promise<import("./vaultRestore").VaultBackupPreview>;
  /** 수신 함수를 등록했다고 알린다. 처음이면 main 이 확인을 시작한다. 다시 오면 (화면을 다시 불러옴) 잠근다. */
  ready(): void;
  onState(listener: (state: VaultRestoreViewState, viewGeneration: number, backupAvailable: boolean) => void): () => void;
  /** 현재 화면의 세대에 묶어 다시 사용자 확인을 거쳐 연다. */
  unlock(viewGeneration: number): void;
  lock(): void;
  /** 현재 화면의 세대에 묶어 재연결 검토의 값을 읽는다. */
  review(viewGeneration: number): void;
  /** 이전 화면에서 지연된 승인을 새 검토에 적용하지 않는다. */
  reconnect(viewGeneration: number, mappings: VaultApprovedMapping[]): void;
}

/** 일반 창이 쓰는 함수이다. 창을 열고 닫게 할 뿐이다. 복원본의 상태와 글은 받지 못한다. */
export interface VaultRestoreOpenerBridge {
  /** 이슈의 로컬 복원본 창을 연다. 열람은 그 창에서 사용자가 확인한 뒤에만 된다. */
  open(target: VaultRestoreTarget): Promise<void>;
  /** 이 창의 프로젝트(열린 스레드의 폴더)가 바뀌었다. 이 창이 연 복원본을 다시 잠근다. */
  projectChanged(): void;
}


/** 복원 전용 창의 window.akra 계약. 일반 창의 Gateway, 브라우저, 파일 접근 권한을 포함하지 않는다. */
export interface AkraVaultRestoreBridge {
  readonly vaultRestore: VaultRestoreBridge;
}
