// GitHub Project 패널이다 (R11). Project 하나의 열린 이슈·PR 과 초안을 Status 별로 보여 주고, 저장소로 거른다.
// 기본 필터는 열린 스레드의 저장소이다. 열 때와 "새로 고침" 때만 읽는다. "스레드로 넘기기" 는 이슈 내용으로 새 스레드의
// 입력창만 채운다. 보내기는 사용자가 한다.
import { type FormEvent, useEffect, useState } from "react";

import type { GitHubBridge, GitHubProject, GitHubProjectItem } from "../../shared/bridge";
import { filterByRepository, groupByStatus, repositoriesOf } from "./githubProjects";

const PROJECT_KEY = "akra.githubProject";

/** 저장한 Project (`owner/번호` 또는 주소)이다. 저장소를 쓸 수 없으면 null 이다. */
function loadProjectRef(): string | null {
  try {
    return window.localStorage.getItem(PROJECT_KEY);
  } catch {
    return null;
  }
}

/** 저장하지 못해도 이번 화면에서는 쓴다. 다음에 열 때 다시 묻는다. */
function saveProjectRef(project: string): void {
  try {
    window.localStorage.setItem(PROJECT_KEY, project);
  } catch {
    // 저장소를 쓸 수 없는 환경이다.
  }
}

type ListState = { phase: "loading" } | { phase: "loaded"; project: GitHubProject } | { phase: "failed"; message: string };

/** "전체" 의 선택값이다. 저장소 이름(`owner/name`)에는 `*` 가 들어갈 수 없다. */
const ALL = "*";

const KIND_LABEL: Record<GitHubProjectItem["kind"], string> = { issue: "이슈", pullRequest: "PR", draft: "초안" };

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function ProjectForm({ initial, onSave, onCancel }: { initial: string; onSave(project: string): void; onCancel?(): void }) {
  const [value, setValue] = useState(initial);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (value.trim()) onSave(value.trim());
  };
  return (
    <form className="work-section work-form" onSubmit={submit} data-testid="github-project-form">
      <p className="muted">읽을 GitHub Project 를 적으세요. gh 로그인에 `read:project` 권한이 필요합니다.</p>
      <label>
        Project
        <input value={value} onChange={(event) => setValue(event.target.value)} placeholder="owner/1 또는 https://github.com/users/owner/projects/1" required />
      </label>
      <button type="submit">저장</button>
      {onCancel && (
        <button type="button" onClick={onCancel}>
          취소
        </button>
      )}
    </form>
  );
}

