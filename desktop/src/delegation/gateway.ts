import { GatewayConnection, type GatewayConnectionOptions } from "../main/gatewayConnection";
import type { ChatDispatch, ChatMethod } from "./messages";

const PREFIX = "akra/delegationDriver/";

export function connectDelegatedChat(
  options: Pick<GatewayConnectionOptions, "endpoint" | "startDaemon">,
  dispatch: ChatDispatch,
): GatewayConnection {
  let epoch = 0;
  let closed = false;
  const connection = new GatewayConnection({
    ...options,
    onState: (state) => {
      epoch++;
      if (state.phase === "connected") {
        connection.send(JSON.stringify({ jsonrpc: "2.0", id: `delegated-chat-register-${epoch}`, method: PREFIX + "register", params: {} }));
      }
    },
    onMessage: (text) => {
      if (closed) return;
      const request = decodeRequest(text);
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
      const { method, jobId, instruction } = request.command;
      void dispatch(method, jobId, instruction).then(
        (result) => reply({ result }),
        () => reply({ error: { code: -32000, message: "Chat driver operation failed" } }),
      );
    },
  });
  const close = connection.close.bind(connection);
  connection.close = () => {
    if (closed) return;
    closed = true;
    connection.send(JSON.stringify({ jsonrpc: "2.0", id: `delegated-chat-unregister-${epoch}`, method: PREFIX + "unregister", params: {} }));
    close();
  };
  connection.start();
  return connection;
}

function decodeRequest(text: string): { id: string; command?: {
  method: ChatMethod;
  jobId: string;
  instruction?: string;
} } | undefined {
  if (text.length > 1_600_000) return undefined;
  try {
    const request = JSON.parse(text);
    if (request?.jsonrpc !== "2.0" || typeof request.id !== "string" || typeof request.method !== "string" ||
      !request.method.startsWith(PREFIX)) return undefined;
    const method = request.method.slice(PREFIX.length);
    const params = request.params;
    if (method === "status") return { id: request.id, command: { method, jobId: "" } };
    if ((method === "submit" || method === "observe" || method === "readResult" || method === "stop") &&
      typeof params?.jobId === "string" && params.jobId.trim() && params.jobId.length <= 256 &&
      (method !== "submit" || (typeof params.instruction === "string" && params.instruction.trim() && params.instruction.length <= 250_000))) {
      return { id: request.id, command: { method, jobId: params.jobId, instruction: method === "submit" ? params.instruction : undefined } };
    }
    return { id: request.id };
  } catch {
    return undefined;
  }
}
