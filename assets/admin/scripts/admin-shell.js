(function () {
  const akraHashTabRoutes = {
    directions: "/admin/akra/directions",
    tasks: "/admin/akra/tasks"
  };

  const redirectAkraHashTab = function () {
    if (window.location.pathname !== "/admin/akra") return;

    const tabName = window.location.hash.replace(/^#/, "");
    const targetPath = akraHashTabRoutes[tabName];
    if (!targetPath) return;

    window.location.replace(targetPath);
  };

  redirectAkraHashTab();
  window.addEventListener("hashchange", redirectAkraHashTab);
})();

document.addEventListener("input", function (event) {
  const filter = event.target.closest("[data-list-filter]");
  if (!filter) return;

  const targetId = filter.dataset.listFilter;
  const list = document.getElementById(targetId);
  if (!list) return;

  const query = filter.value.trim().toLowerCase();
  const rows = [...list.querySelectorAll("[data-filter-text]")];
  let visibleCount = 0;

  for (const row of rows) {
    const isVisible = row.dataset.filterText.toLowerCase().includes(query);
    row.hidden = !isVisible;
    if (isVisible) visibleCount += 1;
  }

  const emptyState = document.querySelector(`[data-filter-empty="${targetId}"]`);
  if (emptyState) {
    emptyState.classList.toggle("is-visible", visibleCount === 0);
  }
});

document.addEventListener("submit", function (event) {
  const form = event.target;
  const submitter = event.submitter;
  const message = submitter?.dataset.confirm || form?.dataset.confirm;
  if (!message) return;

  if (!window.confirm(message)) {
    event.preventDefault();
    event.stopPropagation();
  }
}, true);
