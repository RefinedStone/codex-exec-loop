import { useState, type FormEvent } from "react";
import type { GitRef } from "./gitGraph";

export interface GraphFilter {
  readonly branch: { readonly kind: "all" | "local" | "thread" } | { readonly kind: "branch"; readonly name: string };
  readonly author: string; readonly path: string; readonly message: string; readonly hide: boolean;
}
export const ALL_HISTORY: GraphFilter = { branch:{kind:"all"}, author:"", path:"", message:"", hide:false };
export function GraphFilters({ refs, apply }: { refs: readonly GitRef[]; apply(value: GraphFilter): void }) {
  const [value, set] = useState(ALL_HISTORY);
  const scope = value.branch.kind === "branch" ? value.branch.name : value.branch.kind;
  const submit = (event: FormEvent) => { event.preventDefault(); apply(value); };
  return <details className="work-section"><summary>그래프 필터</summary><form onSubmit={submit}>
    <select aria-label="브랜치 필터" value={scope} onChange={(event) => { const name = event.target.value; set({...value, branch: name === "all" || name === "local" || name === "thread" ? {kind:name} : {kind:"branch",name}}); }}>
      <option value="all">전체</option><option value="local">내 브랜치 (로컬)</option><option value="thread">이 스레드의 브랜치</option>
      {refs.filter(r => r.kind === "local_branch" || r.kind === "remote_branch").map(r => {const name = (r.kind === "local_branch" ? "refs/heads/" : "refs/remotes/") + r.name; return <option key={name} value={name}>{r.name}</option>;})}
    </select>
    {([["path", "저장소 경로"],["author","작성자"],["message","메시지"]] as const).map(([key,label]) => <input key={key} aria-label={label} placeholder={label} value={value[key]} maxLength={1024} onChange={e => set({...value,[key]:e.target.value})} />)}
    <label><input type="checkbox" checked={value.hide} onChange={e=>set({...value,hide:e.target.checked})} />일치하지 않는 커밋 숨기기</label>
    <button type="submit">필터 적용</button><button type="button" onClick={()=>{set(ALL_HISTORY);apply(ALL_HISTORY);}}>초기화</button>
  </form></details>;
}
