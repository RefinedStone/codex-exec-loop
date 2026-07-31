const readTab = document.querySelector("#read-tab");
const diffTab = document.querySelector("#diff-tab");
const readPanel = document.querySelector("#read-panel");
const diffPanel = document.querySelector("#diff-panel");
const readCard = document.querySelector("#read-card");
const readDetail = document.querySelector("#read-detail");
const foldIndicator = document.querySelector("#fold-indicator");
const byteRange = document.querySelector("#byte-range");
const foldCopy = document.querySelector("#fold-copy");

function selectPanel(panel) {
  const showRead = panel === "read";
  readTab.classList.toggle("active", showRead);
  diffTab.classList.toggle("active", !showRead);
  readTab.setAttribute("aria-selected", String(showRead));
  diffTab.setAttribute("aria-selected", String(!showRead));
  readPanel.hidden = !showRead;
  diffPanel.hidden = showRead;
}

function setReadExpanded(expanded) {
  readCard.setAttribute("aria-expanded", String(expanded));
  readDetail.hidden = !expanded;
  byteRange.hidden = !expanded;
  foldCopy.hidden = expanded;
  foldIndicator.textContent = expanded ? "▼" : "›";
}

readTab.addEventListener("click", () => selectPanel("read"));
diffTab.addEventListener("click", () => selectPanel("diff"));
readCard.addEventListener("click", () => {
  setReadExpanded(readCard.getAttribute("aria-expanded") !== "true");
});
