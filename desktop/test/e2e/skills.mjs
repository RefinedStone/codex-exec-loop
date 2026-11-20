// 화면 시험 (#2383): 빌드한 앱을 가짜 Gateway 에 붙여 "설치된 스킬" 패널을 확인한다. 실제 codex, 턴, 네트워크는 쓰지 않는다.
// 홈 폴더와 앱 데이터는 임시 폴더이다: 사용자의 실제 `~/.claude` 와 `~/.codex` 는 읽지도 쓰지도 않는다.
// GitHub 은 main 의 `net.fetch` 를 바꿔 흉내 내고, 확인 창은 "바꾸기"를 누른 것으로 답한다.
// 흐름: 출처 미확인 목록 → 출처 연결 → 지금 확인 → 변경 미리보기 → 적용 → 롤백. 창은 숨긴 채로 돌린다.
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

const root = realpathSync(mkdtempSync(join(tmpdir(), "akra-skills-e2e-")));
const home = join(root, "home");
const write = (path, text) => {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, text);
};
const SKILL_V1 = "---\nname: caveman\n---\ncaveman v1\n";
const SKILL_V2 = "---\nname: caveman\nallowed-tools: Bash\n---\ncaveman v2\n";
const claudeCaveman = join(home, ".claude", "skills", "caveman");
const codexCaveman = join(home, ".codex", "skills", "caveman");
const pluginHelper = join(home, ".codex", "plugins", "market", "skills", "helper");
write(join(claudeCaveman, "SKILL.md"), SKILL_V1);
write(join(claudeCaveman, "notes.md"), "notes v1\n");
write(join(home, ".claude", "skills", "ultracave", "SKILL.md"), "local ultracave\n");
write(join(codexCaveman, "SKILL.md"), SKILL_V1);
write(join(pluginHelper, "SKILL.md"), "plugin helper\n");
if (process.platform !== "win32") {
  write(join(root, "dotfiles", "linked", "SKILL.md"), "linked\n");
  symlinkSync(join(root, "dotfiles", "linked"), join(home, ".claude", "skills", "linked"));
}

// 가짜 GitHub: 주소 → 답. main 의 `net.fetch` 가 이 표만 본다.
const API = "https://api.github.com/repos/sample/skills";
const RAW = "https://raw.githubusercontent.com/sample/skills";
const blobSha = (text) => createHash("sha1").update(`blob ${Buffer.byteLength(text)}\0`).update(text).digest("hex");
const responses = {};
const addCommit = (commit, files) => {
  const tree = Object.entries(files).map(([path, text]) => ({ path, mode: path.endsWith(".sh") ? "100755" : "100644", type: "blob", sha: blobSha(text), size: Buffer.byteLength(text) }));
  responses[`${API}/git/trees/${commit}?recursive=1`] = JSON.stringify({ tree, truncated: false });
  for (const [path, text] of Object.entries(files)) responses[`${RAW}/${commit}/${path}`] = text;
};
const V1 = "1".repeat(40);
const V2 = "2".repeat(40);
const SIBLINGS = { "README.md": "root readme\n", "skills/ultracave/SKILL.md": "remote ultracave\n", "skills/megacave/SKILL.md": "remote megacave\n" };
addCommit(V1, { ...SIBLINGS, "skills/caveman/SKILL.md": SKILL_V1, "skills/caveman/notes.md": "notes v1\n" });
addCommit(V2, { ...SIBLINGS, "skills/caveman/SKILL.md": SKILL_V2, "skills/caveman/scripts/run.sh": "#!/bin/sh\necho hi\n" });
const HEAD = `${API}/commits/heads/main`;
responses[HEAD] = V1;

const skillsRequests = [];
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  skills: (params) => {
    skillsRequests.push(params);
    const skill = (name, path, pluginId) => ({ name, description: "", path: join(path, "SKILL.md"), scope: "user", enabled: true, pluginId });
    return { data: [{ cwd: home, skills: [skill("caveman", codexCaveman, null), skill("helper", pluginHelper, "helper@market")], errors: [] }] };
  },
});
const env = { ...process.env, HOME: home, USERPROFILE: home, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: join(root, "userData") };
delete env.ELECTRON_RUN_AS_NODE;

