// main 의 GatewayConnection 과 renderer 세션을 가짜 Gateway 에 실제로 연결해서 시험한다. 턴은 쓰지 않는다.
import { afterEach, describe, expect, it } from "vitest";

import type { AkraBridge, ConnectionState } from "../src/shared/bridge";
import { GatewayConnection } from "../src/main/gatewayConnection";
import { groupThreadsByCwd, startDesktopSession } from "../src/renderer/session/desktopSession";
import type { DesktopState } from "../src/renderer/session/desktopStore";
import type { Thread } from "../src/shared/protocol/generated/v2/Thread";
import { BASIC_RECORDING, type FakeGateway, startFakeGateway, uniqueTestEndpoint } from "./fake-gateway/fakeGateway";

const cleanups: Array<() => Promise<void> | void> = [];

afterEach(async () => {
  while (cleanups.length > 0) {
    await cleanups.pop()?.();
  }
});

async function waitFor(condition: () => boolean, timeoutMs = 5_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!condition()) {
    if (Date.now() > deadline) {
      throw new Error("condition was not met in time");
    }
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

async function gateway(endpoint = uniqueTestEndpoint()): Promise<FakeGateway> {
  const fake = await startFakeGateway(endpoint, BASIC_RECORDING);
  cleanups.push(() => fake.close());
  return fake;
}

/** main 의 접속과 renderer 의 메시지 버스를 프로세스 안에서 잇는다 (preload 대신). */
function connectBridge(endpoint: string, startDaemon?: () => void) {
  const messageListeners = new Set<(text: string) => void>();
  const stateListeners = new Set<(state: ConnectionState) => void>();
  const states: ConnectionState[] = [];
  const connection = new GatewayConnection({
    endpoint,
    onMessage: (text) => messageListeners.forEach((listener) => listener(text)),
    onState: (state) => {
      states.push(state);
      stateListeners.forEach((listener) => listener(state));
    },
    ...(startDaemon ? { startDaemon } : {}),
    reconnectInitialMs: 50,
    reconnectMaxMs: 200,
  });
  const bridge: AkraBridge = {
    send: (text) => void connection.send(text),
    onMessage: (listener) => {
      messageListeners.add(listener);
      return () => messageListeners.delete(listener);
    },
    onConnectionState: (listener) => {
      stateListeners.add(listener);
      return () => stateListeners.delete(listener);
    },
  };
  cleanups.push(() => connection.close());
  return { connection, bridge, states };
}

function sessionStore() {
  const store: Pick<DesktopState, "setConnection" | "setSession" | "setSessionError"> & {
    snapshot: Partial<DesktopState>;
  } = {
    snapshot: {},
    setConnection: (connection) => Object.assign(store.snapshot, { connection }),
    setSession: (session) => Object.assign(store.snapshot, session, { sessionError: null }),
    setSessionError: (sessionError) => Object.assign(store.snapshot, { sessionError }),
  };
  return store;
}

describe("desktop session over the fake gateway", () => {
  it("initializes and shows gateway info and workspace folders", async () => {
    const fake = await gateway();
    const { connection, bridge } = connectBridge(fake.endpoint);
    const store = sessionStore();
    cleanups.push(startDesktopSession(bridge, store, "test"));
    connection.start();

    await waitFor(() => (store.snapshot.folders?.length ?? 0) > 0);
    expect(store.snapshot.gateway?.codexVersion).toBe("codex-cli 0.159.0");
    expect(store.snapshot.versionCheck?.matches).toBe(true);
    expect(store.snapshot.folders?.map((folder) => folder.cwd)).toEqual(["/work/example-app", "/work/other-lib"]);
    expect(fake.receivedMethods.slice(0, 2)).toEqual(["initialize", "initialized"]);
  });

  it("reconnects and initializes again after the gateway restarts", async () => {
    const endpoint = uniqueTestEndpoint();
    const first = await startFakeGateway(endpoint, BASIC_RECORDING);
    const { connection, bridge, states } = connectBridge(endpoint);
    const store = sessionStore();
    cleanups.push(startDesktopSession(bridge, store, "test"));
    connection.start();
    await waitFor(() => first.receivedMethods.includes("thread/list"));

    await first.close();
    await waitFor(() => states.at(-1)?.phase === "disconnected");
    const second = await gateway(endpoint);
    await waitFor(() => second.receivedMethods.includes("thread/list"));
    expect(second.receivedMethods[0]).toBe("initialize");
  });

  it("starts the daemon when nothing is listening and then connects", async () => {
    const endpoint = uniqueTestEndpoint();
    let daemonStarts = 0;
    const { connection, states } = connectBridge(endpoint, () => {
      daemonStarts += 1;
      void gateway(endpoint);
    });
    connection.start();
    await waitFor(() => states.at(-1)?.phase === "connected");
    expect(daemonStarts).toBe(1);
  });
});

describe("groupThreadsByCwd", () => {
  it("groups by cwd, newest first, with readable titles", () => {
    const thread = (id: string, cwd: string, updatedAt: number, name: string | null, preview: string) =>
      ({ id, cwd, updatedAt, name, preview }) as unknown as Thread;
    const folders = groupThreadsByCwd([
      thread("a", "/x", 1, null, ""),
      thread("b", "/y", 3, "Named", "ignored"),
      thread("c", "/x", 2, null, " preview "),
    ]);
    expect(folders.map((folder) => folder.cwd)).toEqual(["/y", "/x"]);
    expect(folders[1]?.threads.map((item) => item.title)).toEqual(["preview", "(제목 없음)"]);
  });
});
