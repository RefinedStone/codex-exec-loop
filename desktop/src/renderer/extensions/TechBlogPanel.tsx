// 자동 기술 블로그 패널이다 (plan 31). 화면은 홈(대상 고르기), 진행, 결과 3개다. 상태와 단계는 커널의 값을 그린다.
import { useCallback, useEffect, useState } from "react";

import { Markdown } from "../conversation/Markdown";
import { onBlogFocus, setViewingBlogArticle, takeBlogFocus } from "./blogFocus";
import { copyText } from "../conversation/copyText";
import { relativeAge } from "../sidebar/threadStatus";
import type { ExtensionCall } from "./repositoryExtensions";
import { type Article, type Finding, STAGES, stateLabel, type State, type Summary, type Targets, taskMarks, taskStages } from "./techBlogStages";
import "./techBlog.css";

const FIGURE_FINDINGS: Record<string, string> = {
  capture_missing: "그림 파일이 없습니다.", capture_invalid: "PNG 를 읽을 수 없습니다.",
  capture_hash_mismatch: "그림이 등록한 뒤에 바뀌었습니다.", figure_not_in_draft: "그림이 원고에 없습니다.",
};
const short = (sha: string) => sha.slice(0, 8);
const errorText = (error: unknown) => String(error).replace(/^Error:\s*/, "");
/** 서버(`targets`)가 받는 커밋 목록 크기의 최대값이다. */
const MAX_TARGETS = 500;
const REQUEST_EXAMPLES = ["더 짧게", "더 자세히", "처음 보는 사람도 읽게"];

function StateBadge({ state, findings = 0 }: { state: State; findings?: number }) {
  const tone = { running: "", reviewed: " badge-ok", findings: " badge-warn", edited: "", stopped: " badge-bad" }[state];
  return <span className={`badge${tone}`}>{stateLabel(state, findings)}</span>;
}

export default function TechBlogPanel({ call, epoch, workspace }: { call: ExtensionCall; epoch: number; workspace: string | null }) {
  const [selected, select] = useState(() => takeBlogFocus() ?? "");
  useEffect(() => onBlogFocus(() => { const id = takeBlogFocus(); if (id) select(id); }), []);
  return <section className="blog-studio" aria-label="기술 블로그">
    {selected ? <ArticleView key={selected} id={selected} call={call} epoch={epoch} workspace={workspace} onBack={() => select("")} />
      : <Home call={call} epoch={epoch} onOpen={select} />}
  </section>;
}

