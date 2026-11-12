// Akra Desktop UX 블루프린트의 시작점이다. `window.akra` 를 쓰지 않으므로 Electron 없이 브라우저에서도 열린다.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { BlueprintApp } from "./BlueprintApp";
import "../design/tokens.css";
import "../design/app.css";

const container = document.getElementById("root");
if (!container) {
  throw new Error("root element is missing");
}
// macOS 의 Electron 창은 창 단추가 사이드바 위에 겹치므로 그 자리를 비운다.
if (navigator.userAgent.includes("Electron") && navigator.userAgent.includes("Mac")) {
  document.documentElement.classList.add("inset-title-bar");
}
createRoot(container).render(
  <StrictMode>
    <BlueprintApp />
  </StrictMode>,
);
