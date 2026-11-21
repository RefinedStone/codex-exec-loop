// Electron utility process entrypoint. main/renderer 모듈에는 이 파일을 import하지 않는다.
import { IssueConfirmationWorker } from "./worker";

const port = process.parentPort;
if (!port) throw new Error("확인 프로세스의 전용 부모 채널이 없습니다.");
const worker = new IssueConfirmationWorker((message) => port.postMessage(message));
port.on("message", ({ data }: { data: unknown }) => {
  if (typeof data === "object" && data !== null && !Array.isArray(data) &&
    Object.keys(data).length === 1 && "type" in data && data.type === "dispose") {
    void worker.dispose().finally(() => process.exit(0));
    return;
  }
  worker.receive(data);
});
process.once("SIGTERM", () => {
  void worker.dispose().finally(() => process.exit(0));
});
port.postMessage({ type: "ready" });
