import {execFile} from "node:child_process";
import {isAbsolute} from "node:path";
import {promisify} from "node:util";
import {BrowserWindow, type IpcMainInvokeEvent} from "electron";
import {requireDirectory} from "./gitStatus";
import {resolveGatewayExecutable} from "./gatewayEndpoint";
const exec=promisify(execFile);
export async function gitOperator(event:IpcMainInvokeEvent, root:string, params:{cwd:unknown;action:unknown;branch:unknown}):Promise<string>{
 const owner=BrowserWindow.fromWebContents(event.sender);
 if(!owner || owner.webContents.mainFrame!==event.senderFrame)throw new Error("앱 기본 창에서 요청하십시오.");
 const exe=resolveGatewayExecutable(root);if(!isAbsolute(exe))throw new Error("절대 경로의 Akra Gateway 설치가 필요합니다.");
 const {action,branch}=params; let args:string[];
 switch(action){case "list":case "review":args=["protect",action];break;
 case "add":case "remove":case "force-push":if(typeof branch!=="string"||!branch||branch.length>255||/[\x00-\x1f]/.test(branch))throw new Error("잘못된 브랜치 이름입니다.");args=action==="force-push"?[action,"origin",branch]:["protect",action,branch];break;
 default:throw new Error("알 수 없는 Git 작업입니다.");}
 const frame=event.senderFrame; const cwd=await requireDirectory(params.cwd);
 if(owner.isDestroyed() || event.sender.isDestroyed() || event.sender.mainFrame!==frame)throw new Error("확인 요청 창이 변경되었습니다.");
 const controller=new AbortController();const cancel=()=>controller.abort();
 event.sender.once("destroyed",cancel);event.sender.once("did-start-navigation",cancel);
 try{const {stdout}=await exec(exe,["git-operator",...args],{cwd,windowsHide:true,timeout:180000,maxBuffer:64*1024,signal:controller.signal});return stdout.trim();}
 finally{event.sender.removeListener("destroyed",cancel);event.sender.removeListener("did-start-navigation",cancel);}
}
