// 블루프린트의 mock 데이터이다. Gateway, codex, 외부 서비스에서 온 값이 아니다. 모두 만든 값이다.

export type ThreadStatus = "running" | "approval" | "unread" | "failed" | "idle";

export interface MockProject {
  id: string;
  name: string;
  path: string;
}

export interface MockThread {
  id: string;
  projectId: string;
  title: string;
  status: ThreadStatus;
  updated: string;
  branch: string;
  /** worktree 에서 실행하면 true 이다. false 이면 작업 폴더에서 바로 실행한다. */
  worktree: boolean;
  added: number;
  removed: number;
  pullRequest: { number: number; checks: "pass" | "fail" | "pending" } | null;
  pinned: boolean;
  /** 끝난 일로 정리한 스레드이다. 사이드바 목록에서 빠진다. */
  done: boolean;
  /** 미룬 스레드가 목록에 돌아오는 때이다. null 이면 미루지 않았다. */
  snoozedUntil: string | null;
}

export type StepKind = "read" | "search" | "edit" | "command" | "tool";

export interface MockStep {
  id: string;
  kind: StepKind;
  label: string;
  status: "running" | "ok" | "failed";
  duration: string | null;
  output: string | null;
}

export interface PlanStep {
  text: string;
  state: "done" | "active" | "todo";
}

export interface ChangedFile {
  path: string;
  added: number;
  removed: number;
}

export type MockItem =
  | { type: "reasoning"; id: string; seconds: number; text: string }
  | { type: "activity"; id: string; steps: MockStep[] }
  | { type: "plan"; id: string; steps: PlanStep[] }
  | { type: "agent"; id: string; text: string }
  | { type: "changes"; id: string; files: ChangedFile[] };

export interface MockTurn {
  id: string;
  user: string;
  attachments: string[];
  items: MockItem[];
  status: "running" | "done" | "interrupted";
  /** 턴이 끝난 뒤의 한 줄 요약이다 (걸린 시간, 토큰). */
  summary: string | null;
}

export interface MockApproval {
  command: string;
  cwd: string;
  reason: string;
}

export const PROJECTS: MockProject[] = [
  { id: "akra", name: "akra", path: "~/dev/akra" },
  { id: "example-app", name: "example-app", path: "~/dev/example-app" },
  { id: "docs-site", name: "docs-site", path: "~/dev/docs-site" },
];

export const THREADS: MockThread[] = [
  { id: "t1", projectId: "akra", title: "사이드바에 스레드 상태 표시", status: "idle", updated: "방금", branch: "feat/sidebar-status", worktree: true, added: 182, removed: 41, pullRequest: null, pinned: true, done: false, snoozedUntil: null },
  { id: "t2", projectId: "akra", title: "Gateway 재접속 간격 수정", status: "approval", updated: "3분", branch: "fix/reconnect-backoff", worktree: true, added: 24, removed: 9, pullRequest: null, pinned: false, done: false, snoozedUntil: null },
  { id: "t3", projectId: "akra", title: "kernel simulation 시험 실패 조사", status: "running", updated: "12분", branch: "fix/kernel-simulation", worktree: true, added: 6, removed: 2, pullRequest: null, pinned: false, done: false, snoozedUntil: null },
  { id: "t4", projectId: "akra", title: "plan 19 문서 정리", status: "unread", updated: "1시간", branch: "docs/plan-19", worktree: true, added: 96, removed: 120, pullRequest: { number: 2231, checks: "pass" }, pinned: false, done: false, snoozedUntil: null },
  { id: "t5", projectId: "example-app", title: "로그인 폼 검증 추가", status: "failed", updated: "2시간", branch: "feat/login-validation", worktree: true, added: 58, removed: 4, pullRequest: { number: 412, checks: "fail" }, pinned: false, done: false, snoozedUntil: null },
  { id: "t6", projectId: "example-app", title: "의존성 업데이트", status: "idle", updated: "3일", branch: "main", worktree: false, added: 0, removed: 0, pullRequest: null, pinned: false, done: false, snoozedUntil: "월요일 오전 9시" },
  { id: "t7", projectId: "docs-site", title: "검색 색인 빌드 스크립트", status: "idle", updated: "5일", branch: "main", worktree: false, added: 0, removed: 0, pullRequest: null, pinned: false, done: true, snoozedUntil: null },
];