function Home({ call, epoch, onOpen }: { call: ExtensionCall; epoch: number; onOpen(id: string): void }) {
  const [articles, setArticles] = useState<Summary[] | null>(null);
  const [targets, setTargets] = useState<Targets | null>(null);
  const [limit, setLimit] = useState(50);
  const [picked, setPicked] = useState<Set<string> | null>(null);
  const [topic, setTopic] = useState("");
  const [error, setError] = useState("");
  const [starting, setStarting] = useState(false);
  const [checks, recheck] = useState(0);
  useEffect(() => {
    let active = true;
    call<Summary[]>("list").then(value => { if (active) setArticles(value); }, e => { if (active) setError(errorText(e)); });
    return () => { active = false; };
  }, [call, epoch]);
  useEffect(() => {
    let active = true;
    call<Targets>("targets", { limit }).then(value => {
      if (!active) return;
      setTargets(value);
      setPicked(current => current ?? new Set(value.selected));
    }, e => { if (active) setError(errorText(e)); });
    return () => { active = false; };
  }, [call, epoch, limit, checks]);
  const running = articles?.find(a => a.state === "running");
  const chosen = targets && picked ? targets.commits.filter(c => picked.has(c.sha)).map(c => c.sha) : [];
  const blocked = !targets ? "커밋을 읽는 중입니다." : running ? "다른 글을 만드는 중입니다."
    : chosen.length === 0 ? "커밋을 1개 이상 고르세요." : chosen.length > targets.limit ? `커밋은 ${targets.limit}개까지 고를 수 있습니다.` : "";
  const toggle = (sha: string) => setPicked(current => {
    const next = new Set(current);
    if (!next.delete(sha)) next.add(sha);
    return next;
  });
  const generate = async () => {
    setStarting(true); setError("");
    try {
      const article = await call<Article>("generate", { commits: chosen, topic: topic.trim() });
      onOpen(article.id);
    } catch (e) { setError(errorText(e)); } finally { setStarting(false); }
  };
  const now = Date.now();
  return <div className="blog-view">
    {targets && !targets.codex ? <div className="blog-setup" data-testid="blog-setup">
      <h3>새 글을 만들 준비가 안 됐습니다.</h3>
      <ul className="blog-checks">
        <li>✓ Git</li><li>✓ Python 3.9 이상</li>
        <li>✗ Codex CLI <span className="muted">Gateway 를 실행한 컴퓨터에 Codex CLI 를 설치하고 로그인하세요.</span></li>
      </ul>
      <button type="button" className="button" onClick={() => recheck(n => n + 1)}>다시 확인</button>
      <p className="muted">저장된 글은 아래에서 열 수 있습니다.</p>
    </div> : <>
    <h3>새 글</h3>
    {targets && <div className="blog-row">
      <span>대상 작업 · {targets.first ? "최근" : "지난 글 뒤"} 커밋 {targets.first ? targets.commits.length : targets.fresh}개 중 {chosen.length}개 고름</span>
      <span className="bar-spacer" />
      <button type="button" className="ghost" onClick={() => setPicked(new Set(targets.commits.slice(0, targets.limit).map(c => c.sha)))}>모두</button>
      <button type="button" className="ghost" onClick={() => setPicked(new Set())}>해제</button>
    </div>}
    {targets && !targets.first && targets.fresh === 0 && <p className="muted">지난 글 뒤에 새 커밋이 없습니다. 이전 커밋에서 고를 수 있습니다.</p>}
    {targets && <ul className="blog-commits" aria-label="대상 커밋">
      {targets.commits.map(c => <li key={c.sha}><label>
        <input type="checkbox" checked={picked?.has(c.sha) ?? false} onChange={() => toggle(c.sha)} />
        <span className="blog-subject" title={c.subject}>{c.subject}</span>
        <span className="muted">{short(c.sha)} · {relativeAge(c.time, now)}</span>
      </label></li>)}
    </ul>}
    {targets && targets.commits.length >= limit && limit < MAX_TARGETS &&
      <button type="button" className="ghost" onClick={() => setLimit(n => Math.min(n + 100, MAX_TARGETS))}>이전 커밋 더 보기</button>}
    <p className="muted">커밋하지 않은 변경은 글에 넣지 않습니다.</p>
    <label>주제 (선택)<input maxLength={300} value={topic} placeholder="비우면 Akra 가 정합니다" onChange={e => setTopic(e.target.value)} /></label>
    <div className="blog-row">
      <button type="button" className="button primary" disabled={!!blocked || starting} onClick={() => void generate()}>글 만들기</button>
      <span className="muted">{blocked || "고른 커밋의 변경 내용을 모델에 보냅니다 · 모델 호출 2–4회"}</span>
      {running && <button type="button" className="ghost" onClick={() => onOpen(running.id)}>그 글 열기</button>}
    </div>
    </>}
    {error && <p role="alert">{error}</p>}
    <h3>글 목록</h3>
    {articles?.length === 0 && <p className="muted">아직 글이 없습니다. 위에서 작업을 고르고 "글 만들기"를 누르세요.</p>}
    <ul className="blog-articles">
      {articles?.map(a => <li key={a.id}><button type="button" onClick={() => onOpen(a.id)}>
        <span className={a.unread ? "blog-subject blog-unread" : "blog-subject"}>{a.title || "(제목 없음)"}</span>
        {a.state === "running" && a.stage
          ? <span className="muted">{STAGES.find(s => s.id === a.stage)?.label} · {taskStages(a.task).findIndex(s => s.id === a.stage) + 1}/{taskStages(a.task).length}</span>
          : <StateBadge state={a.state} />}
        {a.updated && <span className="muted">{relativeAge(a.updated, now)}</span>}
      </button></li>)}
    </ul>
  </div>;
}

