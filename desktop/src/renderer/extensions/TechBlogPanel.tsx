import { useCallback, useEffect, useState } from "react";
import type { ExtensionCall } from "./repositoryExtensions";
import "./techBlog.css";

type Source = {id: string; path: string; lines: [number, number]};
type Article = {
  id: string; topic: string; style: string; mode: string; commit: string; revision: number;
  sources: Source[]; draft: string; messages: {role: string; text: string}[];
  proposal: {draft: string; reply: string} | null;
  figures: {id: string; alt: string; path: string}[];
  job: {id: string; action: string} | null;
  review: {state: string; findings: {code: string; figure?: string; reason?: string}[];
    checks?: {id: string; decision: string; reason: string}[]} | null;
};
type Summary = Pick<Article, "id" | "topic">;
const states: Record<string,string> = {
  reviewed: "검수 통과", needs_revision: "수정 요청", needs_evidence: "자료 보완",
  awaiting_draft: "원고 대기", awaiting_review: "검수 대기",
};
const findingText: Record<string,string> = {
  figure_required: "주장을 뒷받침할 공개용 PNG를 첨부하세요.", claims_required: "집필 제안의 근거를 먼저 확정하세요.",
  figure_not_in_draft: "첨부 그림을 원고에 삽입하세요.", review_incomplete: "검수 항목이 빠졌습니다.",
};

export default function TechBlogPanel({ call, epoch }: { call: ExtensionCall; epoch: number }) {
  const [articles, setArticles] = useState<Summary[]>([]);
  const [selected, select] = useState("");
  const [topic, setTopic] = useState("");
  const [paths, setPaths] = useState("");
  const [style, setStyle] = useState("PROBLEM_SOLVING");
  const [mode, setMode] = useState("PROJECT_RETROSPECTIVE");
  const [error, setError] = useState("");
  const [creating, setCreating] = useState(false);
  useEffect(() => {
    let active = true;
    call<Summary[]>("list").then(result => { if (active) setArticles(result); },
      e => { if (active) setError(String(e)); });
    return () => { active = false; };
  }, [call, epoch, selected]);
  const create = async () => {
    setCreating(true); setError("");
    try {
      const a = await call<Article>("create", { topic, style, mode, paths: paths.split("\n").map(s=>s.trim()).filter(Boolean) });
      setArticles(items => [a, ...items.filter(x=>x.id !== a.id)]); select(a.id); setTopic(""); setPaths("");
    } catch (e) { setError(String(e)); } finally { setCreating(false); }
  };
  return <section className="blog-studio" aria-label="기술 블로그">
    <label>글 선택<select value={selected} onChange={e => select(e.target.value)}>
      <option value="">새 글</option>{articles.map(a=><option key={a.id} value={a.id}>{a.topic}</option>)}
    </select></label>
    {error && <p role="alert">{error}</p>}
    {selected ? <Editor key={selected} id={selected} call={call} epoch={epoch}/> :
      <form onSubmit={e=>{ e.preventDefault(); void create(); }}>
        <label>글 주제<input required maxLength={300} value={topic} onChange={e=>setTopic(e.target.value)}/></label>
        <label>글의 구성<select value={style} onChange={e=>setStyle(e.target.value)}>
          <option value="PROBLEM_SOLVING">문제 해결기</option><option value="TECHNICAL_DEEP_DIVE">기술 심층 분석</option>
        </select></label>
        <label>조사 방식<select value={mode} onChange={e=>setMode(e.target.value)}>
          <option value="PROJECT_RETROSPECTIVE">프로젝트 회고</option><option value="TECH_EXPLORATION">기술 탐구</option>
        </select></label>
        <label>근거 파일 (선택)<textarea rows={2} value={paths} onChange={e=>setPaths(e.target.value)}
          placeholder="저장소 상대 경로를 한 줄에 하나씩. 비우면 최근 변경 파일에서 선택합니다."/></label>
        <button disabled={!topic.trim() || creating} type="submit">글 만들기</button>
      </form>}
  </section>;
}

