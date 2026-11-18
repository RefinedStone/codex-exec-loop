// These fixed functions run in the inspected page. Callers cannot supply source.
// Keep the source self-contained: the Desktop main tsconfig has no DOM library.
export const NODE_ACTION = `function(operation, args) {
  const node = this;
  if (!node.isConnected) throw Error("Stale ref: node is detached");
  const win = node.ownerDocument.defaultView;
  let current = win;
  const frames = [];
  try {
    while (current !== current.top) {
      const frame = current.frameElement;
      if (!frame) throw Error("Cross-origin frame actions are not supported");
      frames.push(frame);
      current = current.parent;
      void current.document;
    }
  } catch { throw Error("Cross-origin frame actions are not supported"); }
  if (operation === "waitState") {
    const r = node.getBoundingClientRect();
    return { attached: node.isConnected, visible: r.width > 0 && r.height > 0 &&
      win.getComputedStyle(node).visibility !== "hidden" && win.getComputedStyle(node).display !== "none" };
  }
  node.scrollIntoView({block:"center",inline:"center",behavior:"instant"});
  let r = node.getBoundingClientRect();
  const style = win.getComputedStyle(node);
  if (!r.width || !r.height || style.visibility === "hidden" || style.display === "none")
    throw Error("Ref is hidden");
  if (node.matches(":disabled") || node.closest('[aria-disabled="true"]')) throw Error("Ref is disabled");
  let x = Math.max(0, r.left) + (Math.min(win.innerWidth, r.right) - Math.max(0, r.left)) / 2;
  let y = Math.max(0, r.top) + (Math.min(win.innerHeight, r.bottom) - Math.max(0, r.top)) / 2;
  const hit = node.ownerDocument.elementFromPoint(x,y);
  if (operation !== "crop" && hit !== node && !node.contains(hit)) throw Error("Ref is covered by an overlay");
  let left = Math.max(0,r.left), top = Math.max(0,r.top);
  let right = Math.min(win.innerWidth,r.right), bottom = Math.min(win.innerHeight,r.bottom);
  for (const frame of frames) {
    frame.scrollIntoView({block:"center",inline:"center",behavior:"instant"});
    const fr = frame.getBoundingClientRect();
    x += fr.left + frame.clientLeft;
    y += fr.top + frame.clientTop;
    left += fr.left + frame.clientLeft;
    top += fr.top + frame.clientTop;
    right += fr.left + frame.clientLeft;
    bottom += fr.top + frame.clientTop;
    left = Math.max(left,fr.left + frame.clientLeft,0);
    top = Math.max(top,fr.top + frame.clientTop,0);
    right = Math.min(right,fr.left + frame.clientLeft + frame.clientWidth,frame.ownerDocument.defaultView.innerWidth);
    bottom = Math.min(bottom,fr.top + frame.clientTop + frame.clientHeight,frame.ownerDocument.defaultView.innerHeight);
    const frameHit = frame.ownerDocument.elementFromPoint(x,y);
    if (operation !== "crop" && frameHit !== frame && !frame.contains(frameHit))
      throw Error("Frame is covered by an overlay");
  }
  if (operation === "crop") return {x:left,y:top,width:Math.max(0,right-left),height:Math.max(0,bottom-top)};
  if (operation === "type" || operation === "focus") {
    if (node.type === "password" || node.closest('input[type="password"]'))
      throw Error("Password input is not allowed");
    if (operation === "type" && !(node.isContentEditable || node.tagName === "INPUT" || node.tagName === "TEXTAREA"))
      throw Error("Ref is not editable");
    if (node.readOnly) throw Error("Ref is read-only");
    node.focus();
    if (operation === "type" && args.clear) {
      if (node.isContentEditable) {
        const range = node.ownerDocument.createRange();
        range.selectNodeContents(node);
        const selection = win.getSelection(); selection.removeAllRanges(); selection.addRange(range);
      } else node.select();
    }
  }
  if (operation === "select") {
    if (node.tagName !== "SELECT") throw Error("Ref is not a select");
    if (!node.multiple && args.values.length !== 1) throw Error("Select requires one value");
    for (const value of args.values)
      if (![...node.options].some(o => o.value === value && !o.disabled)) throw Error("Select value is unavailable");
    for (const option of node.options) option.selected = args.values.includes(option.value);
    node.dispatchEvent(new win.Event("input",{bubbles:true}));
    node.dispatchEvent(new win.Event("change",{bubbles:true}));
  }
  if (operation === "scroll") node.scrollBy({left:args.deltaX,top:args.deltaY,behavior:"instant"});
  return {x,y};
}`;

