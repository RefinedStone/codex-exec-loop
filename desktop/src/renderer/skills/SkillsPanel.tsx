// 설치된 스킬 패널이다 (#2383): 어디서 설치됐는지, 고른 채널에 업데이트가 있는지, 무엇이 바뀌는지, 적용과 롤백.
// 목록의 Codex 행은 `skills/list` 에서, Claude Code 행은 main 이 디스크에서 읽는다. 파일은 main 만 쓴다.
// 이름이나 날짜로 출처를 짐작하지 않는다: 사용자가 출처를 연결하기 전에는 "출처 미확인"이고 바꾸는 버튼이 없다.
import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";

import type { SkillsListResponse } from "../../shared/protocol/generated/v2/SkillsListResponse";
import type {
  CodexSkillInput,
  SkillActionResult,
  SkillCheckFailureKind,
  SkillManagement,
  SkillPreview,
  SkillPreviewFile,
  SkillRow,
  SkillsBridge,
  SkillSourceInput,
  SkillTrackingPolicy,
} from "../../shared/skills";

const SkillFileDiff = lazy(() => import("./SkillFileDiff"));

const MANAGEMENT_LABEL: Record<Exclude<SkillManagement, "copy">, string> = {
  plugin: "플러그인·마켓플레이스 설치 (읽기 전용): 그 플러그인 관리자에서 업데이트하세요.",
  system: "시스템 번들 (읽기 전용): 런타임과 함께 업데이트됩니다.",
  admin: "관리자 설치 (읽기 전용): 관리자에게 문의하세요.",
  symlink: "심볼릭 링크 (읽기 전용): Akra 는 링크를 고치거나 대체하지 않습니다. 링크를 만든 도구에서 업데이트하세요.",
  external: "Akra 가 아는 스킬 폴더 밖 (읽기 전용)",
};
const SCOPE_LABEL = { user: "사용자", project: "프로젝트", system: "시스템", admin: "관리자" } as const;
const FAILURE_LABEL: Record<SkillCheckFailureKind, string> = {
  offline: "확인 실패 (오프라인)",
  rateLimited: "확인 실패 (요청 한도)",
  moved: "원본 이동",
  missing: "원본 삭제 또는 경로 없음",
  noRelease: "안정 릴리스 없음",
  invalid: "확인 실패",
};
const CHANGE_LABEL = { add: "추가", modify: "수정", delete: "삭제" } as const;

function policyLabel(policy: SkillTrackingPolicy): string {
  return policy.kind === "release" ? "안정 릴리스" : policy.kind === "branch" ? `브랜치 ${policy.branch}` : `고정 ${policy.ref.slice(0, 12)}`;
}

function shortCommit(commit: string | null): string {
  return commit ? commit.slice(0, 12) : "미확인";
}

/** 상태 표시이다. 가장 최근 확인이 실패했으면 "최신"이라고 하지 않는다. */
function statusLabel(row: SkillRow): string {
  if (row.management !== "copy") return "읽기 전용";
  const entry = row.entry;
  switch (row.state.status) {
    case "sourceUnknown":
      return "출처 미확인";
    case "notChecked":
      return "확인 전";
    case "checkFailed":
      return FAILURE_LABEL[entry?.lastFailure?.kind ?? "invalid"];
    case "baselineUnknown":
      return "설치 버전 미확인";
    case "pinned":
      return "고정";
    case "latest":
      return `최신 (${entry ? policyLabel(entry.source.policy) : ""})`;
    case "updateAvailable":
      return "업데이트 가능";
  }
}