function ArticleView({ id, call, epoch, workspace, onBack }: { id: string; call: ExtensionCall; epoch: number; workspace: string | null; onBack(): void }) {
  const [article, setArticle] = useState<Article | null>(null);
  const [error, setError] = useState("");
  const [working, setWorking] = useState(false);
  const receive = useCallback((value: Article) => {
    setArticle(current => !current || current.revision <= value.revision ? value : current);
  }, []);
  useEffect(() => {
    let active = true;
    call<Article>("open", { id }).then(a => { if (active) receive(a); }, e => { if (active) setError(errorText(e)); });
    return () => { active = false; };
  }, [id, call, epoch, receive]);
  /** 요청이 성공하면 참이다. 실패하면 오류를 보이고, 부르는 쪽은 입력을 지우지 않는다. */
  const act = async (action: string, extra: Record<string, unknown> = {}): Promise<boolean> => {
    if (!article || working) return false;
    setWorking(true); setError("");
    try {
      if (action === "delete") { await call("delete", { id }); onBack(); return true; }
      receive(await call<Article>(action, { id, revision: article.revision, ...extra }));
      return true;
    } catch (e) { setError(errorText(e)); return false; } finally { setWorking(false); }
  };
  useEffect(() => { setViewingBlogArticle(id); return () => setViewingBlogArticle(null); }, [id]);
  const unread = !!article?.unread && article.state !== "running";
  useEffect(() => {
    // 결과를 보였으므로 읽음으로 적는다. 읽지 않은 결과일 때만 부른다: 부를 때마다 변경 알림이 간다.
    if (unread) call("seen", { id }).catch(() => {});
  }, [unread, call, id]);
  const back = <button type="button" className="ghost blog-back" onClick={onBack}>← 글 목록</button>;
  if (!article) return <div className="blog-view">{back}<p role={error ? "alert" : "status"}>{error || "글을 읽는 중…"}</p></div>;
  const errorLine = error && <p role="alert">{error}</p>;
  if (article.state === "running" || article.state === "stopped")
    return <Progress article={article} back={back} working={working} act={act}>{errorLine}</Progress>;
  return <Result article={article} back={back} working={working} act={act} workspace={workspace}>{errorLine}</Result>;
}

type ViewProps = { article: Article; back: React.ReactNode; working: boolean; act(action: string, extra?: Record<string, unknown>): Promise<boolean>; children: React.ReactNode };

function Elapsed({ since }: { since: number }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 1000); return () => window.clearInterval(timer); }, []);
  const seconds = Math.max(0, Math.floor(now / 1000) - since);
  return <span className="muted">경과 {String(Math.floor(seconds / 60)).padStart(2, "0")}:{String(seconds % 60).padStart(2, "0")}</span>;
}

