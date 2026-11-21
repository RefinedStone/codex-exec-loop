import { isAbsolute } from "node:path";

import { GatewayConnection, type GatewayConnectionOptions } from "../main/gatewayConnection";
import { CHAT_CONVERSATION, MAX_CHAT_RESUMES } from "../shared/delegatedChat";
import type { ChatDispatch, ChatDispatchOptions, ChatMethod, ChatReport } from "./messages";

const PREFIX = "akra/delegationDriver/";
type Callback = { jobId: string; token: string; workspace: string };

export function connectDelegatedChat(
  options: Pick<GatewayConnectionOptions, "endpoint" | "startDaemon">,
  dispatch: ChatDispatch,
) {
  let epoch = 0;
  let closed = false;
  let callbackSequence = 0;
  let callbackQueue = Promise.resolve();
  let attachedWorkspace: string | undefined;
  const callbacks = new Map<string, {
    capability: Callback; checkpoint: boolean; result: boolean; delivery?: Promise<void>;
  }>();
  const pending = new Map<string, (value: unknown) => void>();
  const connection = new GatewayConnection({
    ...options,
    onState: (state) => {
      epoch++;
      attachedWorkspace = undefined;
      for (const finish of pending.values()) finish(undefined);
      if (state.phase === "connected") {
        connection.send(JSON.stringify({ jsonrpc: "2.0", id: `delegated-chat-register-${epoch}`, method: PREFIX + "register", params: {} }));
      }
    },
    onMessage: (text) => {
      if (closed || text.length > 1_600_000) return;
      let value;
      try {
        value = JSON.parse(text);
      } catch { return; }
      if (value?.jsonrpc === "2.0" && typeof value.id === "string" && value.method === undefined
        && (Object.hasOwn(value, "result") || Object.hasOwn(value, "error"))) {
        pending.get(value.id)?.(value.result);
        return;
      }
      const request = decodeRequest(value);
      if (!request) return;
      const received = epoch;
      const reply = (body: { result: unknown } | { error: { code: number; message: string } }) => {
        if (!closed && epoch === received) {
          connection.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, ...body }));
        }
      };
      if (!request.command) {
        reply({ error: { code: -32602, message: "Invalid chat driver request" } });
        return;
      }
      const { method, jobId, instruction, options, callback } = request.command;
      if (callback && !callbacks.has(jobId)) {
        if (callbacks.size >= 128) callbacks.delete(callbacks.keys().next().value!);
        callbacks.set(jobId, { capability: callback, checkpoint: false, result: false });
      }
      void dispatch(method, jobId, instruction, options).then(
        async (result) => {
          if (method === "stop") callbacks.delete(jobId);
          // Give the callback the first chance to commit the result. The reverse
          // read response remains the fallback if the callback is unavailable.
          const delivery = method === "readResult" ? callbacks.get(jobId)?.delivery : undefined;
          if (delivery) {
            let timeout: ReturnType<typeof setTimeout> | undefined;
            await Promise.race([delivery, new Promise<void>((resolve) => { timeout = setTimeout(resolve, 1_500); })]);
            clearTimeout(timeout);
          }
          reply({ result });
        },
        () => reply({ error: { code: -32000, message: "Chat driver operation failed" } }),
      );
    },
  });

  function request(method: string, params: object): Promise<unknown> {
    return new Promise((resolve) => {
      const id = `delegated-chat-callback-${++callbackSequence}`;
      const finish = (value: unknown) => {
        pending.delete(id);
        clearTimeout(timer);
        resolve(value);
      };
      const timer = setTimeout(() => finish(undefined), 3_000);
      pending.set(id, finish);
      if (!connection.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }))) finish(undefined);
    });
  }

  function report(jobId: string, value: ChatReport): void {
    const entry = callbacks.get(jobId);
    if (closed || !entry || entry[value.type]) return;
    entry[value.type] = true;
    const { capability } = entry;
    const received = epoch;
    const delivery = callbackQueue.then(async () => {
      if (closed || received !== epoch || callbacks.get(jobId) !== entry) return;
      if (attachedWorkspace !== capability.workspace) {
        const response = await request("akra/kernel/attach", {
          workspace: capability.workspace, client_name: "akra-desktop-delegation-driver",
        });
        if (closed || received !== epoch || !response || typeof response !== "object"
          || !("kernel" in response) || typeof response.kernel !== "string") return;
        attachedWorkspace = capability.workspace;
      }
      if (callbacks.get(jobId) !== entry) return;
      const fields = { id: capability.jobId, token: capability.token };
      const command = value.type === "checkpoint"
        ? { type: "checkpoint_delegation", ...fields, message: value.message }
        : { type: "report_delegation_result", ...fields, result: value.result };
      await request("akra/kernel/command", { command });
    }).catch(() => undefined).finally(() => {
      if (callbacks.get(jobId) !== entry) return;
      if (value.type === "result") callbacks.delete(jobId);
    });
    callbackQueue = delivery;
    if (value.type === "result") entry.delivery = delivery;
  }
  const close = connection.close.bind(connection);
  connection.close = () => {
    if (closed) return;
    closed = true;
    callbacks.clear();
    for (const finish of pending.values()) finish(undefined);
    connection.send(JSON.stringify({ jsonrpc: "2.0", id: `delegated-chat-unregister-${epoch}`, method: PREFIX + "unregister", params: {} }));
    close();
  };
  connection.start();
  return Object.assign(connection, { report });
}