const T1_TURNS: MockTurn[] = [
  {
    id: "t1-turn-1",
    user: "사이드바 스레드 줄에 상태(진행 중, 승인 대기, 읽지 않음, 실패)를 표시하고 싶어. 먼저 지금 구조를 보고 계획을 세워줘.",
    attachments: [],
    status: "done",
    summary: "48초 · 6.1k 토큰",
    items: [
      { type: "reasoning", id: "r1", seconds: 4, text: "사이드바는 `desktopStore` 의 `folders`, `activeThreadIds`, `pendingApprovals` 를 읽는다. 읽지 않음과 실패 상태는 아직 저장소에 없다." },
      {
        type: "activity",
        id: "a1",
        steps: [
          { id: "s1", kind: "read", label: "src/renderer/sidebar/Sidebar.tsx", status: "ok", duration: null, output: null },
          { id: "s2", kind: "read", label: "src/renderer/session/desktopStore.ts", status: "ok", duration: null, output: null },
          { id: "s3", kind: "search", label: "activeThreadIds", status: "ok", duration: null, output: "src/renderer/session/desktopStore.ts:61\nsrc/renderer/session/desktopSession.ts:214\nsrc/renderer/sidebar/Sidebar.tsx:29" },
          { id: "s4", kind: "read", label: "src/renderer/session/desktopSession.ts", status: "ok", duration: null, output: null },
        ],
      },
      {
        type: "plan",
        id: "p1",
        steps: [
          { text: "저장소에 스레드별 상태 값을 추가한다", state: "done" },
          { text: "사이드바 줄에 상태 표시를 그린다", state: "done" },
          { text: "스레드를 열면 읽지 않음 상태를 지운다", state: "done" },
          { text: "단위 시험과 화면 시험을 추가한다", state: "done" },
        ],
      },
      {
        type: "agent",
        id: "m1",
        text: "지금 구조에서 상태는 두 가지만 있습니다.\n\n| 상태 | 출처 | 표시 |\n| --- | --- | --- |\n| 진행 중 | `activeThreadIds` | 점 |\n| 승인 대기 | `pendingApprovals` | `승인` 배지 |\n\n**읽지 않음**과 **실패**는 저장소에 값이 없습니다. 위 계획의 순서로 추가하겠습니다. 진행할까요?",
      },
    ],
  },
  {
    id: "t1-turn-2",
    user: "좋아, 진행해.",
    attachments: ["sidebar-sketch.png"],
    status: "done",
    summary: "2분 14초 · 18.2k 토큰",
    items: [
      {
        type: "activity",
        id: "a2",
        steps: [
          { id: "s5", kind: "edit", label: "src/renderer/session/desktopStore.ts", status: "ok", duration: null, output: null },
          { id: "s6", kind: "edit", label: "src/renderer/sidebar/Sidebar.tsx", status: "ok", duration: null, output: null },
          { id: "s7", kind: "edit", label: "src/renderer/styles.css", status: "ok", duration: null, output: null },
          { id: "s8", kind: "command", label: "npm run check", status: "ok", duration: "9.4초", output: "> tsc --noEmit -p tsconfig.node.json && tsc --noEmit -p tsconfig.web.json" },
          {
            id: "s9",
            kind: "command",
            label: "npx vitest run",
            status: "failed",
            duration: "6.1초",
            output: " FAIL  test/sidebarStatus.test.ts > opening a thread clears unread\n  AssertionError: expected 'unread' to be 'idle'\n\n Test Files  1 failed | 17 passed (18)",
          },
          { id: "s10", kind: "edit", label: "src/renderer/session/desktopStore.ts", status: "ok", duration: null, output: null },
          { id: "s11", kind: "command", label: "npx vitest run", status: "ok", duration: "5.8초", output: " Test Files  18 passed (18)\n      Tests  131 passed (131)" },
        ],
      },
      {
        type: "changes",
        id: "c1",
        files: [
          { path: "src/renderer/session/desktopStore.ts", added: 64, removed: 12 },
          { path: "src/renderer/sidebar/Sidebar.tsx", added: 71, removed: 23 },
          { path: "src/renderer/styles.css", added: 47, removed: 6 },
        ],
      },
      {
        type: "agent",
        id: "m2",
        text: "상태 표시를 추가했습니다.\n\n- `desktopStore` 에 `threadStatus` 를 추가했습니다. 값은 `running`, `approval`, `unread`, `failed`, `idle` 입니다.\n- 사이드바 줄은 상태마다 다른 표시를 그립니다.\n- 스레드를 열면 `unread` 는 `idle` 이 됩니다. 첫 시험 실행에서 이 동작이 실패했고, `setOpenThread` 를 수정한 뒤 통과했습니다.\n\n`npm run check` 와 `npx vitest run` 은 통과했습니다. 화면 시험(`npm run e2e`)은 아직 실행하지 않았습니다.",
      },
    ],
  },
];

