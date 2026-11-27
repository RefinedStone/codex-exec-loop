// Windows only: actual Electron DPAPI using an isolated, disposable profile.
import assert from "node:assert/strict";
import { mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { app, safeStorage, session } from "electron";
import { CREDENTIAL_APP_NAME, keepCredentialIdentity, prepareUserData, USER_DATA_DIRECTORY } from "../../src/main/userDataMigration.ts";

const [mode, root] = process.argv.slice(-2);
assert.equal(process.platform, "win32");
assert.ok(mode === "seed" || mode === "upgrade" || mode === "reopen");
assert.ok(root && existsSync(root));
const legacy = join(root, "@refinedstone", "akra-desktop");
const current = join(root, USER_DATA_DIRECTORY);
app.setPath("appData", root);
const expected = "akra-synthetic-profile-fixture";
const cookieUrl = "https://profile.fixture.invalid/";
let renamed;
if (mode === "seed") {
  mkdirSync(legacy, { recursive: true });
  app.setName(CREDENTIAL_APP_NAME);
  app.setPath("userData", legacy);
} else {
  renamed = keepCredentialIdentity(app);
  const result = prepareUserData(app, undefined);
  assert.equal(result.kind, mode === "upgrade" ? "moved" : "none");
}
const profile = app.getPath("userData");
assert.equal(profile, mode === "seed" ? legacy : current);
app.setPath("sessionData", profile);
const timer = setTimeout(() => app.exit(2), 15_000);
void app.whenReady().then(async () => {
  await renamed;
  assert.ok(safeStorage.isEncryptionAvailable(), "Windows DPAPI must be available");
  if (mode === "seed") {
    const encrypted = safeStorage.encryptString(expected);
    assert.ok(!encrypted.includes(Buffer.from(expected)));
    writeFileSync(join(profile, "jira-token.bin"), encrypted);
    writeFileSync(join(profile, "app-settings.json"), JSON.stringify({ keepAwake: "on" }));
    await session.defaultSession.cookies.set({ url: cookieUrl, name: "fixture", value: "preserved",
      expirationDate: Math.floor(Date.now() / 1000) + 3600, secure: true });
    await session.defaultSession.cookies.flushStore();
  } else {
    assert.equal(app.getName(), USER_DATA_DIRECTORY);
    assert.ok(!existsSync(legacy));
    assert.ok(safeStorage.decryptString(readFileSync(join(profile, "jira-token.bin"))) === expected,
      "legacy ciphertext failed to decrypt after rename");
    assert.equal(JSON.parse(readFileSync(join(profile, "app-settings.json"), "utf8")).keepAwake, "on");
    const cookies = await session.defaultSession.cookies.get({ url: cookieUrl, name: "fixture" });
    assert.ok(cookies.some(cookie => cookie.value === "preserved"), "legacy cookie did not survive");
  }
  console.log("isolated Windows profile " + mode + " passed");
  app.quit();
}).catch(error => {
  console.error(error.message);
  app.exit(1);
});
app.on("quit", () => clearTimeout(timer));
