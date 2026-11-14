// 화면 시험 (plan 25): 빌드한 앱을 원격 호스트가 있는 가짜 Gateway 에 붙인다. 실제 클라우드와 턴은 쓰지 않는다.
// 원격 구역은 기본값이 꺼짐이고, 꺼져 있으면 원격 호스트에 접속하지 않는다. 켜면 그 호스트의 대화를 읽기 전용으로
// 보여 주고, 끄면 구역이 사라진다. 창은 숨긴 채로 돌린다.
import { mkdtempSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  hosts: ["cloud"],
  dots: [{ id: "dot-1", displayName: "Mika", status: "active", kind: "orbit", paused: false, rootThreadId: "01a1abfb-8f0a-7581-a8e4-c2953347f470", messagingRoomId: "room-1" }],
  dotMessages: [
    { id: "m1", createdAt: "2026-11-14T01:00:00Z", author: "Me", fromDot: false, text: "status?" },
    { id: "m2", createdAt: "2026-11-14T01:00:05Z", author: "Mika", fromDot: true, text: "All **green**." },
  ],
});
// 켜고 끄는 설정을 사용자의 실제 앱 데이터 폴더에 쓰지 않는다.
const userData = mkdtempSync(join(tmpdir(), "akra-e2e-remote-"));
// AKRA_HOME 도 임시 폴더로 둔다. 이 컴퓨터의 dots 플래그(`~/.akra/dots/cloud-enabled`)가 시험의 기본값을 바꾸지 않는다.
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData, AKRA_HOME: userData };
delete env.ELECTRON_RUN_AS_NODE;

