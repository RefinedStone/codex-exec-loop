// 접속이 맺어질 때마다 initialize 를 다시 하고 Gateway 정보와 작업 폴더 목록을 읽는다 (P2 §4.5).
// 모델을 호출하지 않는 조회 요청만 쓴다 (D-13).
import type { AkraBridge } from "../../shared/bridge";
import type { InitializeParams } from "../../shared/protocol/generated/InitializeParams";
import type { Thread } from "../../shared/protocol/generated/v2/Thread";
import type { ThreadListResponse } from "../../shared/protocol/generated/v2/ThreadListResponse";
import { GENERATED_CODEX_VERSION } from "../../shared/protocol/version";
import { JsonRpcClient } from "../rpc/jsonRpcClient";
import { checkCodexVersion } from "./codexVersion";
import type { DesktopState, GatewayInfo, WorkspaceFolder } from "./desktopStore";

const THREAD_LIST_LIMIT = 100;
const UNTITLED_THREAD = "(제목 없음)";

type SessionStore = Pick<DesktopState, "setConnection" | "setSession" | "setSessionError">;

export function initializeParams(clientVersion: string): InitializeParams {
  return {
    clientInfo: { name: "akra-desktop", title: "Akra Desktop", version: clientVersion },
    capabilities: { experimentalApi: false, requestAttestation: false },
  };
}

export function groupThreadsByCwd(threads: readonly Thread[]): WorkspaceFolder[] {
  const folders = new Map<string, WorkspaceFolder>();
  for (const thread of threads) {
    const folder = folders.get(thread.cwd) ?? { cwd: thread.cwd, threads: [] };
    folder.threads.push({
      id: thread.id,
      title: thread.name ?? (thread.preview.trim() || UNTITLED_THREAD),
      updatedAt: thread.updatedAt,
    });
    folders.set(thread.cwd, folder);
  }
  const sorted = [...folders.values()];
  for (const folder of sorted) {
    folder.threads.sort((left, right) => right.updatedAt - left.updatedAt);
  }
  const latest = (folder: WorkspaceFolder) => folder.threads[0]?.updatedAt ?? 0;
  return sorted.sort((left, right) => latest(right) - latest(left));
}

export async function initializeSession(
  client: JsonRpcClient,
  store: SessionStore,
  clientVersion: string,
): Promise<void> {
  client.reset("reconnected");
  try {
    await client.request("initialize", initializeParams(clientVersion));
    client.notify("initialized");
    const gateway = await client.request<GatewayInfo>("akra/gateway/info");
    const threads = await client.request<ThreadListResponse>("thread/list", { limit: THREAD_LIST_LIMIT });
    store.setSession({
      gateway,
      versionCheck: checkCodexVersion(GENERATED_CODEX_VERSION, gateway.codexVersion),
      folders: groupThreadsByCwd(threads.data),
    });
  } catch (error) {
    store.setSessionError(error instanceof Error ? error.message : String(error));
  }
}

/** 메시지 버스와 JSON-RPC 클라이언트를 잇는다. 반환한 함수로 연결을 끊는다. */
export function startDesktopSession(
  bridge: AkraBridge,
  store: SessionStore,
  clientVersion: string,
): () => void {
  const client = new JsonRpcClient((text) => bridge.send(text));
  const offMessage = bridge.onMessage((text) => client.handleText(text));
  const offState = bridge.onConnectionState((state) => {
    store.setConnection(state);
    if (state.phase === "connected") {
      void initializeSession(client, store, clientVersion);
    } else {
      client.reset(state.phase);
    }
  });
  return () => {
    offMessage();
    offState();
    client.reset("closed");
  };
}
