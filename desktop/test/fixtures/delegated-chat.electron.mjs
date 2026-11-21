import assert from "node:assert/strict";
import { once } from "node:events";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { app, BaseWindow, session } from "electron";

import { DELEGATED_CHAT_PARTITION, DelegatedChatWindow } from "../../src/main/delegatedChatWindow.ts";

const temp = mkdtempSync(join(tmpdir(), "akra-chat-window-"));
app.setPath("userData", temp);
const url = "https://delegated-chat.fixture.invalid/";
const timer = setTimeout(() => app.exit(1), 12_000);
void app.whenReady().then(async () => {
  const partition = session.fromPartition(DELEGATED_CHAT_PARTITION);
  assert.notEqual(partition, session.defaultSession);
  const requested = [];
  // All fixture requests, including any unexpected navigation, terminate in this handler.
  partition.protocol.handle("https", (request) => {
    requested.push(request.url);
    return new Response(`<!doctype html><title>Login fixture</title><button id="login">Login</button>
      <a id="escape" href="https://outside.fixture.invalid/">outside</a>`, {
      headers: { "Content-Type": "text/html" },
    });
  });
  const host = new DelegatedChatWindow(url);
  try {
    const contents = host.getContents();
    const window = BaseWindow.getAllWindows()[0];
    assert.equal(window.isVisible(), false);
    assert.equal(contents.session, partition);
    const loaded = once(contents, "did-finish-load");
    host.openForManualUse();
    await loaded;
    assert.equal(window.isVisible(), true);
    assert.equal(host.isManual, true);
    assert.equal(await contents.executeJavaScript(`document.title`), "Login fixture");
    assert.equal(await contents.executeJavaScript(`typeof window.akra + "," + typeof require`), "undefined,undefined");
    assert.equal(await contents.executeJavaScript(`window.open("https://outside.fixture.invalid/") === null`), true);
    const navigation = once(contents, "will-frame-navigate");
    await contents.executeJavaScript(`document.getElementById("escape").click()`);
    await navigation;
    assert.equal(contents.getURL(), url);
    assert.deepEqual(requested, [url]);
    assert.equal(await contents.executeJavaScript(`Notification.requestPermission()`), "denied");
    window.close();
    assert.equal(window.isVisible(), false);
    assert.equal(host.isManual, false);
    assert.equal(contents.isDestroyed(), false);
    console.log("Delegated chat Electron fixture passed");
  } finally {
    host.close();
  }
}).then(() => app.exit(0), (error) => {
  console.error(error);
  app.exit(1);
});
app.on("quit", () => {
  clearTimeout(timer);
  try { rmSync(temp, { recursive: true, force: true }); } catch {}
});
