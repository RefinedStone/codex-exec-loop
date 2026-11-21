import { utilityProcess } from "electron";

import type { ChatDispatch, ChatWorkerMessage, ChatWorkerRequest } from "../delegation/messages";

export function startDelegatedChatProcess(entry: string, endpoint: string, dispatch: ChatDispatch): () => Promise<void> {
  const child = utilityProcess.fork(entry, [], { serviceName: "Akra Delegated Chat Gateway" });
  let closed = false;
  let stop: NodeJS.Timeout | undefined;
  const send = (message: ChatWorkerMessage) => child.postMessage(message);

  child.on("message", ({ id, method, jobId, instruction, options }: ChatWorkerRequest) => {
    if (closed) return;
    void dispatch(method, jobId, instruction, options, (report) => {
      if (!closed) send({ type: "callback", jobId, report });
    }).then(
      (value) => { if (!closed) send({ type: "result", id, value }); },
      () => { if (!closed) send({ type: "error", id }); },
    );
  });
  const exited = new Promise<void>((resolve) => {
    child.once("exit", () => {
      closed = true;
      clearTimeout(stop);
      resolve();
    });
  });
  child.once("spawn", () => {
    if (!closed) send({ type: "start", endpoint });
  });
  return () => {
    if (closed) return exited;
    closed = true;
    send({ type: "dispose" });
    stop = setTimeout(() => child.kill(), 1_000);
    stop.unref();
    return exited;
  };
}
