import { useEffect, useMemo, useState } from "react";
import { openThreadCwd, useDesktopStore } from "../session/desktopStore";
import { handOffToNewThread, useUiStore } from "../session/uiStore";
import type { GatewayRequest } from "./notionHub";
import type { DelegationActions } from "./DelegationPanel";
import { GitHubPanel } from "./GitHubPanel";
import { GitHubProjectPanel } from "./GitHubProjectPanel";
import { type IssueDraftTarget, issueDraftTarget } from "./issueDraft";
import { IssueDraftPanel } from "./IssueDraftPanel";
import { IssueCreationPanel } from "./IssueCreationPanel";
import { type IssueCreationTarget, issueCreationTarget } from "./issueCreation";
import { findFolderForRepository, readIssueHandoffPrompt } from "./githubProjects";
import { JiraPanel } from "./JiraPanel";
import { LinearPanel } from "./LinearPanel";
import { createProviderGateway } from "./providerGateway";
import { NotionPanel } from "./NotionPanel";

const WORK_SOURCES = ["GitHub", "Projects", "Jira", "Notion", "Linear"] as const;
type WorkSource = (typeof WORK_SOURCES)[number];

/** 폴더의 GitHub 저장소(`owner/name`)이다. 폴더가 없거나 GitHub 가 아니면 null 이다. */
function useGitHubRepository(cwd: string | null): string | null {
  const [repository, setRepository] = useState<string | null>(null);
  useEffect(() => {
    setRepository(null);
    if (!cwd) return;
    let current = true;
    window.akra.github.repository(cwd).then(
      (found) => current && setRepository(found),
      () => undefined,
    );
    return () => {
      current = false;
    };
  }, [cwd]);
  return repository;
}

/**
 * 이슈 내용을 새 스레드의 입력창에 채운다 (R11). 폴더는 열린 스레드의 폴더부터 사이드바의 작업 폴더까지 `origin` 이 같은
 * 것을 고른다. 없으면 고르지 않고, 보낼 때 폴더 선택 창이 열린다.
 */
async function handOffIssue(request: GatewayRequest, url: string, repository: string, number: number): Promise<void> {
  const state = useDesktopStore.getState();
  const openCwd = openThreadCwd(state);
  const folders = [...(openCwd ? [openCwd] : []), ...state.folders.map((folder) => folder.cwd)];
  const cwd = await findFolderForRepository(repository, folders, (folder) => window.akra.github.repository(folder));
  const text = await readIssueHandoffPrompt(request, url, repository, number, cwd);
  useUiStore.setState({ homeCwd: cwd });
  handOffToNewThread(text);
}

export function WorkTab({ reviewJobs, request, onHandOff, onLinearHandOff }: {
  readonly reviewJobs: Pick<DelegationActions, "detail" | "cancel">;
  request: GatewayRequest;
  onHandOff(text: string): Promise<void>;
  onLinearHandOff(text: string): Promise<void>;
}) {
  const providers = useMemo(() => createProviderGateway(request, window.akra.providerCredentials, {
    onChanged: listener => window.akra.onMessage(text => {
      try {
        const message = JSON.parse(text) as { method?: unknown; params?: unknown };
        if (message.method === "akra/providerCredentials/changed") listener(message.params);
      } catch { /* not a credential notification */ }
    }),
    onConnectionChanged: listener => window.akra.onConnectionState(() => listener()),
  }), [request]);
  useEffect(() => { providers.start(); return () => providers.dispose(); }, [providers]);
  const [source, setSource] = useState<WorkSource>("GitHub");
  const workFolder = useDesktopStore(openThreadCwd);
  const currentRepository = useGitHubRepository(workFolder);
  // 고치는 중인 이슈이다 (#2426). 그 글은 이 패널에만 있고 대화 입력창으로 가지 않는다.
  const [editing, setEditing] = useState<IssueDraftTarget | null>(null);
  const [creating, setCreating] = useState<IssueCreationTarget | null>(null);
  const createIssue = async (repository: string) => {
    const state = useDesktopStore.getState();
    const openCwd = openThreadCwd(state);
    const folders = [...(openCwd ? [openCwd] : []), ...state.folders.map((folder) => folder.cwd)];
    const cwd = await findFolderForRepository(repository, folders, (folder) => window.akra.github.repository(folder));
    const target = cwd ? issueCreationTarget(cwd, repository) : null;
    if (!target) throw new Error("이 저장소의 작업 폴더가 없습니다. 그 저장소의 폴더를 사이드바에 추가하세요.");
    setCreating(target);
  };
  if (creating) return <IssueCreationPanel request={request} target={creating} onClose={() => setCreating(null)} openExternal={(url) => window.akra.github.openExternal(url)} />;
  if (editing) return <IssueDraftPanel reviewJobs={reviewJobs} request={request} target={editing} onClose={() => setEditing(null)} />;
  return (
    <div className="work-tab">
      <div className="segmented">
        {WORK_SOURCES.map((name) => (
          <button key={name} type="button" className={source === name ? "selected" : undefined} onClick={() => setSource(name)} data-testid={`${name.toLowerCase()}-toggle`}>
            {name}
          </button>
        ))}
      </div>
      {source === "GitHub" && <GitHubPanel github={window.akra.github} currentRepository={currentRepository}
        onCreate={async () => { if (currentRepository) await createIssue(currentRepository); }}
        onHandOff={async (pull) => onHandOff(await window.akra.github.reviewPrompt(pull.url))} />}
      {source === "Projects" && (
        <GitHubProjectPanel
          github={window.akra.github}
          currentRepository={currentRepository}
          onCreate={createIssue}
          onHandOff={async (item) => {
            if (item.url && item.repository && item.number !== null) await handOffIssue(request, item.url, item.repository, item.number);
          }}
          onEdit={async (item) => {
            if (!item.repository || item.number === null) return;
            const state = useDesktopStore.getState();
            const openCwd = openThreadCwd(state);
            const folders = [...(openCwd ? [openCwd] : []), ...state.folders.map((folder) => folder.cwd)];
            const cwd = await findFolderForRepository(item.repository, folders, (folder) => window.akra.github.repository(folder));
            const target = cwd ? issueDraftTarget(cwd, item.repository, item.number) : null;
            if (!target) throw new Error("이 저장소의 작업 폴더가 없습니다. 그 저장소의 폴더를 사이드바에 추가하세요.");
            setEditing(target);
          }}
        />
      )}
      {source === "Jira" && <JiraPanel jira={providers.jira} onEdit={workFolder ? async issue => { const jira=await providers.jira.draftTarget(issue.key); setEditing({cwd:workFolder,jira,displayKey:issue.key}); } : undefined} onHandOff={async (issue) => onHandOff(await providers.jira.issuePrompt(issue.key))} />}
      {source === "Notion" && <NotionPanel request={request} notion={window.akra.notion} onHandOff={onHandOff} />}
      {source === "Linear" && <LinearPanel linear={providers.linear} onHandOff={async (issue) => onLinearHandOff(await providers.linear.issuePrompt(issue.id))} />}
    </div>
  );
}
