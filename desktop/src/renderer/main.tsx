import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { isDotId, isRemoteHostId } from "../shared/bridge";
import { App, CLIENT_VERSION } from "./App";
import { DotWindow } from "./views/DotWindow";
import "./design/tokens.css";
import "./design/app.css";
import "./styles.css";

const container = document.getElementById("root");
if (!container) {
  throw new Error("root element is missing");
}
// macOS 의 Electron 창은 창 단추가 사이드바 위에 겹치므로 그 자리를 비운다.
if (navigator.userAgent.includes("Mac")) {
  document.documentElement.classList.add("inset-title-bar");
}
// main 이 `?dotWindow=<호스트>&dot=<dot id>` 로 연 창은 그 dot 의 대화만 그린다.
const query = new URLSearchParams(window.location.search);
const dotHost = query.get("dotWindow");
const dotId = query.get("dot");
createRoot(container).render(<StrictMode>{isRemoteHostId(dotHost) && isDotId(dotId) ? <DotWindow host={dotHost} dotId={dotId} clientVersion={CLIENT_VERSION} /> : <App />}</StrictMode>);