function Progress({ article, back, working, act, children }: ViewProps) {
  const stopped = article.state === "stopped";
  const [showDraft, setShowDraft] = useState(false);
  const steps = taskStages(article.task);
  const marks = taskMarks(article.task, article.stage, article.attempt ?? 0, stopped);
  const noun = article.task === "review" ? "검수" : article.task === "revise" ? "다시 쓰기" : "글 만들기";
  const current = STAGES.find(s => s.id === article.stage);
  const evidence = article.evidence;
  return <div className="blog-view" data-testid="blog-progress">
    {back}
    <div className="blog-row">
      <h3>{stopped ? `${noun}가 멈췄습니다` : `${noun} 중`}</h3>
      <span className="bar-spacer" />
      {article.job && <Elapsed since={article.job.started} />}
    </div>
    <ol className="blog-stages">
      {steps.map((stage, index) => <li key={stage.id} className={marks[index]} aria-current={marks[index] === "active" ? "step" : undefined}>
        <span className="stage-mark" aria-hidden="true">{marks[index] === "done" ? "✓" : marks[index] === "failed" ? "✗" : marks[index] === "active" ? <span className="spinner" /> : index + 1}</span>
        <span>{stage.label}{stage.id === "review" && article.task === "generate" && article.attempt ? " (2회째)" : ""}</span>
        <span className="muted">
          {stage.id === "collect" && evidence && `커밋 ${evidence.commits.length}개 · 파일 ${evidence.files}개${evidence.summarized ? ` · 요약만 넣은 파일 ${evidence.summarized}개` : ""}`}
          {stage.id === "revise" && article.task !== "revise" && marks[index] === "wait" && "지적이 있을 때만, 1회"}
          {marks[index] === "failed" && article.stopped?.message}
        </span>
      </li>)}
    </ol>
    <p role="status" className="blog-live">{stopped ? `${current?.label ?? ""} 단계에서 멈췄습니다.` : `${current?.label ?? ""} 중입니다.`}</p>
    {stopped ? <div className="blog-row">
      <button type="button" className="button primary" disabled={working} onClick={() => void act("resume")}>이어서 하기</button>
      {article.draft && <button type="button" className="button" onClick={() => setShowDraft(v => !v)}>{showDraft ? "초안 닫기" : "초안 보기"}</button>}
      <button type="button" className="button" disabled={working} onClick={() => { if (window.confirm("이 글을 버립니까? 되돌릴 수 없습니다.")) void act("delete"); }}>버리기</button>
    </div> : <>
      <div className="blog-row"><button type="button" className="button" disabled={working} onClick={() => void act("cancel")}>취소</button></div>
      <p className="muted">패널을 닫아도 계속됩니다.</p>
    </>}
    {children}
    {(stopped ? showDraft : !!article.draft) && <section className="blog-preview">
      <p className="muted">{stopped ? "멈추기 전의 초안입니다." : "검수 중인 초안입니다. 바뀔 수 있습니다."}</p>
      <Markdown text={article.draft} />
    </section>}
  </div>;
}

function FindingItem({ finding, article }: { finding: Finding; article: Article }) {
  const claim = finding.id?.startsWith("claim/") ? article.claims?.find(c => `claim/${c.id}` === finding.id) : undefined;
  const subject = claim ? `"${claim.text}"` : finding.id === "draft" ? "글 전체" : `그림 ${finding.figure ?? finding.id?.replace("figure/", "") ?? ""}`;
  const sources = (finding.sources ?? []).map(sid => article.evidence?.commits.find(c => c.id === sid)).filter(Boolean);
  return <li>
    <strong>{subject}</strong>
    <p>{finding.reason ?? FIGURE_FINDINGS[finding.code] ?? finding.code}</p>
    {sources.length > 0 && <p className="muted">근거: {sources.map(s => `${short(s!.sha)} ${s!.subject}`).join(" · ")}</p>}
  </li>;
}