function LinkForm({ row, busy, onSubmit, onCancel }: { row: SkillRow; busy: boolean; onSubmit(source: SkillSourceInput): void; onCancel(): void }) {
  const source = row.entry?.source;
  const [repository, setRepository] = useState(source?.repository ?? "");
  const [subPath, setSubPath] = useState(source?.subPath ?? "");
  const [kind, setKind] = useState<SkillTrackingPolicy["kind"]>(source?.policy.kind ?? "release");
  const [ref, setRef] = useState(source?.policy.kind === "branch" ? source.policy.branch : source?.policy.kind === "pinned" ? source.policy.ref : "");
  const policy: SkillTrackingPolicy = kind === "release" ? { kind } : kind === "branch" ? { kind, branch: ref } : { kind, ref };
  return (
    <form
      className="work-form skill-form"
      data-testid="skill-link-form"
      onSubmit={(event) => {
        event.preventDefault();
        onSubmit({ repository, subPath, policy });
      }}
    >
      <label>
        공개 GitHub 저장소 (owner/name 또는 주소)
        <input value={repository} onChange={(event) => setRepository(event.target.value)} placeholder="owner/name" data-testid="skill-link-repository" required />
      </label>
      <label>
        저장소 안 스킬 폴더 (비우면 저장소의 루트)
        <input value={subPath} onChange={(event) => setSubPath(event.target.value)} placeholder="skills/name" data-testid="skill-link-subpath" />
      </label>
      <label>
        추적 정책
        <select value={kind} onChange={(event) => setKind(event.target.value as SkillTrackingPolicy["kind"])} data-testid="skill-link-policy">
          <option value="release">안정 릴리스 (prerelease 제외)</option>
          <option value="branch">지정 브랜치</option>
          <option value="pinned">고정 ref (올리지 않음)</option>
        </select>
      </label>
      {kind !== "release" && (
        <label>
          {kind === "branch" ? "브랜치" : "커밋, 태그 또는 브랜치"}
          <input value={ref} onChange={(event) => setRef(event.target.value)} data-testid="skill-link-ref" required />
        </label>
      )}
      <p className="muted">연결하면 원본을 한 번 확인합니다. 파일은 바꾸지 않습니다. 원본의 파일 묶음이 설치 파일과 같을 때만 설치 커밋을 적습니다.</p>
      <span className="work-actions skill-actions">
        <button type="submit" disabled={busy} data-testid="skill-link-submit">
          {busy ? "확인하는 중…" : "출처 연결"}
        </button>
        <button type="button" onClick={onCancel}>
          닫기
        </button>
      </span>
    </form>
  );
}

function PreviewFile({ file }: { file: SkillPreviewFile }) {
  return (
    <details className="skill-file" data-testid="skill-preview-file">
      <summary>
        <span className={`work-badge skill-change-${file.change}`}>{CHANGE_LABEL[file.change]}</span> {file.path}{" "}
        {file.behavior && (
          <span className="work-badge work-changed" title="스크립트, 실행 파일 또는 SKILL.md 의 머리(도구 선언 등)가 바뀝니다." data-testid="skill-behavior">
            동작 변경 가능
          </span>
        )}
      </summary>
      {file.oldText === null || file.newText === null ? (
        <p className="muted">바이너리이거나 큰 파일입니다. 글로 보일 수 없습니다.</p>
      ) : (
        <Suspense fallback={<p className="muted">차이를 읽는 중…</p>}>
          <SkillFileDiff path={file.path} oldText={file.oldText} newText={file.newText} />
        </Suspense>
      )}
    </details>
  );
}

function PreviewView({ row, preview, busy, onApply, onClose }: { row: SkillRow; preview: SkillPreview; busy: boolean; onApply(): void; onClose(): void }) {
  return (
    <div className="skill-preview" data-testid="skill-preview">
      <div className="work-meta">
        <span>
          원본 {preview.source.repository}
          {preview.source.subPath && `/${preview.source.subPath}`}
        </span>
        <span>
          커밋 {shortCommit(preview.currentCommit)} → {shortCommit(preview.targetCommit)}
        </span>
        <span>
          대상 {row.identity.runtime === "codex" ? "Codex" : "Claude Code"} · {SCOPE_LABEL[row.identity.scope]}
        </span>
      </div>
      <p className="muted">
        이 설치 폴더만 통째로 바꿉니다. 같은 저장소의 다른 스킬은 따라오지 않습니다. 로컬 수정은 병합하지 않고 스냅샷으로 남깁니다. 받은 파일은 실행하지 않습니다. 파일 일치 확인은 안전 인증이 아닙니다: 내용을
        직접 검토하세요.
      </p>
      {preview.files.map((file) => (
        <PreviewFile key={file.path} file={file} />
      ))}
      <span className="work-actions skill-actions">
        <button type="button" onClick={onApply} disabled={busy} data-testid="skill-apply">
          {busy ? "바꾸는 중…" : `커밋 ${shortCommit(preview.targetCommit)} 로 바꾸기`}
        </button>
        <button type="button" onClick={onClose}>
          닫기
        </button>
      </span>
    </div>
  );
}

type OpenView = { key: string; mode: "link" } | { key: string; mode: "preview"; preview: SkillPreview };

