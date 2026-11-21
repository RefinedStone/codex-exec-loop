export type ChatMethod = "status" | "submit" | "observe" | "readResult" | "stop";
export type ChatDispatch = (method: ChatMethod, jobId: string, instruction?: string) => Promise<unknown>;

export type ChatWorkerRequest = {
  id: number;
  method: ChatMethod;
  jobId: string;
  instruction?: string;
};

export type ChatWorkerMessage =
  | { type: "start"; endpoint: string }
  | { type: "result"; id: number; value: unknown }
  | { type: "error"; id: number }
  | { type: "dispose" };