const failures = [];
const check = (name, ok) => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}`);
  if (!ok) failures.push(name);
};
const installed = (directory) => Object.fromEntries(readdirSync(directory, { recursive: true, withFileTypes: true }).filter((entry) => entry.isFile()).map((entry) => [join(entry.parentPath, entry.name).slice(directory.length + 1).replaceAll("\\", "/"), readFileSync(join(entry.parentPath, entry.name), "utf8")]));
const same = (left, right) => JSON.stringify(Object.entries(left).sort()) === JSON.stringify(Object.entries(right).sort());

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  await app.evaluate(({ net, dialog }, table) => {
    globalThis.skillFixture = { responses: table, requests: [], confirms: [] };
    net.fetch = async (input) => {
      const url = String(input);
      globalThis.skillFixture.requests.push(url);
      const body = globalThis.skillFixture.responses[url];
      return body === undefined ? new Response("", { status: 404 }) : new Response(body, { status: 200 });
    };
    dialog.showMessageBox = async (...args) => {
      globalThis.skillFixture.confirms.push(args.at(-1).message);
      return { response: 0 };
    };
  }, responses);
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("skills-toggle").click();
  await page.getByTestId("skills-panel").waitFor({ timeout: 10_000 });
  const row = (id) => page.locator(`[data-testid="skill-row"][data-skill="${id}"]`);
  const status = (id) => row(id).getByTestId("skill-status").innerText();
  await row("claude:user:caveman").waitFor({ timeout: 10_000 });

  check("same-name installs of two runtimes are separate rows", (await row("claude:user:caveman").count()) === 1 && (await row("codex:user:caveman").count()) === 1);
  check("copies without a source show 출처 미확인", (await status("claude:user:caveman")) === "출처 미확인" && (await status("codex:user:caveman")) === "출처 미확인" && (await status("claude:user:ultracave")) === "출처 미확인");
  check("a plugin install is read-only and has no buttons", (await status("codex:user:helper")) === "읽기 전용" && (await row("codex:user:helper").getByRole("button").count()) === 0);
  if (process.platform !== "win32") {
    check("a linked skill folder is read-only", (await status("claude:user:linked")) === "읽기 전용" && (await row("claude:user:linked").getByRole("button").count()) === 0);
  }
  check("before a source is linked there is no check, preview, or rollback button", (await row("claude:user:caveman").getByRole("button").allInnerTexts()).join("|") === "출처 연결");
  check("listing does not use the network", (await app.evaluate(() => globalThis.skillFixture.requests.length)) === 0);
  console.log("--- skills panel text (sources unknown) ---");
  console.log(await page.getByTestId("skills-panel").innerText());
  console.log("----------------------------");

  // 출처를 연결한다. 원본의 묶음이 설치 파일과 같으므로 그 커밋을 설치 커밋으로 적는다.
  const caveman = row("claude:user:caveman");
  await caveman.getByTestId("skill-link").click();
  await caveman.getByTestId("skill-link-repository").fill("https://github.com/Sample/Skills");
  await caveman.getByTestId("skill-link-subpath").fill("skills/caveman");
  await caveman.getByTestId("skill-link-policy").selectOption("branch");
  await caveman.getByTestId("skill-link-ref").fill("main");
  await caveman.getByTestId("skill-link-submit").click();
  await caveman.getByTestId("skill-status").filter({ hasText: "최신 (브랜치 main)" }).waitFor({ timeout: 10_000 });
  check("linking a matching source records the installed commit", (await caveman.getByTestId("skill-source").innerText()).includes("sample/skills/skills/caveman · 브랜치 main · 설치 커밋 111111111111 (파일 묶음 일치)"));
  check("the other caveman row stays without a source", (await status("codex:user:caveman")) === "출처 미확인");

  // 원본의 caveman 이 바뀌었다.
  await app.evaluate((_electron, [url, commit]) => {
    globalThis.skillFixture.responses[url] = commit;
  }, [HEAD, V2]);
  await caveman.getByTestId("skill-check").click();
  await caveman.getByTestId("skill-status").filter({ hasText: "업데이트 가능" }).waitFor({ timeout: 10_000 });
  check("a check does not change files", same(installed(claudeCaveman), { "SKILL.md": SKILL_V1, "notes.md": "notes v1\n" }));

  await caveman.getByTestId("skill-preview-open").click();
  await caveman.getByTestId("skill-preview").waitFor({ timeout: 10_000 });
  const previewFiles = (await caveman.getByTestId("skill-preview-file").locator("summary").allInnerTexts()).map((text) => text.replace(/\s+/g, " ").trim());
  check(`the preview lists per-file changes of this skill only (${previewFiles.join(" | ")})`, previewFiles.join("|") === "수정 SKILL.md 동작 변경 가능|삭제 notes.md|추가 scripts/run.sh 동작 변경 가능");
  await caveman.getByTestId("skill-preview-file").first().locator("summary").click();
  await caveman.getByTestId("skill-diff").first().getByText("caveman v2").first().waitFor({ timeout: 15_000 });
  check("the preview shows the diff of a file", true);
  console.log("--- skill row text (preview open) ---");
  console.log(await caveman.innerText());
  console.log("----------------------------");

  await caveman.getByTestId("skill-apply").click();
  await caveman.getByTestId("skill-reload-note").waitFor({ timeout: 10_000 });
  await caveman.getByTestId("skill-status").filter({ hasText: "최신 (브랜치 main)" }).waitFor({ timeout: 10_000 });
  check("apply replaces the install with exactly the approved bundle", same(installed(claudeCaveman), { "SKILL.md": SKILL_V2, "scripts/run.sh": "#!/bin/sh\necho hi\n" }));
  check("main asked the user before it wrote", (await app.evaluate(() => globalThis.skillFixture.confirms)).join("|") === "이 스킬을 승인한 커밋의 파일로 바꿉니까?");
  check("sibling skills did not follow", same(installed(join(home, ".claude", "skills", "ultracave")), { "SKILL.md": "local ultracave\n" }) && readdirSync(join(home, ".claude", "skills")).sort().join("|") === (process.platform === "win32" ? "caveman|ultracave" : "caveman|linked|ultracave"));
  check("the same-name Codex copy is untouched", same(installed(codexCaveman), { "SKILL.md": SKILL_V1 }));
  check("the list is read again with forceReload after the apply", skillsRequests.at(-1)?.forceReload === true);

  await caveman.getByTestId("skill-rollback").click();
  await caveman.getByTestId("skill-status").filter({ hasText: "업데이트 가능" }).waitFor({ timeout: 10_000 });
  check("rollback restores the previous files and the ledger state", same(installed(claudeCaveman), { "SKILL.md": SKILL_V1, "notes.md": "notes v1\n" }) && (await caveman.getByTestId("skill-source").innerText()).includes("설치 커밋 111111111111"));

  const requests = await app.evaluate(() => globalThis.skillFixture.requests);
  check("only public GitHub read addresses of the linked repository were requested", requests.length > 0 && requests.every((url) => url.startsWith(`${API}/`) || url.startsWith(`${RAW}/`)));
  check("no file of another skill was downloaded", requests.every((url) => !url.startsWith(RAW) || url.includes("/skills/caveman/")));
  check("the panel never starts a turn", !gateway.receivedMethods.includes("turn/start") && !gateway.receivedMethods.includes("turn/steer"));
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  // 창을 닫으면 앱이 끝난다. app.close() 는 쓰지 않는다 (P2 기록 참고).
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
  rmSync(root, { recursive: true, force: true });
}
process.exit(failures.length === 0 ? 0 : 1);