function decodeRequest(request: { jsonrpc?: unknown; id?: unknown; method?: unknown; params?: any } | null): { id: string; command?: {
  method: ChatMethod;
  jobId: string;
  instruction?: string;
  options?: ChatDispatchOptions;
  callback?: Callback;
} } | undefined {
    if (request?.jsonrpc !== "2.0" || typeof request.id !== "string" || typeof request.method !== "string" ||
      !request.method.startsWith(PREFIX)) return undefined;
    const method = request.method.slice(PREFIX.length);
    const params = request.params;
    if (method === "status" || method === "submit") {
      const expectedAccountId = params?.expectedAccountId;
      if (expectedAccountId !== undefined && (typeof expectedAccountId !== "string" ||
        !expectedAccountId.trim() || expectedAccountId.length > 256)) return { id: request.id };
      const options: ChatDispatchOptions = expectedAccountId === undefined ? {} : { expectedAccountId };
      if (method === "status") return {
        id: request.id, command: { method, jobId: "", options: expectedAccountId === undefined ? undefined : options },
      };
      if (typeof params?.jobId !== "string" || !params.jobId.trim() || params.jobId.length > 256 ||
        typeof params.instruction !== "string" || !params.instruction.trim() || params.instruction.length > 250_000) {
        return { id: request.id };
      }
      if (params.session !== undefined) {
        if (params.session?.mode === "new" && params.session.chatUrl === undefined) options.session = { mode: "new" };
        else if (params.session?.mode === "continue" && typeof params.session.chatUrl === "string" &&
          CHAT_CONVERSATION.test(params.session.chatUrl)) options.session = { mode: "continue", chatUrl: params.session.chatUrl };
        else return { id: request.id };
      }
      if (params.maxResumes !== undefined) {
        if (!Number.isInteger(params.maxResumes) || params.maxResumes < 0 || params.maxResumes > MAX_CHAT_RESUMES) {
          return { id: request.id };
        }
        options.maxResumes = params.maxResumes;
      }
      let callback: Callback | undefined;
      if (params.callback !== undefined) {
        const value = params.callback;
        if (typeof value?.jobId !== "string" || !/^[a-zA-Z0-9_-]{1,256}$/.test(value.jobId)
          || typeof value.token !== "string" || !/^[a-f0-9]{64}$/.test(value.token)) return { id: request.id };
        if (value.workspace !== undefined) {
          if (typeof value.workspace !== "string" || value.workspace.length > 4096
            || value.workspace.includes("\0") || !isAbsolute(value.workspace)) return { id: request.id };
          callback = { jobId: value.jobId, token: value.token, workspace: value.workspace };
        }
      }
      return {
        id: request.id,
        command: { method, jobId: params.jobId, instruction: params.instruction, options, callback },
      };
    }
    if ((method === "observe" || method === "readResult" || method === "stop") &&
      typeof params?.jobId === "string" && params.jobId.trim() && params.jobId.length <= 256) {
      return { id: request.id, command: { method, jobId: params.jobId } };
    }
    return { id: request.id };
}