function Result({ article, back, working, act, workspace, children }: ViewProps & { workspace: string | null }) {
  const [raw, setRaw] = useState(false);
  const [editing, setEditing] = useState(false);
  const [asking, setAsking] = useState(false);
  const [request, setRequest] = useState("");
  const findings = article.review?.findings ?? [];
  const used = article.evidence?.commits ?? [];
  const canRewrite = article.version === 2 && !!article.draft.trim();
  const stale = article.exported && article.exported.edition !== article.edition;
  const rewrite = (instruction: string) => void act("revise", { instruction }).then(ok => { if (ok) { setAsking(false); setRequest(""); } });
  if (editing) return <Editor article={article} working={working} act={act} onDone={() => setEditing(false)}>{children}</Editor>;
  return <div className="blog-view" data-testid="blog-result">
    {back}
    <div className="blog-row">
      <h3 className="blog-title">{article.title || "(제목 없음)"}</h3>
      <span className="bar-spacer" />
      <span className="muted">판 {article.edition}</span>
      {article.previous && <button type="button" className="ghost" disabled={working} onClick={() => void act("restore")}>이전 판으로</button>}
    </div>
    <div className="blog-row">
      <StateBadge state={article.state} findings={findings.length} />
      <span className="muted">
        {article.created && new Date(article.created * 1000).toLocaleDateString()}
        {used.length > 0 && ` · 커밋 ${used.length}개`}{article.calls > 0 && ` · 모델 호출 ${article.calls}회`}
      </span>
    </div>
    <div className="blog-row">
      <button type="button" className="button primary" disabled={working || !article.draft} onClick={() => void act("export")}>
        {article.exported ? "다시 내보내기" : "파일로 내보내기"}</button>
      <button type="button" className="button" disabled={!article.draft} onClick={() => copyText(article.draft, "Markdown 을 복사했습니다")}>Markdown 복사</button>
      {canRewrite && <button type="button" className="button" aria-expanded={asking} onClick={() => setAsking(v => !v)}>다시 쓰기…</button>}
      <button type="button" className="button" onClick={() => setEditing(true)}>직접 고치기</button>
      <details className="blog-more">
        <summary className="button" aria-label="더 보기">⋯</summary>
        <div className="blog-menu">
          {canRewrite && article.state === "edited" && <button type="button" disabled={working} onClick={() => void act("review")}>검수하기 · 모델 호출 1회</button>}
          <button type="button" disabled={working} onClick={() => { if (window.confirm("이 글을 지웁니까? 되돌릴 수 없습니다.")) void act("delete"); }}>삭제</button>
        </div>
      </details>
    </div>
    {article.exported && <div className="blog-row" data-testid="blog-exported">
      <span className="muted">{stale ? "내보낸 파일보다 새 판입니다." : <><code>{article.exported.path}</code> 에 썼습니다.</>}</span>
      {!stale && workspace && <button type="button" className="ghost" onClick={() => window.akra.gitOpenFile(workspace, article.exported!.path).catch(() => {})}>편집기로 열기</button>}
    </div>}
    {asking && <div className="blog-request">
      <label>무엇을 바꿀까요?<textarea rows={2} maxLength={1000} value={request} placeholder="예: 도입부에 문제 상황을 먼저 적어 주세요"
        onChange={e => setRequest(e.target.value)}
        onKeyDown={e => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); rewrite(request); }
          if (e.key === "Escape") setAsking(false);
        }} autoFocus /></label>
      <div className="blog-row">{REQUEST_EXAMPLES.map(text => <button key={text} type="button" className="chip-button" onClick={() => setRequest(text)}>{text}</button>)}</div>
      <div className="blog-row">
        <button type="button" className="button primary" disabled={working} onClick={() => rewrite(request)}>다시 쓰기</button>
        <button type="button" className="button" onClick={() => setAsking(false)}>닫기</button>
        <span className="muted">모델 호출 2회 · 지금 원고는 이전 판으로 남습니다</span>
      </div>
    </div>}
    {children}
    {article.stopped && (article.task === "revise" || article.task === "review") && <p role="status" data-testid="blog-task-stopped">
      {article.task === "revise" ? "다시 쓰기" : "검수"}가 멈췄습니다. {article.stopped.message} 글은 그대로입니다.</p>}
    {article.state === "findings" && <div className="blog-row">
      <span>검수가 {findings.length}곳을 지적했습니다. 그대로 내보낼 수 있습니다.</span>
      {canRewrite && <button type="button" className="button" disabled={working} onClick={() => rewrite("")}>지적 반영해 다시 쓰기</button>}
    </div>}
    <div className="segmented" role="group" aria-label="보기">
      <button type="button" className={raw ? undefined : "selected"} aria-pressed={!raw} onClick={() => setRaw(false)}>미리보기</button>
      <button type="button" className={raw ? "selected" : undefined} aria-pressed={raw} onClick={() => setRaw(true)}>원문</button>
    </div>
    <section className="blog-preview">{raw ? <pre>{article.draft}</pre> : <Markdown text={article.draft} />}</section>
    <details open={article.state === "findings"}><summary>검수 결과 · 지적 {findings.length}건</summary>
      {findings.length === 0 ? <p className="muted">{article.review ? "지적이 없습니다." : "검수하지 않았습니다."}</p>
        : <ol className="blog-findings">{findings.map((f, i) => <FindingItem key={i} finding={f} article={article} />)}</ol>}
    </details>
    {used.length > 0 && <details><summary>근거 · 커밋 {used.length}개</summary>
      <ul>{used.map(c => <li key={c.id}><code>{short(c.sha)}</code> {c.subject}</li>)}</ul>
    </details>}
    {article.messages.length > 0 && <details><summary>기록 · {article.messages.length}개</summary>
      <ol>{article.messages.map((m, i) => <li key={i}><strong>{m.role === "user" ? "나" : m.role === "assistant" ? "AI" : "시스템"}</strong> {m.text}</li>)}</ol>
    </details>}
  </div>;
}