function ItemRow({ item, github, onHandOff }: { item: GitHubProjectItem; github: GitHubBridge; onHandOff(item: GitHubProjectItem): Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const handOff = async () => {
    setBusy(true);
    setError(null);
    try {
      await onHandOff(item);
    } catch (cause) {
      setError(message(cause));
    } finally {
      setBusy(false);
    }
  };
  const url = item.url;
  return (
    <li className="work-item" data-testid="github-project-item">
      {url ? (
        <button type="button" className="work-link" onClick={() => github.openExternal(url)} title={url}>
          {item.title}
        </button>
      ) : (
        <span>{item.title}</span>
      )}
      <div className="work-meta">
        {item.repository && (
          <span className="muted">
            {item.repository}#{item.number}
          </span>
        )}
        <span className="work-badge">{KIND_LABEL[item.kind]}</span>
        {item.kind === "issue" && (
          <button type="button" onClick={() => void handOff()} disabled={busy} data-testid="github-project-hand-off">
            {busy ? "읽는 중…" : "스레드로 넘기기"}
          </button>
        )}
      </div>
      {error && <div className="error">{error}</div>}
    </li>
  );
}

export function GitHubProjectPanel({
  github,
  currentRepository,
  onHandOff,
}: {
  github: GitHubBridge;
  /** 열린 스레드 폴더의 `owner/name` 이다. 모르면 null 이다. */
  currentRepository: string | null;
  onHandOff(item: GitHubProjectItem): Promise<void>;
}) {
  const [projectRef, setProjectRef] = useState<string | null>(loadProjectRef);
  const [editing, setEditing] = useState(false);
  const [state, setState] = useState<ListState>({ phase: "loading" });
  const [refreshCount, setRefreshCount] = useState(0);
  // null 이면 아직 고르지 않았다. 그때는 현재 저장소(없으면 전체)를 보여 준다.
  const [chosen, setChosen] = useState<string | null>(null);

  useEffect(() => {
    if (!projectRef) return;
    let current = true;
    setState({ phase: "loading" });
    github.project(projectRef).then(
      (project) => current && setState({ phase: "loaded", project }),
      (error: unknown) => current && setState({ phase: "failed", message: message(error) }),
    );
    return () => {
      current = false;
    };
  }, [github, projectRef, refreshCount]);

  const save = (project: string) => {
    saveProjectRef(project);
    setProjectRef(project);
    setEditing(false);
    setChosen(null);
  };

  if (!projectRef || editing) {
    return (
      <aside className="work-panel" data-testid="github-project-panel">
        <div className="work-header">
          <strong>GitHub Project</strong>
        </div>
        <ProjectForm initial={projectRef ?? ""} onSave={save} onCancel={projectRef ? () => setEditing(false) : undefined} />
      </aside>
    );
  }

  const loaded = state.phase === "loaded" ? state.project : null;
  const repositories = loaded ? repositoriesOf(loaded.items) : [];
  // `origin` 의 대소문자는 GitHub 의 이름과 다를 수 있다. 선택 상자는 대소문자를 구분하므로 목록의 이름으로 바꾼다.
  const wanted = chosen ?? currentRepository ?? ALL;
  const selected = wanted === ALL ? ALL : (repositories.find((name) => name.toLowerCase() === wanted.toLowerCase()) ?? wanted);
  const repository = selected === ALL ? null : selected;
  if (repository && !repositories.includes(repository)) repositories.unshift(repository);
  const shown = loaded ? filterByRepository(loaded.items, repository) : [];

  return (
    <aside className="work-panel" data-testid="github-project-panel">
      <div className="work-header">
        <strong>{loaded?.title || "GitHub Project"}</strong>
        <span className="work-actions">
          <button type="button" onClick={() => setRefreshCount((count) => count + 1)} disabled={state.phase === "loading"}>
            {state.phase === "loading" ? "읽는 중…" : "새로 고침"}
          </button>
          <button type="button" onClick={() => setEditing(true)} data-testid="github-project-change">
            변경
          </button>
        </span>
      </div>
      {state.phase === "failed" && (
        <div className="error work-section" data-testid="github-project-error">
          Project 를 읽을 수 없습니다. gh 로그인과 권한(`gh auth refresh -s read:project`)을 확인하세요. {state.message}
        </div>
      )}
      {loaded && (
        <div className="work-scroll">
          <div className="work-section work-form">
            <label>
              저장소
              <select value={selected} onChange={(event) => setChosen(event.target.value)} data-testid="github-project-filter">
                <option value={ALL}>전체</option>
                {repositories.map((name) => (
                  <option key={name} value={name}>
                    {name}
                    {currentRepository && name.toLowerCase() === currentRepository.toLowerCase() ? " (현재)" : ""}
                  </option>
                ))}
              </select>
            </label>
          </div>
          {shown.length === 0 && <p className="muted work-section">열린 항목이 없습니다.</p>}
          {groupByStatus(shown).map((group) => (
            <section className="work-section" key={group.status ?? ALL} data-testid="github-project-group">
              <h3>
                {group.status ?? "Status 없음"} ({group.items.length})
              </h3>
              <ul>
                {group.items.map((item, index) => (
                  <ItemRow key={item.url ?? `draft-${index}`} item={item} github={github} onHandOff={onHandOff} />
                ))}
              </ul>
            </section>
          ))}
          {(loaded.closedCount > 0 || loaded.truncated) && (
            <p className="muted work-section">
              {loaded.closedCount > 0 ? `닫힌 항목 ${loaded.closedCount}개는 숨겼습니다.` : ""}
              {loaded.truncated ? " 항목이 많아서 앞의 일부만 읽었습니다." : ""}
            </p>
          )}
        </div>
      )}
    </aside>
  );
}