export function SkillsPanel({
  cwd,
  connected,
  skills,
  listCodexSkills,
  onSkillsChanged,
}: {
  /** 프로젝트 범위를 읽을 작업 폴더이다. */
  cwd: string | null;
  connected: boolean;
  skills: SkillsBridge;
  listCodexSkills(cwd: string | null, forceReload: boolean): Promise<SkillsListResponse>;
  /** `skills/changed` 알림이다: 로컬 파일이 바뀌었다는 신호이다. 원격 업데이트 신호가 아니다. */
  onSkillsChanged(listener: () => void): () => void;
}) {
  const [rows, setRows] = useState<SkillRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [codexError, setCodexError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [open, setOpen] = useState<OpenView | null>(null);
  const [messages, setMessages] = useState<Record<string, string>>({});
  /** 이 패널에서 파일을 바꾼 설치이다. 진행 중인 세션이 새 내용을 쓰는지는 확인할 수 없다. */
  const [replaced, setReplaced] = useState<ReadonlySet<string>>(new Set());
  const generation = useRef(0);

  const load = useCallback(
    async (forceReload: boolean) => {
      generation.current += 1;
      const current = generation.current;
      let codex: CodexSkillInput[] = [];
      let codexFailure: string | null = null;
      try {
        const response = await listCodexSkills(cwd, forceReload);
        codex = response.data.flatMap((entry) => entry.skills.map((skill) => ({ name: skill.name, path: skill.path, scope: skill.scope, enabled: skill.enabled, pluginId: skill.pluginId })));
      } catch (failure) {
        codexFailure = failure instanceof Error ? failure.message : String(failure);
      }
      try {
        const listed = await skills.list({ cwd, codex });
        if (current !== generation.current) return;
        setRows(listed);
        setError(null);
        setCodexError(codexFailure);
      } catch (failure) {
        if (current === generation.current) setError(failure instanceof Error ? failure.message : String(failure));
      }
    },
    [cwd, skills, listCodexSkills],
  );

  useEffect(() => {
    void load(false);
    return onSkillsChanged(() => void load(false));
  }, [load, onSkillsChanged, connected]);

  const act = async (row: SkillRow, action: () => Promise<SkillActionResult>, replaces = false): Promise<void> => {
    setBusy(row.key);
    try {
      const result = await action();
      setMessages((all) => ({ ...all, [row.key]: result.ok || result.cancelled ? "" : result.message }));
      if (result.ok) setOpen(null);
      if (result.ok && replaces) setReplaced((keys) => new Set(keys).add(row.key));
      // 파일을 바꾼 뒤에는 Codex 가 캐시 대신 디스크를 다시 읽게 한다.
      await load(result.ok && replaces);
    } finally {
      setBusy(null);
    }
  };

  const showPreview = async (row: SkillRow): Promise<void> => {
    setBusy(row.key);
    try {
      const result = await skills.preview(row.identity);
      setMessages((all) => ({ ...all, [row.key]: result.ok ? "" : result.message }));
      setOpen(result.ok ? { key: row.key, mode: "preview", preview: result.preview } : null);
    } finally {
      setBusy(null);
    }
  };

  const renderRow = (row: SkillRow) => {
    const entry = row.entry;
    const writable = row.management === "copy" && row.scanError === null;
    const isBusy = busy === row.key;
    return (
      <li key={row.key} className="work-item skill-row" data-testid="skill-row" data-skill={`${row.identity.runtime}:${row.identity.scope}:${row.name}`}>
        <div className="work-meta">
          <strong>{row.name}</strong>
          <span className="work-badge">{SCOPE_LABEL[row.identity.scope]}</span>
          <span className={`work-badge skill-status-${row.management === "copy" ? row.state.status : "readOnly"}`} data-testid="skill-status">
            {statusLabel(row)}
          </span>
          {row.state.localModified && (
            <span className="work-badge work-changed" data-testid="skill-modified">
              로컬 수정
            </span>
          )}
          {row.enabled === false && <span className="work-badge">Codex 에서 꺼짐</span>}
        </div>
        <div className="muted skill-path" title={row.identity.installPath}>
          {row.identity.installPath}
        </div>
        {row.management !== "copy" && <div className="muted">{MANAGEMENT_LABEL[row.management]}</div>}
        {row.scanError && <div className="error">쓸 수 없습니다: {row.scanError}</div>}
        {entry && (
          <div className="muted" data-testid="skill-source">
            {entry.source.repository}
            {entry.source.subPath && `/${entry.source.subPath}`} · {policyLabel(entry.source.policy)} · 설치 커밋 {shortCommit(entry.installedCommit)}
            {entry.installedCommit && entry.installedManifestHash === row.localHash && " (파일 묶음 일치)"}
          </div>
        )}
        {entry?.lastSuccess && (
          <div className="muted">
            {entry.lastFailure ? "마지막 성공 확인" : "확인"} {new Date(entry.lastSuccess.at).toLocaleString()} · {entry.lastSuccess.ref} → {shortCommit(entry.lastSuccess.commit)}
          </div>
        )}
        {entry?.lastFailure && (
          <div className="error" data-testid="skill-failure">
            {new Date(entry.lastFailure.at).toLocaleString()} · {entry.lastFailure.message} 지금 최신인지 알 수 없습니다.
          </div>
        )}
        {row.state.status === "baselineUnknown" && <div className="muted">원본의 파일 묶음이 설치 파일과 다릅니다. 설치 버전을 알 수 없습니다. 차이를 검토한 뒤에 원본의 묶음으로 바꿀 수 있습니다.</div>}
        {replaced.has(row.key) && (
          <div className="muted" data-testid="skill-reload-note">
            디스크의 설치를 바꿨습니다. 진행 중인 세션에 반영됐는지는 확인할 수 없습니다: 새 세션을 시작하세요.
          </div>
        )}
        {messages[row.key] && (
          <div className="error" data-testid="skill-message">
            {messages[row.key]}
          </div>
        )}
        {writable && (
          <span className="work-actions skill-actions">
            <button type="button" onClick={() => setOpen(open?.key === row.key && open.mode === "link" ? null : { key: row.key, mode: "link" })} disabled={isBusy} data-testid="skill-link">
              {entry ? "출처 변경" : "출처 연결"}
            </button>
            {entry && (
              <button type="button" onClick={() => void act(row, () => skills.check(row.identity))} disabled={isBusy} data-testid="skill-check">
                {isBusy ? "확인하는 중…" : "지금 확인"}
              </button>
            )}
            {row.state.canReplace && (
              <button type="button" onClick={() => void showPreview(row)} disabled={isBusy} data-testid="skill-preview-open">
                변경 미리보기
              </button>
            )}
            {row.hasSnapshot && row.localHash && (
              <button type="button" onClick={() => void act(row, () => skills.rollback(row.identity, row.localHash ?? ""), true)} disabled={isBusy} data-testid="skill-rollback">
                롤백
              </button>
            )}
          </span>
        )}
        {open?.key === row.key && open.mode === "link" && <LinkForm row={row} busy={isBusy} onSubmit={(source) => void act(row, () => skills.link(row.identity, source))} onCancel={() => setOpen(null)} />}
        {open?.key === row.key && open.mode === "preview" && (
          <PreviewView
            row={row}
            preview={open.preview}
            busy={isBusy}
            onApply={() => {
              const { targetCommit, targetHash, localHash } = open.preview;
              void act(row, () => skills.apply(row.identity, { commit: targetCommit, targetHash, localHash }), true);
            }}
            onClose={() => setOpen(null)}
          />
        )}
      </li>
    );
  };

  const sections = [
    { title: "Codex", note: "Codex 가 찾은 스킬입니다.", rows: (rows ?? []).filter((row) => row.identity.runtime === "codex") },
    { title: "Claude Code", note: "디스크의 설치 현황입니다. 런타임에서 켜져 있는지는 보지 않습니다.", rows: (rows ?? []).filter((row) => row.identity.runtime === "claude") },
  ];
  return (
    <aside className="work-panel" data-testid="skills-panel">
      <div className="work-header">
        <strong>설치된 스킬</strong>
        <span className="muted">출처와 업데이트</span>
        <span className="work-actions">
          <button type="button" onClick={() => void load(true)} data-testid="skills-refresh">
            새로 고침
          </button>
        </span>
      </div>
      {error && (
        <div className="error work-section" data-testid="skills-error">
          {error}
        </div>
      )}
      {codexError && (
        <p className="muted work-section" data-testid="skills-codex-error">
          Codex 의 스킬 목록을 읽지 못했습니다: {codexError}
        </p>
      )}
      <div className="work-scroll">
        {rows === null && !error && <p className="muted work-section">읽는 중…</p>}
        {rows !== null &&
          sections.map((section) => (
            <section key={section.title} className="work-section" data-testid="skills-section">
              <h3>
                {section.title} ({section.rows.length}) · {section.note}
              </h3>
              <ul>{section.rows.map(renderRow)}</ul>
            </section>
          ))}
      </div>
    </aside>
  );
}