const T2_TURNS: MockTurn[] = [
  {
    id: "t2-turn-1",
    user: "Gateway 가 끊긴 뒤 재접속 간격이 너무 빨라. 지수 간격으로 바꾸고 push 해줘.",
    attachments: [],
    status: "running",
    summary: null,
    items: [
      {
        type: "activity",
        id: "a3",
        steps: [
          { id: "s12", kind: "read", label: "src/main/gatewayConnection.ts", status: "ok", duration: null, output: null },
          { id: "s13", kind: "edit", label: "src/main/gatewayConnection.ts", status: "ok", duration: null, output: null },
          { id: "s14", kind: "command", label: "npx vitest run test/gatewaySession.test.ts", status: "ok", duration: "2.2초", output: " Test Files  1 passed (1)" },
        ],
      },
      { type: "changes", id: "c2", files: [{ path: "src/main/gatewayConnection.ts", added: 24, removed: 9 }] },
    ],
  },
];

const T3_TURNS: MockTurn[] = [
  {
    id: "t3-turn-1",
    user: "kernel simulation 시험이 가끔 실패해. 원인을 찾아줘.",
    attachments: [],
    status: "running",
    summary: null,
    items: [
      { type: "reasoning", id: "r3", seconds: 7, text: "실패는 순서에 의존한다. 기록 순번과 마감 시간 처리를 먼저 본다." },
      {
        type: "activity",
        id: "a4",
        steps: [
          { id: "s15", kind: "read", label: "src/composition/kernel_host/simulation.rs", status: "ok", duration: null, output: null },
          { id: "s16", kind: "command", label: "cargo test --locked kernel_host::simulation -- --test-threads=1", status: "running", duration: null, output: null },
        ],
      },
    ],
  },
];

const TURNS: Record<string, MockTurn[]> = { t1: T1_TURNS, t2: T2_TURNS, t3: T3_TURNS };

/** 대화 mock 이 없는 스레드는 제목으로 짧은 대화를 만든다. */
export function initialTurns(thread: MockThread): MockTurn[] {
  return (
    TURNS[thread.id] ?? [
      {
        id: `${thread.id}-turn-1`,
        user: `${thread.title} 작업을 해줘.`,
        attachments: [],
        status: "done",
        summary: "1분 2초 · 9.4k 토큰",
        items: [
          {
            type: "agent",
            id: `${thread.id}-m1`,
            text:
              thread.status === "failed"
                ? "작업을 끝내지 못했습니다. PR 의 CI 검사가 실패했습니다. 실패한 검사의 기록을 읽고 수정할 수 있습니다."
                : "작업을 끝냈습니다. 변경은 `변경` 패널에서 확인할 수 있습니다.",
          },
        ],
      },
    ]
  );
}

export const T2_APPROVAL: MockApproval = {
  command: "git push origin fix/reconnect-backoff",
  cwd: "~/dev/akra-worktrees/reconnect-backoff",
  reason: "네트워크에 쓰는 명령입니다. 현재 권한(승인 후 실행)에서는 허용이 필요합니다.",
};

export const MOCK_DIFF = `diff --git a/src/renderer/session/desktopStore.ts b/src/renderer/session/desktopStore.ts
--- a/src/renderer/session/desktopStore.ts
+++ b/src/renderer/session/desktopStore.ts
@@ -18,6 +18,9 @@ export interface WorkspaceThread {
   id: string;
   title: string;
   updatedAt: number;
+  /** 사이드바 표시에 쓰는 상태이다. */
+  status: ThreadStatus;
 }

+export type ThreadStatus = "running" | "approval" | "unread" | "failed" | "idle";
+
 /** 안정 API 에는 \`project/list\` 가 없어서 스레드의 cwd 로 작업 폴더를 묶는다 (D-16). */
@@ -112,7 +115,10 @@ export const useDesktopStore = create<DesktopState>((set) => ({
-  setOpenThread: (openThreadId) => set({ openThreadId, transcript: null }),
+  setOpenThread: (openThreadId) =>
+    set((state) => ({
+      openThreadId,
+      transcript: null,
+      folders: clearUnread(state.folders, openThreadId),
+    })),
diff --git a/src/renderer/sidebar/Sidebar.tsx b/src/renderer/sidebar/Sidebar.tsx
--- a/src/renderer/sidebar/Sidebar.tsx
+++ b/src/renderer/sidebar/Sidebar.tsx
@@ -71,12 +71,9 @@ export function Sidebar({
                     onClick={() => onOpenThread(thread.id)}
                     data-testid="thread-entry"
                   >
-                    {activeThreadIds.includes(thread.id) && <span className="active-dot" aria-label="진행 중" />}
+                    <ThreadStatusMark status={thread.status} />
                     <span className="thread-title">{thread.title}</span>
-                    {waitingForApproval.has(thread.id) && (
-                      <span className="approval-badge" data-testid="approval-badge">
-                        승인
-                      </span>
-                    )}
+                    <span className="thread-age">{relativeAge(thread.updatedAt)}</span>
                   </button>
diff --git a/src/renderer/styles.css b/src/renderer/styles.css
--- a/src/renderer/styles.css
+++ b/src/renderer/styles.css
@@ -80,6 +80,14 @@
   white-space: nowrap;
 }

+.thread-status-running {
+  animation: spin 900ms linear infinite;
+}
+
+.thread-status-unread {
+  background: var(--accent);
+}
+
 .active-dot {
   flex: none;
   width: 7px;
`;

