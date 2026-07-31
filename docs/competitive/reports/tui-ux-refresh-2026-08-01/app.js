(() => {
  const tabs = [...document.querySelectorAll("[data-gallery]")];
  const panels = [...document.querySelectorAll(".capture[role='tabpanel']")];

  function selectTab(tab) {
    const targetId = tab.dataset.gallery;
    tabs.forEach((candidate) => {
      const selected = candidate === tab;
      candidate.setAttribute("aria-selected", String(selected));
      candidate.tabIndex = selected ? 0 : -1;
    });
    panels.forEach((panel) => {
      const selected = panel.id === targetId;
      panel.hidden = !selected;
      panel.classList.toggle("is-active", selected);
    });
  }

  tabs.forEach((tab, index) => {
    tab.addEventListener("click", () => selectTab(tab));
    tab.addEventListener("keydown", (event) => {
      if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
      event.preventDefault();
      let nextIndex = index;
      if (event.key === "ArrowLeft") nextIndex = (index - 1 + tabs.length) % tabs.length;
      if (event.key === "ArrowRight") nextIndex = (index + 1) % tabs.length;
      if (event.key === "Home") nextIndex = 0;
      if (event.key === "End") nextIndex = tabs.length - 1;
      tabs[nextIndex].focus();
      selectTab(tabs[nextIndex]);
    });
  });

  const filterButtons = [...document.querySelectorAll("[data-filter]")];
  const backlogRows = [...document.querySelectorAll("[data-priority]")];
  filterButtons.forEach((button) => {
    button.addEventListener("click", () => {
      const filter = button.dataset.filter;
      filterButtons.forEach((candidate) => candidate.classList.toggle("is-active", candidate === button));
      backlogRows.forEach((row) => {
        row.hidden = filter !== "all" && row.dataset.priority !== filter;
      });
    });
  });

  const dialog = document.querySelector(".lightbox");
  const dialogImage = dialog.querySelector("img");
  const dialogCaption = dialog.querySelector("p");
  document.querySelectorAll("[data-zoom]").forEach((button) => {
    button.addEventListener("click", () => {
      const source = button.querySelector("img");
      dialogImage.src = source.currentSrc || source.src;
      dialogImage.alt = source.alt;
      dialogCaption.textContent = source.alt;
      dialog.showModal();
    });
  });
  dialog.querySelector(".lightbox-close").addEventListener("click", () => dialog.close());
  dialog.addEventListener("click", (event) => {
    if (event.target === dialog) dialog.close();
  });

  const navLinks = [...document.querySelectorAll(".rail nav a")];
  const sections = [...document.querySelectorAll("[data-section]")];
  if ("IntersectionObserver" in window) {
    const observer = new IntersectionObserver((entries) => {
      const visible = entries
        .filter((entry) => entry.isIntersecting)
        .sort((a, b) => b.intersectionRatio - a.intersectionRatio)[0];
      if (!visible) return;
      navLinks.forEach((link) => link.classList.toggle("is-active", link.hash === "#" + visible.target.id));
    }, { rootMargin: "-22% 0px -60% 0px", threshold: [0.02, 0.2, 0.45] });
    sections.forEach((section) => observer.observe(section));
  }
})();
