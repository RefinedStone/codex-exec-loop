import type { JsonRpcClient } from "../rpc/jsonRpcClient";
import { KERNEL_COMMAND_EFFECT, KERNEL_METHOD, type CommandRequest, type CommandResponse, type KernelCommand } from "./protocol";

/** Each call is a new action. Transport reset rejects it; it is not automatically replayed. */
export function sendKernelCommand(client: JsonRpcClient, command: KernelCommand): Promise<CommandResponse> {
  const request: CommandRequest = KERNEL_COMMAND_EFFECT[command.type] === "read"
    ? { command }
    : { command, idempotency_key: crypto.randomUUID() };
  return client.request<CommandResponse>(KERNEL_METHOD.command, request);
}