export const MODELS = ["GPT-5 Codex", "GPT-5 Codex Mini"] as const;
export const EFFORTS = ["낮음", "보통", "높음"] as const;
export const PERMISSIONS = ["읽기 전용", "승인 후 실행", "자동 실행"] as const;

export const SLASH_COMMANDS = [
  { name: "/plan", hint: "실행 전에 계획을 먼저 세운다" },
  { name: "/review", hint: "현재 변경을 검토한다" },
  { name: "/compact", hint: "대화를 요약해 컨텍스트를 줄인다" },
  { name: "/status", hint: "세션과 사용량 상태를 본다" },
] as const;

export const MENTION_FILES = [
  "src/renderer/sidebar/Sidebar.tsx",
  "src/renderer/session/desktopStore.ts",
  "src/renderer/styles.css",
  "docs/plan/17-akra-desktop/README.md",
] as const;

export const SUGGESTIONS = [
  { icon: "pullRequest", title: "PR #2231 리뷰 댓글 3개 반영", source: "GitHub" },
  { icon: "briefcase", title: "PROJ-142 로그인 오류 조사", source: "Jira" },
  { icon: "alert", title: "실패한 CI 검사 원인 찾기", source: "example-app #412" },
] as const;

export const KERNEL = {
  stages: ["계획", "실행", "검토", "통합", "정리"],
  activeStage: 1,
  decision: "실행 단계입니다. 슬롯 2개가 작업 중이고 1개는 승인을 기다립니다.",
  slots: [
    { name: "slot-1", threadId: "t3", state: "실행 중" },
    { name: "slot-2", threadId: "t2", state: "승인 대기" },
    { name: "slot-3", threadId: null, state: "비어 있음" },
  ],
  offers: [
    { id: "o1", title: "example-app #412 의 CI 실패를 조사할까요?", detail: "검사 `test` 가 2시간 전에 실패했습니다. 새 스레드를 worktree 에서 시작합니다." },
    { id: "o2", title: "plan 19 문서 PR 을 머지할까요?", detail: "PR #2231 의 CI Gate 가 통과했고 해결하지 않은 리뷰가 없습니다." },
  ],
  records: [
    { seq: 418, text: "slot-2 가 승인을 요청함 (git push)" },
    { seq: 417, text: "slot-1 턴 시작" },
    { seq: 416, text: "계획 단계 완료, 실행 단계로 이동" },
    { seq: 415, text: "작업 2개를 슬롯에 배정" },
  ],
} as const;

export const WORK_ITEMS = {
  GitHub: [
    { key: "#2231", title: "docs(plan): plan 19 문서 정리", meta: "CI 통과 · 리뷰 댓글 3" },
    { key: "#412", title: "feat: 로그인 폼 검증 추가", meta: "CI 실패" },
  ],
  Jira: [
    { key: "PROJ-142", title: "로그인 오류 조사", meta: "진행 중 · 높음" },
    { key: "PROJ-137", title: "설정 화면 문구 수정", meta: "할 일 · 보통" },
  ],
  Notion: [
    { key: "문서", title: "Desktop UX 메모", meta: "어제 수정" },
    { key: "문서", title: "주간 작업 기록", meta: "3일 전 수정" },
  ],
} as const;

export const DOCUMENTS = [
  { title: "docs/plan/17-akra-desktop/README.md", kind: "저장소 문서" },
  { title: "PR #2231", kind: "GitHub" },
  { title: "PROJ-142", kind: "Jira" },
] as const;

export const TERMINAL_LINES = [
  "$ git status --short",
  " M src/renderer/session/desktopStore.ts",
  " M src/renderer/sidebar/Sidebar.tsx",
  " M src/renderer/styles.css",
  "$ npx vitest run",
  " Test Files  18 passed (18)",
  "      Tests  131 passed (131)",
  "$ ",
] as const;