function Editor({ id, call, epoch }: {id: string; call: ExtensionCall; epoch: number}) {
  const [article, setArticle] = useState<Article | null>(null);
  const [edit, setEdit] = useState<{text: string; revision: number} | null>(null);
  const [model, setModel] = useState("");
  const [note, setNote] = useState("");
  const [caption, setCaption] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const [error, setError] = useState("");
  const [working, setWorking] = useState(false);
  const receive = useCallback((value: Article) => {
    setArticle(current => !current || current.revision <= value.revision ? value : current);
  }, []);
  useEffect(() => {
    let active = true;
    call<Article>("open", {id}).then(a=>{ if(active) receive(a); },e=>{if(active) setError(String(e));});
    return () => { active = false; };
  }, [id, call, epoch, receive]);
  const act = async (action: string, extra: Record<string,unknown> = {}) => {
    if (!article || working) return;
    setWorking(true); setError("");
    try {
      const a = await call<Article>(action, { id, revision: edit?.revision ?? article.revision, ...extra });
      receive(a); setEdit(null);
      if (action === "write") setNote("");
    } catch (e) { setError(String(e)); } finally { setWorking(false); }
  };
  const attach = async () => {
    if (!file || file.size > 2 * 1024 * 1024) { setError("2 MiB 이하 PNG를 선택하세요."); return; }
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const chunks = [];
      for (let n=0; n<bytes.length; n+=8192) chunks.push(String.fromCharCode(...bytes.subarray(n,n+8192)));
      await act("figure", {png: btoa(chunks.join("")), caption});
    } catch (e) { setError(String(e)); }
  };
  if (!article) return <p role="status">{error || "원고를 읽는 중…"}</p>;
  const busy = working || !!article.job;
  const dirty = edit !== null;
  return <div className="blog-editor">
    <h3>{article.topic}</h3>
    <details><summary>집필 근거 · {article.commit.slice(0,8)}</summary>
      <ul>{article.sources.map(s=><li key={s.id}>{s.id} · {s.path} ({s.lines.join("–")}행)</li>)}</ul>
    </details>
    <label>원고<textarea aria-label="원고" rows={10} disabled={busy} value={edit?.text ?? article.draft}
      onChange={e=>setEdit({text:e.target.value, revision:edit?.revision ?? article.revision})}/></label>
    {dirty && <div className="blog-actions"><button disabled={busy || edit.revision !== article.revision} onClick={()=>void act("save",{draft:edit.text})}>수정 저장</button>
      <button onClick={()=>setEdit(null)}>저장된 원고 보기</button>
      {edit.revision !== article.revision && <span role="status">다른 창에서 원고가 바뀌었습니다.</span>}</div>}
    {article.proposal && <section className="blog-proposal"><h4>AI 집필 제안</h4>
      <pre>{article.proposal.draft}</pre>
      <button disabled={busy || dirty} onClick={()=>void act("accept")}>제안을 원고에 적용</button></section>}
    <details><summary>공개용 그림 ({article.figures.length})</summary>
      <ul>{article.figures.map(f=><li key={f.id}><code>{`![${f.alt}](${f.path})`}</code></li>)}</ul>
      <label>그림 설명<input value={caption} onChange={e=>setCaption(e.target.value)}/></label>
      <label>공개용 PNG<input type="file" accept="image/png" onChange={e=>setFile(e.target.files?.[0] ?? null)}/></label>
      <button disabled={busy || dirty || !file || !caption.trim()} onClick={()=>void attach()}>그림 등록</button>
    </details>
    <label>모델 ID<input placeholder="사용할 Codex 모델" value={model} onChange={e=>setModel(e.target.value)}/></label>
    <label>편집 요청<textarea rows={3} value={note} onChange={e=>setNote(e.target.value)}
      placeholder="강조할 부분이나 제안에서 수정할 점"/></label>
    <div className="blog-actions">
      <button disabled={busy || dirty || !model.trim()} onClick={()=>void act("write",{model:model.trim(),instruction:note})}>
        {article.draft || article.proposal ? "AI에 수정 요청" : "AI에 초안 요청"}</button>
      <button disabled={busy || dirty || !model.trim() || !article.draft} onClick={()=>void act("review",{model:model.trim()})}>독립 검수</button>
    </div>
    {article.job && <p role="status">{article.job.action === "write" ? "AI 집필 중…" : "독립 검수 중…"}</p>}
    {article.review && <section><h4>{states[article.review.state] ?? article.review.state}</h4>
      {article.review.findings.map((f,i)=><p key={i}>{f.reason ?? findingText[f.code] ?? f.code} {f.figure}</p>)}
      {article.review.checks?.map(c=><p key={c.id}><strong>{c.id} · {c.decision}</strong> {c.reason}</p>)}
    </section>}
    {error && <p role="alert">{error}</p>}
    {article.messages.length>0 && <section><h4>편집 대화</h4><ol>{article.messages.map((m,i)=><li key={i}>
      <strong>{m.role === "user" ? "나" : m.role === "assistant" ? "AI" : "시스템"}</strong> {m.text}
    </li>)}</ol></section>}
  </div>;
}
