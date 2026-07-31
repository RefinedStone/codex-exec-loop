(() => {
  const galleryButtons = [...document.querySelectorAll("[data-gallery]")];
  const galleryPanels = [...document.querySelectorAll(".capture-panel")];

  function selectGallery(id, focus = false) {
    galleryButtons.forEach((button) => {
      const selected = button.dataset.gallery === id;
      button.setAttribute("aria-selected", String(selected));
      if (selected && focus) button.focus();
    });
    galleryPanels.forEach((panel) => {
      panel.hidden = panel.id !== id;
      panel.classList.toggle("is-active", panel.id === id);
    });
  }

  galleryButtons.forEach((button, index) => {
    button.addEventListener("click", () => selectGallery(button.dataset.gallery));
    button.addEventListener("keydown", (event) => {
      if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
      event.preventDefault();
      let nextIndex = index;
      if (event.key === 'ArrowLeft') nextIndex = (index - 1 + galleryButtons.length) % galleryButtons.length;
      if (event.key === 'ArrowRight') nextIndex = (index + 1) % galleryButtons.length;
      if (event.key === 'Home') nextIndex = 0;
      if (event.key === 'End') nextIndex = galleryButtons.length - 1;
      selectGallery(galleryButtons[nextIndex].dataset.gallery, true);
    });
  });

  const filterButtons = [...document.querySelectorAll("[data-filter]")];
  const backlogItems = [...document.querySelectorAll("[data-priority]")];
  filterButtons.forEach((button) => {
    button.addEventListener("click", () => {
      const filter = button.dataset.filter;
      filterButtons.forEach((candidate) => {
        const active = candidate === button;
        candidate.classList.toggle("is-active", active);
        candidate.setAttribute("aria-pressed", String(active));
      });
      backlogItems.forEach((item) => {
        item.hidden = filter !== "all" && item.dataset.priority !== filter;
      });
    });
  });

  const dialog = document.querySelector("#imageDialog");
  const dialogImage = dialog?.querySelector("img");
  const dialogCaption = dialog?.querySelector("p");
  document.querySelectorAll("[data-zoom]").forEach((button) => {
    button.addEventListener("click", () => {
      const source = button.querySelector("img");
      const figure = button.closest("figure");
      if (!source || !dialog || !dialogImage || !dialogCaption) return;
      dialogImage.src = source.src;
      dialogImage.alt = source.alt;
      dialogCaption.textContent = figure?.querySelector("figcaption strong")?.textContent ?? source.alt;
      dialog.showModal();
    });
  });
  dialog?.querySelector(".dialog-close")?.addEventListener("click", () => dialog.close());
  dialog?.addEventListener("click", (event) => {
    if (event.target === dialog) dialog.close();
  });

  const navLinks = [...document.querySelectorAll("[data-nav]")];
  const sections = [...document.querySelectorAll("[data-section]")];
  if ('IntersectionObserver' in window) {
    const observer = new IntersectionObserver((entries) => {
      const visible = entries
        .filter((entry) => entry.isIntersecting)
        .sort((a, b) => b.intersectionRatio - a.intersectionRatio)[0];
      if (!visible) return;
      navLinks.forEach((link) => {
        const active = link.dataset.nav === visible.target.id;
        link.classList.toggle("is-active", active);
        if (active) link.setAttribute("aria-current", "location");
        else link.removeAttribute("aria-current");
      });
    }, { rootMargin: "-18% 0px -68%", threshold: [0, 0.15, 0.35] });
    sections.forEach((section) => observer.observe(section));
  }
})();
