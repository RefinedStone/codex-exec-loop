import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
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
createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
