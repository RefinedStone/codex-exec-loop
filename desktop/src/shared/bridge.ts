// main 과 renderer 가 공유하는 메시지 버스 계약이다. main 은 메시지를 해석하지 않고 문자열 그대로 전달한다.
import type { GitHunkAction, GitHunkDiff } from "./gitHunks";

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
  remoteOpen: "akra:remote-open",
  remoteClose: "akra:remote-close",
  remoteSend: "akra:remote-send",
  remoteMessage: "akra:remote-message",
  remoteState: "akra:remote-state",
  chooseFolder: "akra:choose-folder",
  gitStatus: "akra:git-status",
  gitFileDiff: "akra:git-file-diff",
  gitFileAction: "akra:git-file-action",
  gitHunks: "akra:git-hunks",
  gitHunkAction: "akra:git-hunk-action",
  gitOpenFile: "akra:git-open-file",
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
  githubSummary: "akra:github-summary",
  githubReviewPrompt: "akra:github-review-prompt",
  githubProject: "akra:github-project",
  githubIssuePrompt: "akra:github-issue-prompt",
  githubRepository: "akra:github-repository",
  openExternal: "akra:open-external",
  jiraStatus: "akra:jira-status",
  jiraConnect: "akra:jira-connect",
  jiraDisconnect: "akra:jira-disconnect",
  jiraSummary: "akra:jira-summary",
  jiraIssuePrompt: "akra:jira-issue-prompt",
  dotsGetEnabled: "akra:dots-get-enabled",
  dotsSetEnabled: "akra:dots-set-enabled",
  dotsOpenWindow: "akra:dots-open-window",
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
  /** 이슈의 본문과 최근 댓글로 만든 첫 메시지이다. */
  issuePrompt(url: string): Promise<string>;
  /** 폴더의 `origin` 이 가리키는 GitHub `owner/name` 이다. git 저장소가 아니거나 GitHub 가 아니면 null 이다. */
  repository(cwd: string): Promise<string | null>;
  /** github.com 주소를 기본 브라우저로 연다. */
  openExternal(url: string): void;
}

/** cloud: Atlassian Cloud (이메일 + API 토큰). server: Server/Data Center (PAT). */
export type JiraDeployment = "cloud" | "server";

/** 연결 화면에서 main 으로 한 번만 보낸다. main 은 토큰을 renderer 로 돌려주지 않는다. */
export interface JiraConnectRequest {
  siteUrl: string;
  deployment: JiraDeployment;
  email: string;
  token: string;
}

export type JiraStatus = { connected: false } | { connected: true; siteUrl: string; deployment: JiraDeployment; email: string | null };

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

/** Jira 업무 패널이다 (R4). main 이 Jira REST 를 직접 부른다. */
export interface JiraBridge {
  status(): Promise<JiraStatus>;
  connect(request: JiraConnectRequest): Promise<JiraStatus>;
  disconnect(): Promise<JiraStatus>;
  summary(): Promise<JiraSummary>;
  /** 티켓 내용(설명, 최근 댓글)으로 만든 첫 메시지이다. */
  issuePrompt(key: string): Promise<string>;
  /** 연결된 Jira 사이트의 티켓 주소를 기본 브라우저로 연다. */
  openExternal(url: string): void;
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

export type GitFileDiff =
  | { kind: "gitDiff"; text: string; truncated: boolean }
  | { kind: "content"; text: string; truncated: boolean }
  | { kind: "binary" };

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
  /**
   * 바뀐 파일 하나를 스테이지하거나, 스테이지를 취소하거나, 변경을 버린다. 경로만 보낸다. main 이 그 파일의 상태를
   * git 에서 다시 읽고, 버리기 전에 사용자에게 확인한다.
   */
  gitFileAction(cwd: string, action: GitFileAction, path: string): Promise<GitActionResult>;
  gitHunks(cwd: string, path: string): Promise<GitHunkDiff>;
  gitHunkAction(cwd: string, action: GitHunkAction, path: string, hunkId: string): Promise<GitActionResult>;
  /** 파일을 편집기(VS Code)로 연다. 편집기가 없으면 파일이 있는 폴더를 연다. */
  gitOpenFile(cwd: string, path: string): Promise<void>;
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
  github: GitHubBridge;
  jira: JiraBridge;
  notion: NotionBridge;
  dots: DotsBridge;
}

/** dots 클라우드 호스트의 활성화 설정이다 (plan 25 §3.2). Gateway 를 다시 시작해야 적용된다. */
export interface DotsBridge {
  getEnabled(): Promise<boolean>;
  /** 플래그를 바꾸고 새 상태를 돌려준다. Gateway 재시작 전에는 적용되지 않는다. */
  setEnabled(enabled: boolean): Promise<boolean>;
  /** 그 dot 과의 대화만 보이는 작은 창을 연다. 이미 열려 있으면 그 창을 앞으로 가져온다. */
  openWindow(host: string, dotId: string): void;
}

/** dot 창의 주소에 넣는 dot id 이다. 주소와 파일 이름에 안전한 글자만 받는다. */
export function isDotId(dotId: unknown): dotId is string {
  return typeof dotId === "string" && /^[A-Za-z0-9_-]{1,128}$/.test(dotId);
}
