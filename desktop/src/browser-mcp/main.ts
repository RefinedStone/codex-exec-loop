import { BrowserBrokerClient, readSession } from "./broker";
import { BrowserMcpServer } from "./server";

async function main(): Promise<void> {
  const session = await readSession(process.env.AKRA_BROWSER_SESSION_FILE);
  let server: BrowserMcpServer | undefined;
  const broker = new BrowserBrokerClient(session, () => {
    server?.close();
    process.stdin.destroy();
    process.exitCode = 0;
  });
  await broker.start();
  server = new BrowserMcpServer({
    input: process.stdin,
    output: process.stdout,
    call: (threadId, tool, args, signal) => broker.call(threadId, tool, args, signal),
    onClosed: () => broker.close(),
  });
}

void main().catch(() => {
  process.stderr.write("Browser session is unavailable.\n");
  process.stdin.destroy();
  process.exitCode = 1;
});