/** PNG 를 base64 로 바꾼다. 큰 배열을 한 번에 펼치면 호출 스택을 넘으므로 나눠 바꾼다. */
async function pngBase64(file: File): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const chunks: string[] = [];
  for (let n = 0; n < bytes.length; n += 8192) chunks.push(String.fromCharCode(...bytes.subarray(n, n + 8192)));
  return btoa(chunks.join(""));
}

function Editor({ article, working, act, onDone, children }: Omit<ViewProps, "back"> & { onDone(): void }) {
  const [text, setText] = useState<string | null>(null);
  const [caption, setCaption] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const [notice, setNotice] = useState("");
  const dirty = text !== null && text !== article.draft;
  const leave = () => { if (!dirty || window.confirm("저장하지 않은 글이 있습니다. 버립니까?")) onDone(); };
  const save = () => void act("save", { draft: text ?? article.draft }).then(ok => { if (ok) onDone(); });
  const attach = async () => {
    if (!file || file.size > 2 * 1024 * 1024) { setNotice("2 MiB 이하 PNG 를 고르세요."); return; }
    setNotice("");
    if (await act("figure", { png: await pngBase64(file), caption: caption.trim() })) {
      setCaption(""); setFile(null); setText(null);
    }
  };
  return <div className="blog-view" data-testid="blog-editor">
    <div className="blog-row">
      <button type="button" className="ghost blog-back" onClick={leave}>← 결과로</button>
      <span className="bar-spacer" />
      <span className="muted">판 {article.edition} 을 고치는 중</span>
    </div>
    <label>원고<textarea className="blog-draft" aria-label="원고" value={text ?? article.draft} onChange={e => setText(e.target.value)} /></label>
    <div className="blog-row">
      <button type="button" className="button primary" disabled={working || !dirty} onClick={save}>저장</button>
      <button type="button" className="button" onClick={leave}>취소</button>
      <span className="muted">저장하면 검수 결과가 지워집니다</span>
    </div>
    {children}
    <details><summary>그림 ({article.figures.length})</summary>
      <ul>{article.figures.map(f => <li key={f.id}><code>{`![${f.alt}](${f.path})`}</code></li>)}</ul>
      <label>설명<input value={caption} onChange={e => setCaption(e.target.value)} /></label>
      <label>PNG<input type="file" accept="image/png" onChange={e => setFile(e.target.files?.[0] ?? null)} /></label>
      <button type="button" className="button" disabled={working || dirty || !file || !caption.trim()} onClick={() => void attach()}>그림 등록</button>
      {dirty && <p className="muted">원고를 저장한 뒤 그림을 등록할 수 있습니다. 그림의 참조는 원고 끝에 붙습니다.</p>}
      {notice && <p role="alert">{notice}</p>}
    </details>
  </div>;
}
