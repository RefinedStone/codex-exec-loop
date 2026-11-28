import { useMemo } from "react";
import { Composer, type ComposerActions } from "../conversation/Composer";
import { Icon } from "../design/icons";
import { Picker } from "../design/Picker";
import { useDesktopStore } from "../session/desktopStore";
import { useUiStore } from "../session/uiStore";
import { StatusMark, useThreadMarks } from "../sidebar/Sidebar";
import { allThreads, folderName, relativeAge, threadMark } from "../sidebar/threadStatus";
import type { GatewayRequest } from "../work/notionHub";
import { WorkspaceRecovery } from "./WorkspaceRecovery";

const CHOOSE_FOLDER = "폴더 고르기…";
const RECENT_COUNT = 5;

export function Home({ actions, request, onChooseFolder, onOpenThread }: {
  readonly actions: ComposerActions;
  readonly request: GatewayRequest;
  readonly onChooseFolder: () => Promise<string | null>;
  readonly onOpenThread: (threadId: string) => void;
}) {
  const homeCwd = useUiStore((state) => state.homeCwd);
  const workspaceMode = useUiStore((state) => state.homeWorkspaceMode);
  const folders = useDesktopStore((state) => state.folders);
  const marks = useThreadMarks();
  const recent = useMemo(() => allThreads(folders).slice(0, RECENT_COUNT), [folders]);
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
        <Composer key={homeCwd ?? ""} actions={actions} home />
        {homeCwd && <WorkspaceRecovery key={`recovery:${homeCwd}`} cwd={homeCwd} request={request} />}
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
