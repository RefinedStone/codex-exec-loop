// 대화 밖의 화면이다 (R10): 새 스레드(홈), 현황판, 검색과 명령, 단축키 목록, 알림.
import { useEffect, useMemo, useState } from "react";
import { isAttentionPreferenceEvent, readAttentionPreference, writeAttentionPreference, type AttentionPreference, type AttentionPreferenceField } from "../kernel/attentionPreferences";
import type { AttentionItem } from "../../shared/kernel-protocol/generated";


import { Composer, type ComposerActions } from "../conversation/Composer";
import { Icon } from "../design/icons";
import { Picker } from "../design/Picker";
import { useDesktopStore } from "../session/desktopStore";
import { openPanel, useUiStore } from "../session/uiStore";
import { StatusMark, useThreadMarks } from "../sidebar/Sidebar";
import { allThreads, folderName, type ListedThread, relativeAge, threadMark, type ThreadMark } from "../sidebar/threadStatus";

const CHOOSE_FOLDER = "폴더 고르기…";
const RECENT_COUNT = 5;

export function Home({ actions, onChooseFolder, onOpenThread }: { actions: ComposerActions; onChooseFolder(): Promise<string | null>; onOpenThread(threadId: string): void }) {
  const homeCwd = useUiStore((state) => state.homeCwd);
  const workspaceMode = useUiStore((state) => state.homeWorkspaceMode);
  const folders = useDesktopStore((state) => state.folders);
  const marks = useThreadMarks();
  const recent = useMemo(() => allThreads(folders).slice(0, RECENT_COUNT), [folders]);
  // 같은 이름의 폴더가 둘이면 구분할 수 있게 전체 경로를 보여 준다.
  const options = [...new Set([...(homeCwd ? [homeCwd] : []), ...folders.map((folder) => folder.cwd)]), CHOOSE_FOLDER];
  const choose = (value: string) => {
    if (value !== CHOOSE_FOLDER) {
      useUiStore.setState({ homeCwd: value });
      return;
    }
    void onChooseFolder().then((cwd) => cwd && useUiStore.setState({ homeCwd: cwd }));
  };
  return (
    <div className="home" data-testid="home">
      <div className="home-inner">
        <h1>무엇을 할까요?</h1>
        <div className="home-pickers">
          <Picker icon={<Icon name="folder" size={12} />} value={homeCwd ?? CHOOSE_FOLDER} options={options} onChange={choose} />
          <label>작업 위치 <select aria-label="작업 위치" value={workspaceMode} onChange={event => useUiStore.setState({ homeWorkspaceMode: event.target.value === "isolated" ? "isolated" : "existing" })}>
            <option value="existing">기존 폴더</option>
            <option value="isolated">격리 공간 (HEAD 기준)</option>
          </select></label>
        </div>
        {workspaceMode === "isolated" && <p className="muted">커밋된 HEAD에서 별도 브랜치와 worktree를 준비합니다. 미커밋 변경은 복사하지 않습니다.</p>}
        {/* 폴더를 바꾸면 입력창을 새로 만든다. 이전 폴더의 `@` 후보와 진행 중인 찾기를 버린다 (#2837). 초안은 저장소에 남는다. */}
        <Composer key={homeCwd ?? ""} actions={actions} home />
        {recent.length > 0 && <h3>최근 스레드</h3>}
        <ul className="recent">
          {recent.map((thread) => (
            <li key={thread.id}>
              <button type="button" onClick={() => onOpenThread(thread.id)}>
                <StatusMark mark={threadMark(thread, marks)} />
                <span className="recent-title">{thread.title}</span>
                <span className="muted">{folderName(thread.cwd)}</span>
                <span className="muted">{relativeAge(thread.updatedAt, Date.now())}</span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

const FLEET_COLUMNS: { title: string; marks: ThreadMark[]; hint: string }[] = [
  { title: "확인 필요", marks: ["approval", "failed"], hint: "승인 또는 실패 확인을 기다립니다" },
  { title: "진행 중", marks: ["running"], hint: "에이전트가 작업합니다" },
  { title: "읽지 않음", marks: ["unread"], hint: "끝났고 아직 보지 않았습니다" },
  { title: "최근", marks: ["idle"], hint: "본 스레드입니다" },
];
/** shortcut: 한 열에 최근 30개까지만 그린다. 스레드가 수천 개일 때 화면이 느려지지 않게 한다. 더 필요하면 가상 스크롤로 바꾼다. */
const FLEET_COLUMN_LIMIT = 30;

function FleetCard({ thread, mark, onOpenThread }: { thread: ListedThread; mark: ThreadMark; onOpenThread(threadId: string): void }) {
  return (
    <button type="button" className="fleet-card" onClick={() => onOpenThread(thread.id)} data-testid="fleet-card">
      <span className="fleet-card-title">
        <StatusMark mark={mark} />
        {thread.title}
      </span>
      <span className="fleet-card-meta">
        <Icon name="folder" size={12} />
        {folderName(thread.cwd)}
        {thread.branch && (
          <>
            <Icon name="branch" size={12} />
            {thread.branch}
          </>
        )}
        <span className="bar-spacer" />
        {relativeAge(thread.updatedAt, Date.now())}
      </span>
    </button>
  );
}

function KernelAttentionList({ kernel, items, onOpenThread }: { kernel: string | null; items: readonly AttentionItem[] | null; onOpenThread(id: string): void }) {
  const [, update] = useState(0);
  const [notice, setNotice] = useState<string | null>(null);
  useEffect(() => {
    const refresh = (event: StorageEvent) => { if (isAttentionPreferenceEvent(event.key)) update(value => value + 1); };
    window.addEventListener("storage", refresh);
    const timer = window.setInterval(() => update(value => value + 1), 30_000);
    return () => { window.removeEventListener("storage", refresh); window.clearInterval(timer); };
  }, []);
  const save = <K extends AttentionPreferenceField>(item: AttentionItem, field: K, value: AttentionPreference[K]) => {
    if (!kernel || !writeAttentionPreference(localStorage, kernel, item, field, value)) setNotice("개인 정리 상태를 저장하지 못했습니다.");
    else { setNotice(null); update(revision => revision + 1); }
  };
  const rows = (kernel && items ? items.map(item => ({ item, preference: readAttentionPreference(localStorage, kernel, item) })) : [])
    .sort((a, b) => Number(b.preference.pinned) - Number(a.preference.pinned));
  const now = Date.now();
  return <section className="fleet-column" data-testid="kernel-attention">
    <h3>현재 저장소 · 커널 확인 필요 <span className="muted">{items?.length ?? "—"}</span></h3>
    <p className="muted">읽음·미루기·고정은 이 기기의 개인 표시 설정입니다. 응답을 대신하지 않습니다.</p>
    {notice && <p role="status">{notice}</p>}
    {items === null && <p className="muted fleet-empty">커널 확인 항목을 아직 읽지 못했습니다.</p>}
    {items?.length === 0 && <p className="muted fleet-empty">없음</p>}
    {rows.map(({ item, preference }) => {
      const snoozed = preference.snoozedUntil !== null && preference.snoozedUntil > now;
      return <div key={item.id} className="fleet-card">
        <button type="button" className="fleet-card" onClick={() => item.subject.kind === "thread" ? onOpenThread(item.subject.id) : openPanel(item.subject.kind === "issue_decision" ? "issues" : "kernel")}>
          <span className="fleet-card-title">{item.summary}</span>
          <span className="fleet-card-meta">{item.cause} · {item.subject.kind} · {preference.read ? "읽음" : "안 읽음"}{snoozed ? " · 미룸" : ""}</span>
          <span className="muted">{item.next.map(step => "name" in step ? step.name : step.step).join(" → ")}</span>
        </button>
        <div className="fleet-card-meta">
          <button type="button" className="button" onClick={() => save(item, "read", !preference.read)}>{preference.read ? "안 읽음" : "읽음"}</button>
          <button type="button" className="button" onClick={() => save(item, "pinned", !preference.pinned)}>{preference.pinned ? "고정 해제" : "고정"}</button>
          <button type="button" className="button" onClick={() => save(item, "snoozedUntil", snoozed ? null : now + 60 * 60 * 1000)}>{snoozed ? "미루기 해제" : "1시간 미루기"}</button>
        </div>
      </div>;
    })}
  </section>;
}

export function Fleet({ onOpenThread }: { onOpenThread(threadId: string): void }) {
  const folders = useDesktopStore((state) => state.folders);
  const marks = useThreadMarks();
  const kernel = useDesktopStore((state) => state.kernel);
  const items = kernel.phase === "attached" ? kernel.attentionItems : null;
  const threads = useMemo(() => allThreads(folders).map((thread) => ({ thread, mark: threadMark(thread, marks) })), [folders, marks]);
  return (
    <div className="fleet" data-testid="fleet">
      <header className="fleet-header">
        <h1>현황판</h1>
        <span className="muted">모든 작업 폴더의 스레드를 상태별로 봅니다.</span>
      </header>
      <KernelAttentionList kernel={kernel.kernel} items={items} onOpenThread={onOpenThread} />
      <div className="fleet-columns">
        {FLEET_COLUMNS.map((column) => {
          const items = threads.filter((item) => column.marks.includes(item.mark));
          return (
            <section key={column.title} className="fleet-column">
              <h3 title={column.hint}>
                {column.title} <span className="muted">{items.length}</span>
              </h3>
              {items.slice(0, FLEET_COLUMN_LIMIT).map((item) => (
                <FleetCard key={item.thread.id} thread={item.thread} mark={item.mark} onOpenThread={onOpenThread} />
              ))}
              {items.length === 0 && <p className="muted fleet-empty">없음</p>}
            </section>
          );
        })}
      </div>
    </div>
  );
}

/** 확대 값을 바꾼 직후에 배율을 잠깐 보인다 (plan 26 Z2). */
export function ZoomNotice() {
  const text = useUiStore((state) => state.zoomNotice);
  if (text === null) return null;
  return (
    <div className="zoom-notice" role="status" data-testid="zoom-notice">
      {text}
    </div>
  );
}

export function Toasts() {
  const toasts = useUiStore((state) => state.toasts);
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((item) => (
        <div key={item.id} className="toast">
          {item.text}
        </div>
      ))}
    </div>
  );
}


export { CommandPalette, commandHandlers, ShortcutsDialog, useShortcuts } from "./CommandViews";