export const PASSWORD_FOCUS = `(() => {
  let node = document.activeElement;
  try {
    while (node) {
      if (node.shadowRoot?.activeElement) node = node.shadowRoot.activeElement;
      else if (node.tagName === "IFRAME" || node.tagName === "FRAME") node = node.contentDocument.activeElement;
      else break;
    }
  } catch { throw Error("Cross-origin frame actions are not supported"); }
  if (node && node.type === "password") throw Error("Password input is not allowed");
  return node;
})()`;

export function waitInstaller(id: string, text: string | undefined, backendObject: boolean, state: string, timeoutMs: number): string {
  // The only substitutions are JSON data. No caller string is executable source.
  return `function() {
    const id = ${JSON.stringify(id)}, text = ${JSON.stringify(text ?? null)}, state = ${JSON.stringify(state)};
    const node = ${backendObject ? "this" : "null"};
    const doc = node ? node.ownerDocument : document;
    const win = doc.defaultView;
    const key = "__akraBrowserWaits";
    const waits = globalThis[key] || (globalThis[key] = new Map());
    const check = () => {
      let matches;
      if (node) {
        const r = node.getBoundingClientRect();
        const visible = node.isConnected && r.width > 0 && r.height > 0 &&
          win.getComputedStyle(node).visibility !== "hidden" && win.getComputedStyle(node).display !== "none";
        matches = state === "attached" ? node.isConnected : state === "detached" ? !node.isConnected :
          state === "hidden" ? !visible : visible;
        if (text !== null && node.isConnected) matches = matches && node.textContent.includes(text);
      } else {
        const walker = doc.createTreeWalker(doc.body || doc.documentElement, 4);
        let n, total = 0; matches = false;
        while ((n = walker.nextNode()) && total < 1000000) {
          total += n.textContent.length;
          const parent = n.parentElement;
          const rect = parent.getBoundingClientRect();
          const style = win.getComputedStyle(parent);
          if (!parent.closest('input[type="password"],script,style') && n.textContent.includes(text) &&
              rect.width > 0 && rect.height > 0 && style.visibility !== "hidden" && style.display !== "none") {
            matches = true; break;
          }
        }
        if (state === "hidden" || state === "detached") matches = !matches;
      }
      if (matches) { cleanup(); globalThis.__akraBrowserWait(JSON.stringify({id})); }
    };
    const observer = new MutationObserver(check), resize = new ResizeObserver(check);
    let deadline;
    const cleanup = () => { clearTimeout(deadline); observer.disconnect(); resize.disconnect(); win.removeEventListener("resize",check);
      doc.removeEventListener("load",check,true);
      doc.removeEventListener("transitionend",check,true); doc.removeEventListener("animationend",check,true);
      doc.removeEventListener("input",check,true); waits.delete(id); };
    waits.set(id,cleanup);
    deadline = setTimeout(cleanup,${timeoutMs});
    observer.observe(doc,{subtree:true,childList:true,attributes:true,characterData:true});
    resize.observe(node || doc.documentElement); win.addEventListener("resize",check);
    doc.addEventListener("load",check,true); doc.addEventListener("transitionend",check,true);
    doc.addEventListener("animationend",check,true); doc.addEventListener("input",check,true);
    check(); return true;
  }`;
}
