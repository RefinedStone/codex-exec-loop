import { connectDelegatedChat } from "./gateway";
import type { ChatWorkerMessage, ChatWorkerRequest } from "./messages";

const port = process.parentPort;
if (!port) throw new Error("Delegation Gateway requires a private parent channel");

let connection: ReturnType<typeof connectDelegatedChat> | undefined;
let sequence = 0;
const pending = new Map<number, { finish(message: ChatWorkerMessage): void }>();

port.on("message", ({ data }: { data: ChatWorkerMessage }) => {
  switch (data.type) {
    case "start": {
      if (connection) return;
      connection = connectDelegatedChat({ endpoint: data.endpoint }, (method, jobId, instruction, options) => {
        const id = ++sequence;
        return new Promise((resolve, reject) => {
          const timer = setTimeout(() => {
            pending.delete(id);
            reject(new Error("Delegation driver timeout"));
          }, 4_000);
          pending.set(id, {
            finish(message) {
              clearTimeout(timer);
              if (message.type === "result") resolve(message.value);
              else reject(new Error("Delegation driver rejected the operation"));
            },
          });
          port.postMessage({ id, method, jobId, instruction, options } satisfies ChatWorkerRequest);
        });
      });
      break;
    }
    case "result":
    case "error": {
      const request = pending.get(data.id);
      pending.delete(data.id);
      request?.finish(data);
      break;
    }
    case "dispose":
      connection?.close();
      // Let the local socket finish writing its unregister before the process exits.
      port.removeAllListeners("message");
      break;
    case "callback":
      connection?.report(data.jobId, data.report);
      break;
  }
});
