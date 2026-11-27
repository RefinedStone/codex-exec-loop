import {useState} from "react";
import type {AkraBridge} from "../../shared/bridge";
export function ProtectedPush({cwd,branch,git,refresh}:{cwd:string;branch:string;git:Pick<AkraBridge,"gitOperator">;refresh():void}){
 const [name,setName]=useState(branch);const [busy,setBusy]=useState(false);const [note,setNote]=useState("");
 const run=async(action:"list"|"review"|"add"|"remove"|"force-push")=>{setBusy(true);try{setNote(await git.gitOperator(cwd,action,name));if(action==="force-push")refresh();}catch(e){setNote(e instanceof Error?e.message:String(e));}finally{setBusy(false);}};
 return <details className="work-section"><summary>보호 브랜치 및 lease push</summary><p>origin의 보호 목록을 먼저 검토하십시오. 목록 변경과 강제 push는 별도 시스템 창에서 확인합니다.</p>
 <label>목적지 브랜치 또는 보호 패턴<input value={name} maxLength={255} onChange={e=>setName(e.target.value)} /></label>
 <div className="publish-buttons">{([["list","목록 보기"],["review","첫 검토"],["add","보호 추가"],["remove","보호 해제"],["force-push","원격 fetch 및 강제 push 검토"]] as const).map(([action,label])=><button type="button" key={action} disabled={busy} onClick={()=>void run(action)}>{label}</button>)}</div>
 {note && <pre role="status">{note}</pre>}</details>;
}