const failures = [];
const check = (name, ok) => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}`);
  if (!ok) failures.push(name);
};

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  const warnings = [];
  page.on("dialog", (dialog) => {
    warnings.push(dialog.message());
    void dialog.accept();
  });
  const toggleRemoteAgents = async () => {
    await page.getByRole("button", { name: "검색과 명령" }).click();
    await page.getByTestId("palette").getByText("Dots 켜기 / 끄기").click();
  };

  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().waitFor({ timeout: 15_000 });
  check("the remote section is hidden by default", (await page.getByTestId("remote-host").count()) === 0);
  check("no remote connection is opened while it is off", gateway.remoteReceivedMethods.length === 0);

  await toggleRemoteAgents();
  await page.getByTestId("remote-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 15_000 });
  check("turning it on shows a warning first", warnings.length === 1 && warnings[0].includes("약관"));
  // 원격 구역은 로컬 작업 폴더보다 위에 있다. 스크롤하지 않아도 보인다.
  const remoteTop = (await page.getByTestId("remote-host").boundingBox())?.y ?? Infinity;
  const firstLocalTop = (await page.getByTestId("thread-entry").first().boundingBox())?.y ?? -Infinity;
  check("the remote section is above the local threads", remoteTop < firstLocalTop);
  check("the sidebar lists the dots of the remote host", (await page.getByTestId("remote-dot-entry").filter({ hasText: "Mika" }).count()) === 1);
  // Dots 는 클라우드 구역과 다른 구역이고, 그 위에 있다.
  const dotsTop = (await page.getByTestId("dots-section").boundingBox())?.y ?? Infinity;
  const cloudTop = (await page.getByTestId("remote-host").boundingBox())?.y ?? -Infinity;
  check("the dots have their own section above the cloud section", dotsTop < cloudTop && (await page.getByTestId("remote-host").getByTestId("remote-dot-entry").count()) === 0);
  // dot 을 누르면 그 dot 과의 대화(메시지 방)가 옆 칸에 열린다. 보던 화면은 그대로 남는다. 읽기 전용이고 입력창이 없다.
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("composer-input").waitFor({ timeout: 10_000 });
  await page.getByTestId("remote-dot-entry").first().click();
  await page.getByTestId("dot-message").nth(1).waitFor({ timeout: 10_000 });
  check("the dot chat opens beside the open thread", (await page.getByTestId("dot-dock").count()) === 1 && (await page.getByTestId("composer-input").count()) === 1);
  check("the chat has no refresh button while the live channel is up", (await page.getByTestId("dot-refresh").count()) === 0);
  check("the dot chat shows the messages of the room", (await page.getByTestId("dot-message").count()) === 2 && (await page.getByTestId("dot-chat").innerText()).includes("green"));
  check("the dot chat is read over the remote connection", gateway.remoteReceivedMethods.includes("akra/dots/messages") && !gateway.receivedMethods.includes("akra/dots/messages"));
  check("the dot chat has no composer before the user permits sending", (await page.getByTestId("dot-composer-input").count()) === 0);
  check("nothing was sent to the dot before the user permits it", !gateway.remoteReceivedMethods.includes("akra/dots/send"));
  // "세션 한 번 허용": 경고를 확인하면 입력창이 나타난다. 보낸 글은 가짜 Gateway 의 방에 들어간다.
  await page.getByTestId("dot-allow-send").click();
  await page.getByTestId("dot-composer-input").fill("ship it");
  // 다른 대화를 열어도 옆 칸은 남는다. 클라우드 스레드는 읽기 전용이다.
  await page.getByTestId("remote-thread-entry").first().click();
  await page.getByTestId("remote-banner").waitFor({ timeout: 10_000 });
  await page.getByTestId("turn").first().waitFor({ timeout: 15_000 });
  check("the dot chat stays beside another conversation", (await page.getByTestId("dot-dock").count()) === 1);
  check("the remote thread is read over the remote connection", gateway.remoteReceivedMethods.includes("thread/read") && !gateway.remoteReceivedMethods.includes("thread/resume") && !gateway.receivedMethods.includes("thread/read"));
  // 클라우드 스레드에는 입력창도, 보내기를 여는 단추도 없다. 입력창은 옆 칸의 dot 것 하나뿐이다.
  check("the cloud conversation has no composer", (await page.getByTestId("composer-input").count()) === 0);
  check("the cloud conversation cannot be opened for sending", (await page.getByTestId("remote-allow-send").count()) === 0);
  // 옆 칸을 닫았다 열어도 쓰던 글과 보내기 허락이 남는다. dot 줄을 다시 누르면 닫힌다.
  await page.getByTestId("remote-dot-entry").first().click();
  await page.getByTestId("dot-dock").waitFor({ state: "detached", timeout: 10_000 });
  check("closing the dot chat leaves the conversation in view", (await page.getByTestId("remote-banner").count()) === 1);
  await page.getByTestId("remote-dot-entry").first().click();
  await page.getByTestId("dot-composer-input").waitFor({ timeout: 10_000 });
  check("the draft for the dot survives closing the chat", (await page.getByTestId("dot-composer-input").inputValue()) === "ship it");
  await page.getByTestId("dot-message").nth(1).waitFor({ timeout: 10_000 });
  await page.getByTestId("dot-composer-send").click();
  await page.getByTestId("dot-message").nth(2).waitFor({ timeout: 10_000 });
  check("permitting the dot shows a second warning", warnings.length === 2 && warnings[1].includes("실계정"));
  check("the message goes to the dot once over the remote connection", gateway.remoteReceivedMethods.filter((method) => method === "akra/dots/send").length === 1 && !gateway.receivedMethods.includes("akra/dots/send"));
  check("the sent message appears in the chat", (await page.getByTestId("dot-chat").innerText()).includes("ship it"));
  // 보낸 뒤에는 dot 이 답을 만드는 동안 표시가 보이고, "새로 고침"을 누르지 않아도 답이 나타난다. 그 뒤에는 다시
  // 읽지 않는다. 실시간 상태는 Gateway 가 민다.
  await page.getByTestId("dot-typing").waitFor({ timeout: 5_000 });
  check("the chat shows that the dot is working", (await page.getByTestId("dot-status").innerText()).includes("Thinking"));
  await page.getByTestId("dot-message").nth(3).waitFor({ timeout: 15_000 });
  check("the reply of the dot appears without a manual refresh", (await page.getByTestId("dot-chat").innerText()).includes("On it."));
  await page.getByTestId("dot-typing").waitFor({ state: "detached", timeout: 5_000 });
  check("my message that the dot has read shows the read mark", (await page.getByTestId("dot-read").count()) === 1);
  check("the room was watched once over the remote connection", gateway.remoteReceivedMethods.filter((method) => method === "akra/dots/watch").length === 1);
  await page.waitForTimeout(3_000);
  const readsAfterReply = gateway.remoteReceivedMethods.filter((method) => method === "akra/dots/messages").length;
  await page.waitForTimeout(5_000);
  check("the chat stops reading after the reply", gateway.remoteReceivedMethods.filter((method) => method === "akra/dots/messages").length === readsAfterReply);
  // 따로 뜬 창으로 옮긴다. 옆 칸은 닫힌다. 그 창은 dot 의 대화만 보이고 자기 접속으로 읽는다.
  const [dotWindow] = await Promise.all([app.waitForEvent("window"), page.getByTestId("dot-pop-out").click()]);
  await dotWindow.getByTestId("dot-message").nth(1).waitFor({ timeout: 15_000 });
  check("moving the chat to a window closes the side chat", (await page.getByTestId("dot-dock").count()) === 0);
  check("the dot window shows only the chat with the dot", (await dotWindow.getByTestId("dot-window").count()) === 1 && (await dotWindow.getByTestId("thread-entry").count()) === 0 && (await dotWindow.getByTestId("dot-pop-out").count()) === 0);
  // 보내기 허락은 앱을 켠 동안 하나이다. 창을 옮겨도 다시 묻지 않는다.
  await dotWindow.getByTestId("dot-composer-input").waitFor({ timeout: 10_000 });
  check("the dot window keeps the permission to send", warnings.length === 2);
  // 창의 단추로 본 창의 옆 칸으로 되돌린다. 창은 닫힌다.
  await Promise.all([dotWindow.waitForEvent("close"), dotWindow.getByTestId("dot-dock-back").click()]);
  await page.getByTestId("dot-dock").waitFor({ timeout: 10_000 });
  check("the dot window moves the chat back beside the main window", app.windows().length === 1 && (await page.getByTestId("dot-composer-input").count()) === 1);
  check("the cloud conversation was read without a resume", gateway.remoteReceivedMethods.includes("thread/read") && !gateway.remoteReceivedMethods.includes("thread/resume"));

  console.log("--- remote conversation text ---");
  console.log(await page.getByTestId("remote-banner").innerText());
  console.log((await page.getByTestId("remote-host").innerText()).split("\n").slice(0, 4).join(" | "));
  console.log("----------------------------");

  await toggleRemoteAgents();
  await page.getByTestId("remote-host").waitFor({ state: "detached", timeout: 10_000 });
  check("turning it off hides the section and leaves the remote conversation", (await page.getByTestId("remote-banner").count()) === 0);
  check("no turn, thread start, or kernel request went to the remote host", !gateway.remoteReceivedMethods.some((method) => method.startsWith("turn/") || method === "thread/start" || method.startsWith("akra/kernel/")));
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  // 창을 닫으면 앱이 끝난다. app.close() 는 쓰지 않는다 (P2 기록 참고).
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
  rmSync(userData, { recursive: true, force: true, maxRetries: 5 });
}
process.exit(failures.length === 0 ? 0 : 1);
