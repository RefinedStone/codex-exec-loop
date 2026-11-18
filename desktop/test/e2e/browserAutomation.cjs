const assert = require("node:assert/strict");
const { once } = require("node:events");
const { createServer } = require("node:http");
const { writeFileSync } = require("node:fs");
const { app, BrowserWindow, nativeImage } = require("electron");
const { BrowserAutomation } = require("../../out/browser-qa/automation.cjs");

const html = `<!doctype html><title>Automation fixture</title>
<form onsubmit="event.preventDefault();document.querySelector('#saved').textContent='Submitted'"><label>Name <input id="name"></label></form><label>Password <input type="password" value="secret"></label>
<label>Choice <select id="choice"><option value="a">Alpha</option><option value="b">Beta</option></select></label>
<button onclick="document.querySelector('#saved').textContent='Saved';console.log('saved')">Save</button>
<div id="saved"></div><button onclick="alert('hello')">Dialog</button>
<fieldset disabled><button>Disabled action</button></fieldset>
<div id="shadow"></div><iframe src="/frame" style="width:200px;height:120px"></iframe><canvas width="120" height="60"></canvas>
<div style="height:1800px">Long page</div>`;
const site = createServer((request, response) => {
  response.writeHead(request.url === "/missing" ? 404 : 200, { "content-type": "text/html" });
  response.end(request.url === "/frame" ? '<label>Frame name <input id="frame-name" style="width:600px"></label>' : html);
});
let window;
let engine;
const deadline = setTimeout(() => { console.error("Browser automation fixture timed out"); app.exit(1); }, 60_000);
async function run() {
  site.listen(0, "127.0.0.1");
  await once(site, "listening");
  while (site.address().port <= 10240) {
    await new Promise((resolve) => site.close(resolve));
    site.listen(0, "127.0.0.1");
    await once(site, "listening");
  }
  await app.whenReady();
  window = new BrowserWindow({ width: 1000, height: 800, show: true, webPreferences: { sandbox: true, contextIsolation: true, nodeIntegration: false, backgroundThrottling: false } });
  await window.loadURL("about:blank");
  if (!window.isFocused()) {
    const focused = once(window, "focus", { signal: AbortSignal.timeout(5000) });
    window.focus();
    await focused;
  }
  const pauses = [];
  engine = new BrowserAutomation(window.webContents, (reason) => pauses.push(reason));
  const execute = async (tool, args = {}, expectError = false) => {
    const result = await engine.execute(tool, args, AbortSignal.timeout(10_000));
    assert.equal(result.isError === true, expectError, `${tool}: ${JSON.stringify(result)}`);
    return result;
  };
  const decoded = (result) => JSON.parse(result.content.find((part) => part.type === "text").text);
  const snapshot = async () => decoded(await execute("browser_snapshot", { diff: false })).text;
  const ref = (text, role, name) => {
    const line = text.split("\n").find((line) => line.includes(`${role} "${name.trim()}"`));
    assert.ok(line, `Missing ${role} ${name} in ${text}`);
    return /^\[(r\d+)\]/.exec(line)[1];
  };
  const url = `http://127.0.0.1:${site.address().port}`;
  await execute("browser_navigate", { url });
  let text = await snapshot();
  assert.ok(!text.includes("secret"));
  const oldName = ref(text, "textbox", "Name ");
  window.blur();
  await window.webContents.executeJavaScript("globalThis.submitted=new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('No submit event')),3000);document.addEventListener('submit',()=>{clearTimeout(timer);resolve(true)},{once:true});});true");
  await execute("browser_type", { ref: oldName, text: "Akra", clear: true, submit: true });
  await window.webContents.executeJavaScript("submitted");
  assert.equal(await window.webContents.executeJavaScript("document.querySelector('#name').value"), "Akra");
  assert.equal(await window.webContents.executeJavaScript("document.querySelector('#saved').textContent"), "Submitted");
  await execute("browser_press", { key: "Tab" });
  text = await snapshot();
  await execute("browser_click", { ref: ref(text, "button", "Disabled action") }, true);
  await window.webContents.executeJavaScript("const root=document.querySelector('#shadow').attachShadow({mode:'open'});root.innerHTML='<input type=\"password\">';root.querySelector('input').focus();");
  await execute("browser_press", { key: "X" }, true);
  assert.equal(await window.webContents.executeJavaScript("document.querySelector('#shadow').shadowRoot.querySelector('input').value"), "");
  await window.webContents.executeJavaScript("document.querySelector('#shadow').shadowRoot.querySelector('input').type='text'");
  await execute("browser_press", { key: "X" });
  assert.equal(await window.webContents.executeJavaScript("document.querySelector('#shadow').shadowRoot.querySelector('input').value"), "X");
  await window.webContents.executeJavaScript("const host=document.createElement('div');document.body.append(host);const closed=host.attachShadow({mode:'closed'});const input=document.createElement('input');input.type='password';closed.append(input);globalThis.closedInput=input;input.focus();");
  await execute("browser_press", { key: "X" }, true);
  assert.equal(await window.webContents.executeJavaScript("closedInput.value"), "");
  await window.webContents.executeJavaScript("const cover=document.createElement('div');cover.id='overlay';cover.style='position:fixed;inset:0;z-index:9999;background:white';document.body.append(cover)");
  await execute("browser_click", { ref: ref(text, "button", "Save") }, true);
  await window.webContents.executeJavaScript("document.querySelector('#overlay').remove()");
  await execute("browser_select", { ref: ref(text, "combobox", "Choice "), values: ["b"] });
  assert.equal(await window.webContents.executeJavaScript("document.querySelector('#choice').value"), "b");
  await execute("browser_click", { ref: ref(text, "button", "Save") });
  assert.equal(await window.webContents.executeJavaScript("document.querySelector('#saved').textContent"), "Saved");
  await execute("browser_wait", { text: "Saved", timeoutMs: 1000 });
  assert.equal(pauses.length, 0, `Synthetic input paused automation: ${pauses}`);
  text = await snapshot();
  if (process.env.AKRA_BROWSER_SNAPSHOT_OUTPUT) {
    const raw = await window.webContents.debugger.sendCommand("Accessibility.getFullAXTree");
    const interactive = decoded(await execute("browser_snapshot", { interactive: true, diff: false }));
    const diff = decoded(await execute("browser_snapshot", { interactive: true }));
    writeFileSync(process.env.AKRA_BROWSER_SNAPSHOT_OUTPUT, JSON.stringify({ rawAx: JSON.stringify(raw), compact: text, interactive: interactive.text, unchangedDiff: diff.text }));
  }
  await execute("browser_type", { ref: ref(text, "textbox", "Password "), text: "blocked" }, true);
  await execute("browser_type", { ref: ref(text, "textbox", "Frame name "), text: "Inside" });
  assert.equal(await window.webContents.executeJavaScript("document.querySelector('iframe').contentDocument.querySelector('input').value"), "Inside");
  const cropped = await execute("browser_screenshot", { ref: ref(text, "textbox", "Frame name ") });
  const cropSize = nativeImage.createFromBuffer(Buffer.from(cropped.content.find(part => part.type === "image").data, "base64")).getSize();
  assert.ok(cropSize.width <= 200, `Frame crop escaped its viewport: ${cropSize.width}`);
  const image = await execute("browser_screenshot");
  assert.ok(image.content.some((part) => part.type === "image" && part.mimeType === "image/jpeg"));
  const jpeg = Buffer.from(image.content.find((part) => part.type === "image").data, "base64");
  const size = nativeImage.createFromBuffer(jpeg).getSize();
  assert.ok(Math.max(size.width, size.height) <= 1280);
  console.log("MEASURE", JSON.stringify({
    snapshotBytes: Buffer.byteLength(text), jpegBytes: jpeg.length,
    pngBytes: (await window.webContents.capturePage()).toPNG().length, ...size,
  }));
  const duplicate = await execute("browser_screenshot");
  assert.ok(!duplicate.content.some((part) => part.type === "image"));
  await execute("browser_scroll", { y: 400 });
  await execute("browser_click", { ref: ref(text, "button", "Dialog") });
  assert.ok(decoded(await execute("browser_console")).entries.length > 0);
  await window.webContents.executeJavaScript(`fetch('${url}/missing').then(r=>r.text())`);
  await execute("browser_wait", { event: "networkidle", timeoutMs: 2000 });
  assert.ok(decoded(await execute("browser_network_failures")).entries.some((entry) => entry.status === 404));
  const originalCommand = window.webContents.debugger.sendCommand.bind(window.webContents.debugger);
  let observerReady;
  const installed = new Promise(resolve => { observerReady = resolve; });
  window.webContents.debugger.sendCommand = async (method, params) => {
    const response = await originalCommand(method, params);
    if (method === "Runtime.evaluate" && params.expression?.includes("__akraBrowserWaits")) observerReady();
    return response;
  };
  const delayed = execute("browser_wait", { text: "Delayed content ready", timeoutMs: 2000 });
  await installed;
  await window.webContents.executeJavaScript("const delayed=document.createElement('div');delayed.textContent='Delayed content ready';document.body.append(delayed)");
  await delayed;
  window.webContents.debugger.sendCommand = originalCommand;
  await execute("browser_navigate", { url: `${url}/second` });
  await execute("browser_click", { ref: oldName }, true);
  await execute("browser_back");
  await execute("browser_forward");
  await execute("browser_reload");
  await window.webContents.executeJavaScript("const list=document.createElement('ul');for(let i=0;i<1500;i++){const li=document.createElement('li');li.textContent='Repeated entry '+i;list.append(li);}document.body.append(list);const button=document.createElement('button');button.textContent='Final action';document.body.append(button)");
  const compact = decoded(await execute("browser_snapshot", { maxChars: 1000, diff: false }));
  assert.ok(compact.text.length <= 1000);
  assert.equal(compact.omittedListItems, 1490);
  assert.ok(ref(compact.text, "button", "Final action"));
  assert.equal(decoded(await execute("browser_snapshot", { maxChars: 1000 })).text, "");
  const paused = new Promise((resolve) => window.webContents.once("before-input-event", resolve));
  window.webContents.sendInputEvent({ type: "keyDown", keyCode: "A" });
  await paused;
  assert.ok(pauses.length > 0);
  console.log("PASS: real Electron navigation, refs, typing, key, select, click, wait, frame, screenshot dedupe, scroll, dialog, console, failures, human pause");
}
run().then(() => app.exit(0), (error) => { console.error(error); app.exit(1); }).finally(() => {
  clearTimeout(deadline);
  engine?.dispose();
  window?.destroy();
  site.close();
});
