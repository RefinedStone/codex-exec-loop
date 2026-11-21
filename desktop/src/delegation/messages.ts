import type { DelegatedChatSession } from "../shared/delegatedChat";

export type ChatMethod = "status" | "submit" | "observe" | "readResult" | "stop";
export interface ChatDispatchOptions {
  session?: DelegatedChatSession;
  expectedAccountId?: string;
  maxResumes?: number;
}
export type ChatReport =
  | { type: "checkpoint"; message: string }
  | { type: "result"; result: string };
export type ChatDispatch = (
  method: ChatMethod, jobId: string, instruction?: string, options?: ChatDispatchOptions,
  report?: (value: ChatReport) => void,
) => Promise<unknown>;

export type ChatWorkerRequest = {
  id: number;
  method: ChatMethod;
  jobId: string;
  instruction?: string;
  options?: ChatDispatchOptions;
};

export type ChatWorkerMessage =
  | { type: "start"; endpoint: string }
  | { type: "result"; id: number; value: unknown }
  | { type: "error"; id: number }
  | { type: "callback"; jobId: string; report: ChatReport }
  | { type: "dispose" };
