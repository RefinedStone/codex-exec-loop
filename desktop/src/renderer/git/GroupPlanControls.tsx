import {useLayoutEffect,useState} from "react";
import type { GitOperation, GroupDestination } from "./gitOperations";
import { groupDestination } from "./gitOperations";
export const GROUP_DROP = "akra:group-base-drop";
export const DRAG_BRANCH = "application/x-akra-current-branch";
export function GroupPlanControls({cwd,branch,base,picks,disabled,plan}:{cwd:string;branch:string;base:string;picks:readonly string[];disabled:boolean;plan(op:GitOperation, branch:string):void}) {
 const [publish,setPublish]=useState<GroupDestination>(); const [note,setNote]=useState("로컬 변경만 계획합니다. 원격 push 단계는 제외합니다.");
 const [checking,setChecking]=useState(false);
 function preview(){if(!disabled && !checking && picks.length<=32) plan({kind:"realign",base,commits:picks,...(publish?{publish}:{})},branch);}
 useLayoutEffect(()=>{const receive=()=>preview();window.addEventListener(GROUP_DROP,receive);return ()=>window.removeEventListener(GROUP_DROP,receive);});
 async function inspect(){setChecking(true);setPublish(undefined);try {const target=groupDestination(JSON.parse(await window.akra.gitOperator(cwd,"group-preview",branch)));if(!target || target.branch!==branch) throw new Error("목적지를 확인할 수 없습니다.");setPublish(target);setNote("추가 확인 후에만 원격에 반영합니다: "+target.url+" "+target.branch+" @ "+target.sha);}catch(e){setNote("원격 단계 제외: "+(e instanceof Error?e.message:String(e)));}finally{setChecking(false);}}
 return <div><button type="button" disabled={disabled||checking||picks.length>32} onClick={preview}>이 커밋을 새 base로 계획</button> <button type="button" disabled={disabled||checking} onClick={()=>void inspect()}>원격 단계 검토</button>{publish&&<button onClick={()=>{setPublish(undefined);setNote("로컬만 변경합니다.");}}>로컬만</button>}<small role="status">{note}</small></div>;
}
